//! Development-only reset. Call only after disconnecting all old worker sessions.
use super::*;

impl Vault {
    /// Clear local content while retaining the vault identity, keys and settings.
    /// Database deletion is atomic. Object cleanup follows commit, so a failed
    /// cleanup never leaves live records pointing at missing files; retry it.
    pub fn wipe_data_for_development(&mut self) -> Result<()> {
        let objects = self.root.join("objects");
        if !fs::symlink_metadata(&objects)?.file_type().is_dir() {
            return Err(Error::Format);
        }
        let files = fs::read_dir(&objects)?
            .map(|entry| {
                let entry = entry?;
                regular(&entry.path())?;
                Ok(entry.path())
            })
            .collect::<Result<Vec<_>>>()?;
        let tx = self.db.transaction()?;
        tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
        // Enumerating ordinary tables also clears future content/cache tables.
        // FTS shadow tables must only be changed through their owning FTS index.
        let tables = tx
            .prepare("SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('vault_meta','app_settings','onboarding','entity','property_definition')")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for table in tables {
            tx.execute(
                &format!("DELETE FROM \"{}\"", table.replace('"', "\"\"")),
                [],
            )?;
        }
        tx.execute_batch(
            "DELETE FROM entity WHERE id NOT IN (SELECT profile_id FROM vault_meta);
             DELETE FROM entity_alias;
             UPDATE entity SET label='Ich', deleted_at=NULL WHERE id IN (SELECT profile_id FROM vault_meta);
             DELETE FROM property_definition WHERE key NOT IN ('person.tax_id','person.social_insurance_number','person.birth_date');
             INSERT INTO import_control VALUES(1,NULL);
             INSERT INTO segment_fts(segment_fts) VALUES('rebuild');",
        )?;
        tx.commit()?;
        for file in files {
            fs::remove_file(file).map_err(|_| Error::Validation(
                "Database cleared, but some stored files could not be removed. Check permissions and choose Wipe data again.",
            ))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PASSWORD: &str = "synthetic-wipe-password";

    #[test]
    fn wipe_clears_content_and_indexes_and_allows_repeated_imports() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let file = temp.path().join("original.txt");
        fs::write(&file, "synthetic unique-search-token").unwrap();
        let mut vault = Vault::create(&root, PASSWORD).unwrap();
        vault.set_automatic_evaluation(false).unwrap();
        vault.complete_onboarding().unwrap();
        let header = fs::read(root.join("header.json")).unwrap();
        for _ in 0..2 {
            vault
                .save_note(None, "Synthetic private label", "Synthetic private value")
                .unwrap();
            let item = vault
                .import_document(&file, "original.txt", DocumentClass::Personal)
                .unwrap();
            vault.enable_text_search(item).unwrap();
            vault.begin_evaluation(item, false).unwrap();
            vault.begin_import_progress(item, "synthetic-run").unwrap();
            vault.db.execute("INSERT INTO import_step_cache VALUES((SELECT source_id FROM collection_item WHERE local_id=?),'pipeline','batch','step','{}')", [sql_id(item).unwrap()]).unwrap();
            vault
                .db
                .execute(
                    "INSERT INTO credential_import VALUES('archive','fingerprint',x'0102','now')",
                    [],
                )
                .unwrap();
            vault
                .db
                .execute(
                    "UPDATE import_control SET pause_reason='synthetic pause'",
                    [],
                )
                .unwrap();
            let graph = vault.knowledge_map().unwrap();
            assert!(!graph.nodes.is_empty());
            vault
                .save_knowledge_viewport(&crate::KnowledgeViewport {
                    selected_node: Some(graph.nodes[0].id.clone()),
                    ..Default::default()
                })
                .unwrap();
            // A new ordinary table is covered without changing the wipe logic.
            vault.db.execute_batch("CREATE TABLE IF NOT EXISTS future_cache (value TEXT); INSERT INTO future_cache VALUES('private');").unwrap();
            assert!(fs::read_dir(root.join("objects")).unwrap().next().is_some());
            vault.wipe_data_for_development().unwrap();
            vault.verify().unwrap();
            assert!(vault.collection("", false).unwrap().items.is_empty());
            assert!(vault.import_jobs().unwrap().is_empty());
            assert!(vault.import_pause_reason().unwrap().is_none());
            assert!(!vault.settings().unwrap().automatic_evaluation);
            assert!(vault.settings().unwrap().onboarding_complete);
            assert!(fs::read_dir(root.join("objects")).unwrap().next().is_none());
            assert_eq!(fs::read(&file).unwrap(), b"synthetic unique-search-token");
            assert_eq!(fs::read(root.join("header.json")).unwrap(), header);
            let tables = vault.db.prepare("SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%'").unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<std::result::Result<Vec<_>, _>>().unwrap();
            for table in tables {
                let expected = match table.as_str() {
                    "vault_meta" | "app_settings" | "onboarding" | "entity" | "import_control" => 1,
                    "property_definition" => 3,
                    _ => 0,
                };
                let count: i64 = vault
                    .db
                    .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                assert_eq!(count, expected, "{table}");
            }
            assert_eq!(
                vault
                    .db
                    .query_row(
                        "SELECT count(*) FROM segment_fts WHERE segment_fts MATCH 'synthetic'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            drop(vault);
            vault = Vault::unlock(&root, PASSWORD).unwrap();
        }
        vault.wipe_data_for_development().unwrap(); // Empty resets are safe too.
        vault
            .save_note(None, "After reset", "Still writable")
            .unwrap();
        vault.verify().unwrap();
    }

    #[test]
    fn failed_transaction_keeps_records_and_objects() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let file = temp.path().join("original.txt");
        fs::write(&file, "synthetic data").unwrap();
        let mut vault = Vault::create(&root, PASSWORD).unwrap();
        vault
            .import_document(&file, "original.txt", DocumentClass::Personal)
            .unwrap();
        vault.db.execute_batch("CREATE TRIGGER prevent_wipe BEFORE DELETE ON source BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
        assert!(vault.wipe_data_for_development().is_err());
        assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
        vault.verify().unwrap();
        assert!(fs::read_dir(root.join("objects")).unwrap().next().is_some());
    }

    #[test]
    fn rejects_redirected_object_directory_without_touching_external_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let mut vault = Vault::create(&root, PASSWORD).unwrap();
        vault.save_note(None, "Keep", "Keep this note").unwrap();
        fs::remove_dir(root.join("objects")).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "keep").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("objects")).unwrap();
        assert!(vault.wipe_data_for_development().is_err());
        assert_eq!(vault.collection("", false).unwrap().items.len(), 1);
        assert!(outside.join("keep").exists());
    }
}
