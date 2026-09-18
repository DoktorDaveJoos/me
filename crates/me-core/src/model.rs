use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Note(String),
    Document {
        extension: String,
        state: String,
    },
    Credential {
        category: String,
        vault: String,
        archived: bool,
    },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u64,
    pub stable_id: String,
    pub title: String,
    pub content: Content,
    pub pinned: bool,
}

#[derive(Clone, Default)]
pub struct Collection {
    pub items: Vec<Item>,
    pub total: usize,
    pub pins: usize,
}
impl Collection {
    pub fn len(&self) -> usize {
        self.total
    }
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
    pub fn has_pins(&self) -> bool {
        self.pins > 0
    }
    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct Revision {
    pub value: String,
    pub state: String,
    pub recorded_at: String,
    pub source_title: String,
}

#[derive(Clone, Copy)]
pub enum DocumentClass {
    /// Preserve only. No automatic document-content indexing before review.
    Unclassified,
    /// Explicit user classification; document content can be indexed locally.
    Personal,
    /// Never searchable by document content.
    Credential,
}

pub fn supported_document(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            [
                "pdf", "png", "jpg", "jpeg", "heic", "heif", "webp", "tif", "tiff", "bmp", "gif",
                "txt", "md", "tsv", "log", "rtf", "doc", "docx", "odt", "xls", "xlsx", "ods",
                "csv", "ppt", "pptx", "odp", "eml",
            ]
            .contains(&ext.to_ascii_lowercase().as_str())
        })
}

/// Formats with a content extractor, independently of storage support.
pub fn processable_document(extension: &str) -> bool {
    [
        "pdf", "png", "jpg", "jpeg", "heic", "heif", "webp", "tif", "tiff", "bmp", "gif", "txt",
        "md", "csv", "tsv", "log", "docx", "odt", "rtf", "eml",
    ]
    .contains(&extension.to_ascii_lowercase().as_str())
}
