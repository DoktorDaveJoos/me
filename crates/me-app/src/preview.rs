//! Session-only data for the UI preview, not a persistent vault schema.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Note(String),
    Document(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u64,
    pub title: String,
    pub content: Content,
    pub pinned: bool,
}

#[derive(Default)]
pub struct Collection {
    items: Vec<Item>,
    next_id: u64,
}

impl Collection {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }
    pub fn has_pins(&self) -> bool {
        self.items.iter().any(|item| item.pinned)
    }

    pub fn save_note(
        &mut self,
        id: Option<u64>,
        title: &str,
        value: &str,
    ) -> Result<u64, &'static str> {
        let title = title.trim();
        let value = value.trim();
        if title.is_empty() {
            return Err("Gib deiner Angabe eine Bezeichnung.");
        }
        if value.is_empty() {
            return Err("Ergänze einen Wert oder eine kurze Notiz.");
        }
        if let Some(id) = id {
            let item = self
                .items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or("Diese Angabe ist nicht mehr vorhanden.")?;
            if !matches!(item.content, Content::Note(_)) {
                return Err("Dokumente lassen sich hier nicht bearbeiten.");
            }
            item.title = title.to_owned();
            item.content = Content::Note(value.to_owned());
            Ok(id)
        } else {
            Ok(self.push(title.to_owned(), Content::Note(value.to_owned())))
        }
    }

    pub fn stage_document(&mut self, path: &Path) -> bool {
        // Paths only: don't inspect or copy files on the UI thread.
        if self
            .items
            .iter()
            .any(|item| matches!(&item.content, Content::Document(existing) if existing == path))
        {
            return false;
        }
        let Some(name) = path.file_name() else {
            return false;
        };
        self.push(
            name.to_string_lossy().into_owned(),
            Content::Document(path.to_owned()),
        );
        true
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.pinned = !item.pinned;
        }
    }

    pub fn visible(&self, query: &str, pinned_only: bool) -> Vec<&Item> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut result: Vec<_> = self
            .items
            .iter()
            .filter(|item| {
                if pinned_only && !item.pinned {
                    return false;
                }
                // Search the displayed filename, never its private directory names.
                let value = match &item.content {
                    Content::Note(value) => value.as_str(),
                    Content::Document(_) => "",
                };
                let searchable = format!("{} {value}", item.title).to_lowercase();
                terms.iter().all(|term| searchable.contains(term))
            })
            .collect();
        result.sort_by_key(|item| (std::cmp::Reverse(item.pinned), std::cmp::Reverse(item.id)));
        result
    }

    fn push(&mut self, title: String, content: Content) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Item {
            id,
            title,
            content,
            pinned: false,
        });
        id
    }
}

pub fn supported_document(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            [
                "pdf", "png", "jpg", "jpeg", "heic", "webp", "tif", "tiff", "bmp", "gif", "txt",
                "md", "rtf", "doc", "docx", "odt", "xls", "xlsx", "ods", "csv", "ppt", "pptx",
                "odp",
            ]
            .contains(&ext.to_ascii_lowercase().as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_notes_can_be_edited_without_changing_their_identity_or_pin() {
        let mut collection = Collection::default();
        let id = collection
            .save_note(None, "  Schuhgröße  ", " 42 ")
            .unwrap();
        collection.toggle_pin(id);
        assert_eq!(collection.save_note(Some(id), "Schuhgröße", "43"), Ok(id));
        let item = collection.get(id).unwrap();
        assert!(item.pinned);
        assert_eq!(item.content, Content::Note("43".into()));
        assert_eq!(collection.len(), 1);
        assert!(collection.save_note(Some(id), "", "44").is_err());
        assert_eq!(
            collection.get(id).unwrap().content,
            Content::Note("43".into())
        );
    }

    #[test]
    fn searching_combines_title_and_value_but_does_not_search_document_directories() {
        let mut collection = Collection::default();
        collection
            .save_note(None, "Zertifikat", "Sprachkurs München")
            .unwrap();
        collection.stage_document(Path::new("/private/Finanzen/Rechnung Laptop.pdf"));
        assert_eq!(collection.visible("ZERTIFIKAT münchen", false).len(), 1);
        assert_eq!(collection.visible("  laptop rechnung  ", false).len(), 1);
        assert!(collection.visible("Finanzen", false).is_empty());
        assert!(collection.visible("Sprachkurs Laptop", false).is_empty());
    }

    #[test]
    fn duplicate_paths_are_not_added_twice_and_pins_precede_recent_items() {
        let mut collection = Collection::default();
        let first = collection.save_note(None, "Notiz", "Für später").unwrap();
        assert!(collection.stage_document(Path::new("/tmp/Lebenslauf.pdf")));
        assert!(!collection.stage_document(Path::new("/tmp/Lebenslauf.pdf")));
        collection.toggle_pin(first);
        assert_eq!(collection.visible("", false)[0].id, first);
        assert_eq!(collection.visible("", true).len(), 1);
        assert_eq!(Collection::default().len(), 0);
        assert!(supported_document(Path::new("Zertifikat.DOCX")));
        assert!(!supported_document(Path::new("Video.mp4")));
    }
}
