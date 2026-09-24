//! Encrypted preferences and a durable, bounded document evaluation queue.
use crate::{Error, Result, Vault, processable_document, vault::sql_id};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppSettings {
    pub automatic_evaluation: bool,
    /// First-run setup only; never a substitute for live provider readiness.
    pub onboarding_complete: bool,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            automatic_evaluation: true,
            onboarding_complete: false,
        }
    }
}

impl Vault {
    pub fn settings(&self) -> Result<AppSettings> {
        Ok(AppSettings {
            onboarding_complete: self.db.query_row(
                "SELECT completed FROM onboarding WHERE singleton=1",
                [],
                |r| r.get(0),
            )?,
            automatic_evaluation: self.db.query_row(
                "SELECT automatic_evaluation FROM app_settings WHERE singleton=1",
                [],
                |r| r.get(0),
            )?,
        })
    }

    /// Call after an authenticated ChatGPT connection has been verified.
    pub fn complete_onboarding(&mut self) -> Result<()> {
        self.db
            .execute("UPDATE onboarding SET completed=1 WHERE singleton=1", [])?;
        Ok(())
    }

    pub fn set_automatic_evaluation(&mut self, enabled: bool) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE app_settings SET automatic_evaluation=? WHERE singleton=1",
            [enabled],
        )?;
        if enabled {
            tx.execute("UPDATE document_evaluation SET state='queued',error_message=NULL WHERE state='manual' AND source_id IN (SELECT id FROM source WHERE sensitivity!='credential' AND retention='keep')", [])?;
        } else {
            tx.execute(
                "UPDATE document_evaluation SET state='manual' WHERE state='queued'",
                [],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn next_automatic_document(&self) -> Result<Option<u64>> {
        self.next_automatic_document_excluding(&[])
    }

    pub fn next_automatic_document_excluding(&self, active: &[u64]) -> Result<Option<u64>> {
        if !self.settings()?.automatic_evaluation || self.import_pause_reason()?.is_some() {
            return Ok(None);
        }
        let mut stmt=self.db.prepare("SELECT i.local_id,i.extension FROM document_evaluation e JOIN collection_item i ON i.source_id=e.source_id JOIN source s ON s.id=e.source_id WHERE e.state='queued' AND i.kind='document' AND i.deleted_at IS NULL AND s.sensitivity!='credential' AND s.retention='keep' AND NOT EXISTS(SELECT 1 FROM job j WHERE j.source_id=e.source_id AND j.kind='extract_facts' AND j.state IN ('done','needs_review')) ORDER BY i.local_id")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, extension) = row?;
            if processable_document(&extension) && !active.contains(&(id as u64)) {
                return Ok(Some(id as u64));
            }
        }
        Ok(None)
    }

    /// Rechecks the persisted preference at the worker boundary. A queued UI
    /// callback cannot start an automatic job after the preference was disabled.
    pub fn begin_evaluation(&mut self, item: u64, automatic: bool) -> Result<bool> {
        if automatic && !self.settings()?.automatic_evaluation {
            return Ok(false);
        }
        if self.import_pause_reason()?.is_some() {
            return Err(Error::Validation(
                "Automatic imports are paused. Resume the queue after checking the provider connection or usage limit.",
            ));
        }
        let row:Option<(String,String,String)>=self.db.query_row("SELECT e.state,s.sensitivity,i.extension FROM document_evaluation e JOIN collection_item i ON i.source_id=e.source_id JOIN source s ON s.id=e.source_id WHERE i.local_id=? AND i.kind='document' AND i.deleted_at IS NULL AND s.retention='keep'",[sql_id(item)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (state, sensitivity, extension) =
            row.ok_or(Error::Validation("Document not found."))?;
        if sensitivity == "credential" || !processable_document(&extension) {
            return Err(Error::Validation(
                "This document can't be analyzed with AI.",
            ));
        }
        if automatic && state != "queued" {
            return Ok(false);
        }
        if state == "running"
            || (automatic && state == "done" && self.evaluation_warning(item)?.is_none())
        {
            return Err(Error::Validation(
                "This document is being analyzed or is already complete.",
            ));
        }
        if !automatic && state == "done" && self.evaluation_warning(item)?.is_none() {
            self.db.execute("DELETE FROM import_step_cache WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=?)", [sql_id(item)?])?;
            self.db.execute("DELETE FROM import_step_progress WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=?)", [sql_id(item)?])?;
            self.db.execute("DELETE FROM extraction_checkpoint WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=?)", [sql_id(item)?])?;
        }
        self.db.execute("UPDATE document_evaluation SET state='running',error_message=NULL WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=?)",[sql_id(item)?])?;
        Ok(true)
    }

    pub fn finish_evaluation(&mut self, item: u64, error: Option<&str>) -> Result<()> {
        self.finish_evaluation_with_warning(item, error, None)
    }

    pub fn finish_evaluation_with_warning(
        &mut self,
        item: u64,
        error: Option<&str>,
        warning: Option<&str>,
    ) -> Result<()> {
        let message = error.map(|e| e.chars().take(1000).collect::<String>());
        let pending = self.review_questions(item)?.len();
        let warning = crate::question_warning(pending)
            .or_else(|| warning.map(|e| e.chars().take(1000).collect::<String>()));
        self.db.execute("UPDATE document_evaluation SET state=?,error_message=?,warning_message=? WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=?) AND state='running'",params![if error.is_some(){"failed"}else{"done"},message,warning,sql_id(item)?])?;
        Ok(())
    }

    pub fn evaluation_warning(&self, item: u64) -> Result<Option<String>> {
        Ok(self.db.query_row("SELECT e.warning_message FROM document_evaluation e JOIN collection_item i ON i.source_id=e.source_id WHERE i.local_id=?",[sql_id(item)?],|r|r.get(0)).optional()?.flatten())
    }

    pub fn evaluation_error(&self, item: u64) -> Result<Option<String>> {
        Ok(self.db.query_row("SELECT e.error_message FROM document_evaluation e JOIN collection_item i ON i.source_id=e.source_id WHERE i.local_id=?",[sql_id(item)?],|r|r.get(0)).optional()?.flatten())
    }
}

#[cfg(test)]
mod onboarding_tests {
    use crate::Vault;

    #[test]
    fn interrupted_onboarding_resumes_and_completion_survives_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let mut vault = Vault::create(&root, "synthetic-onboarding-password").unwrap();
        vault.save_note(None, "Synthetic", "Preserved").unwrap();
        vault.set_automatic_evaluation(false).unwrap();
        assert!(!vault.settings().unwrap().onboarding_complete);
        drop(vault);
        let mut vault = Vault::unlock(&root, "synthetic-onboarding-password").unwrap();
        assert!(!vault.settings().unwrap().onboarding_complete);
        vault.complete_onboarding().unwrap();
        vault.complete_onboarding().unwrap();
        drop(vault);
        let vault = Vault::unlock(&root, "synthetic-onboarding-password").unwrap();
        let settings = vault.settings().unwrap();
        assert!(settings.onboarding_complete);
        assert!(!settings.automatic_evaluation);
        assert_eq!(vault.collection("Preserved", false).unwrap().total, 1);
    }

    #[test]
    fn version_eleven_migration_preserves_data_and_requires_initial_provider_setup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let mut vault = Vault::create(&root, "synthetic-migration-password").unwrap();
        vault.save_note(None, "Synthetic", "Preserved").unwrap();
        vault
            .db
            .execute_batch("DROP TABLE onboarding; PRAGMA user_version=11;")
            .unwrap();
        drop(vault);
        let vault = Vault::unlock(&root, "synthetic-migration-password").unwrap();
        assert!(!vault.settings().unwrap().onboarding_complete);
        assert_eq!(vault.collection("Preserved", false).unwrap().total, 1);
        assert_eq!(
            vault
                .db
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            13
        );
    }
}
