//! Durable, evidence-oriented import status. Only worker-owned runs may advance.
use crate::{Result, Vault, processable_document, vault::sql_id};
use rusqlite::params;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportStage {
    Normalizing,
    Interpreting,
    Context,
    Extracting,
    Verifying,
    Complete,
}
impl ImportStage {
    pub const STEPS: [Self; 5] = [
        Self::Normalizing,
        Self::Interpreting,
        Self::Context,
        Self::Extracting,
        Self::Verifying,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normalizing => "Normalization",
            Self::Interpreting => "Interpretation",
            Self::Context => "Context",
            Self::Extracting => "Extraction",
            Self::Verifying => "Verification",
            Self::Complete => "Ready",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Normalizing => "normalizing",
            Self::Interpreting => "interpreting",
            Self::Context => "context",
            Self::Extracting => "extracting",
            Self::Verifying => "verifying",
            Self::Complete => "complete",
        }
    }
    fn from_key(key: &str) -> Self {
        match key {
            "interpreting" => Self::Interpreting,
            "context" => Self::Context,
            "extracting" => Self::Extracting,
            "verifying" => Self::Verifying,
            "complete" => Self::Complete,
            _ => Self::Normalizing,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ImportJob {
    pub item: u64,
    pub title: String,
    pub state: String,
    pub stage: ImportStage,
    pub current: u32,
    pub total: u32,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub processable: bool,
    pub proposals: u32,
    pub questions: u32,
}
impl Vault {
    /// Keep an explicitly attached Search form out of automatic fact extraction.
    /// Its existing form workflow claims it manually after intake finishes.
    pub fn defer_automatic_document(&mut self, item: u64) -> Result<()> {
        self.db.execute("UPDATE document_evaluation SET state='manual' WHERE state='queued' AND source_id=(SELECT source_id FROM collection_item WHERE local_id=?)",[sql_id(item)?])?;
        Ok(())
    }
    pub fn import_jobs(&self) -> Result<Vec<ImportJob>> {
        let mut stmt = self.db.prepare("SELECT i.local_id,i.title,e.state,coalesce(p.stage,'normalizing'),coalesce(p.current,0),coalesce(p.total,0),e.error_message,e.warning_message,i.extension,s.sensitivity,(SELECT count(*) FROM ai_proposal a WHERE a.source_id=e.source_id AND a.state='proposed'),(SELECT count(*) FROM ai_question q WHERE q.source_id=e.source_id AND q.state='pending') FROM document_evaluation e JOIN collection_item i ON i.source_id=e.source_id JOIN source s ON s.id=i.source_id LEFT JOIN import_progress p ON p.source_id=e.source_id WHERE i.deleted_at IS NULL AND s.retention='keep' ORDER BY i.local_id DESC")?;
        Ok(stmt
            .query_map([], |r| {
                let extension: String = r.get(8)?;
                let sensitivity: String = r.get(9)?;
                Ok(ImportJob {
                    item: r.get::<_, i64>(0)? as u64,
                    title: r.get(1)?,
                    state: r.get(2)?,
                    stage: ImportStage::from_key(&r.get::<_, String>(3)?),
                    current: r.get(4)?,
                    total: r.get(5)?,
                    error: r.get(6)?,
                    warning: r.get(7)?,
                    proposals: r.get(10)?,
                    questions: r.get(11)?,
                    processable: sensitivity != "credential" && processable_document(&extension),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn begin_import_progress(&mut self, item: u64, run: &str) -> Result<()> {
        self.db.execute("INSERT INTO import_progress(source_id,run_id,stage) SELECT source_id,?,'normalizing' FROM collection_item WHERE local_id=? ON CONFLICT(source_id) DO UPDATE SET run_id=excluded.run_id,stage=excluded.stage,current=0,total=0", params![run,sql_id(item)?])?;
        Ok(())
    }
    /// Counters describe real pages/sections; zero means an unknown total.
    pub fn update_import_progress(
        &mut self,
        item: u64,
        run: &str,
        stage: ImportStage,
        current: u32,
        total: u32,
    ) -> Result<bool> {
        let changed = self.db.execute("UPDATE import_progress SET stage=?,current=?,total=? WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=?) AND run_id=? AND EXISTS(SELECT 1 FROM document_evaluation e WHERE e.source_id=import_progress.source_id AND e.state='running')", params![stage.key(),current.min(total),total,sql_id(item)?,run])?;
        Ok(changed == 1)
    }
}
