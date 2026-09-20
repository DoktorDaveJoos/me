//! Durable step results and conservative spend accounting. Reservations are never
//! refunded after an uncertain network outcome; retries cannot reset the budget.
use crate::{Error, ExtractionInput, ImportStage, Result, Vault, vault::sql_id};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProvider {
    Local,
    OpenAi,
    TypeSafe,
}
impl ImportProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::OpenAi => "openai",
            Self::TypeSafe => "typesafe",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "ME.",
            Self::OpenAi => "OpenAI",
            Self::TypeSafe => "TypeSafe",
        }
    }
    pub fn from_key(key: &str) -> Self {
        match key {
            "openai" => Self::OpenAi,
            "typesafe" => Self::TypeSafe,
            _ => Self::Local,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportErrorKind {
    Quota,
    RateLimit,
    Authentication,
    Timeout,
    Connection,
    InvalidOutput,
    Budget,
    Cancelled,
    Interrupted,
    Storage,
    Unprocessable,
}
impl ImportErrorKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Quota => "quota",
            Self::RateLimit => "rate_limit",
            Self::Authentication => "authentication",
            Self::Timeout => "timeout",
            Self::Connection => "connection",
            Self::InvalidOutput => "invalid_output",
            Self::Budget => "budget",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Storage => "storage",
            Self::Unprocessable => "unprocessable",
        }
    }
    pub fn from_key(key: &str) -> Self {
        match key {
            "quota" => Self::Quota,
            "rate_limit" => Self::RateLimit,
            "authentication" => Self::Authentication,
            "timeout" => Self::Timeout,
            "connection" => Self::Connection,
            "invalid_output" => Self::InvalidOutput,
            "budget" => Self::Budget,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            "unprocessable" => Self::Unprocessable,
            _ => Self::Storage,
        }
    }
    pub fn pauses_queue(self) -> bool {
        matches!(self, Self::Quota | Self::RateLimit | Self::Authentication)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportFailure {
    pub provider: ImportProvider,
    pub kind: ImportErrorKind,
    pub message: String,
}
impl ImportFailure {
    pub fn new(
        provider: ImportProvider,
        kind: ImportErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            kind,
            message: message.into(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepProgress {
    pub current: u32,
    pub total: u32,
}
impl StepProgress {
    pub fn fraction(self) -> f32 {
        if self.total == 0 {
            0.
        } else {
            self.current.min(self.total) as f32 / self.total as f32
        }
    }
    pub fn complete(self) -> bool {
        self.total > 0 && self.current >= self.total
    }
}
#[derive(Clone, Debug)]
pub struct ImportUsage {
    pub openai_calls: u32,
    pub typesafe_calls: u32,
    pub openai_limit: u32,
    pub typesafe_limit: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub token_limit: u64,
    pub unreported_calls: u32,
}
impl Default for ImportUsage {
    fn default() -> Self {
        Self {
            openai_calls: 0,
            typesafe_calls: 0,
            openai_limit: 12,
            typesafe_limit: 24,
            input_tokens: 0,
            output_tokens: 0,
            token_limit: 180_000,
            unreported_calls: 0,
        }
    }
}
impl ImportUsage {
    pub fn exhausted(&self) -> bool {
        self.openai_calls >= self.openai_limit
            || self.typesafe_calls >= self.typesafe_limit
            || self.input_tokens.saturating_add(self.output_tokens) >= self.token_limit
    }
}
impl Vault {
    pub fn finish_import_evaluation(
        &mut self,
        item: u64,
        run: &str,
        error: Option<&str>,
        warning: Option<&str>,
    ) -> Result<bool> {
        let active:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM import_progress p JOIN collection_item i ON i.source_id=p.source_id JOIN document_evaluation e ON e.source_id=p.source_id WHERE i.local_id=? AND i.kind='document' AND p.run_id=? AND e.state='running')",params![sql_id(item)?,run],|r|r.get(0))?;
        if active {
            self.finish_evaluation_with_warning(item, error, warning)?;
        }
        Ok(active)
    }
    pub fn import_pause_reason(&self) -> Result<Option<String>> {
        Ok(self.db.query_row(
            "SELECT pause_reason FROM import_control WHERE singleton=1",
            [],
            |r| r.get(0),
        )?)
    }
    /// Explicit user action; the next provider call still verifies availability.
    pub fn resume_import_queue(&mut self) -> Result<()> {
        self.db.execute(
            "UPDATE import_control SET pause_reason=NULL WHERE singleton=1",
            [],
        )?;
        Ok(())
    }
    pub fn record_import_failure(
        &mut self,
        item: u64,
        run: &str,
        failure: &ImportFailure,
    ) -> Result<bool> {
        let tx = self.db.transaction()?;
        let changed=tx.execute("UPDATE import_progress SET error_code=?,error_provider=? WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=? AND kind='document') AND run_id=? AND EXISTS(SELECT 1 FROM document_evaluation e WHERE e.source_id=import_progress.source_id AND e.state='running')",params![failure.kind.key(),failure.provider.key(),sql_id(item)?,run])?;
        if changed == 1 && failure.kind.pauses_queue() {
            tx.execute(
                "UPDATE import_control SET pause_reason=? WHERE singleton=1",
                [failure.message.chars().take(1000).collect::<String>()],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }
    pub fn import_usage(&self, item: u64) -> Result<ImportUsage> {
        let source: String = self.db.query_row(
            "SELECT source_id FROM collection_item WHERE local_id=? AND kind='document'",
            [sql_id(item)?],
            |r| r.get(0),
        )?;
        self.source_usage(&source)
    }
    fn source_usage(&self, source: &str) -> Result<ImportUsage> {
        let mut usage=self.db.query_row("SELECT openai_limit,typesafe_limit,token_limit FROM import_budget WHERE source_id=?",[source],|r|Ok(ImportUsage {openai_limit:r.get(0)?,typesafe_limit:r.get(1)?,token_limit:r.get::<_,i64>(2)?.max(0) as u64,..Default::default()})).optional()?.unwrap_or_default();
        let row:(u32,u32,i64,i64,u32)=self.db.query_row("SELECT coalesce(sum(provider='openai'),0),coalesce(sum(provider='typesafe'),0),coalesce(sum(input_tokens),0),coalesce(sum(output_tokens),0),coalesce(sum(input_tokens IS NULL OR output_tokens IS NULL),0) FROM import_request WHERE source_id=?",[source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        usage.openai_calls = row.0;
        usage.typesafe_calls = row.1;
        usage.input_tokens = row.2.max(0) as u64;
        usage.output_tokens = row.3.max(0) as u64;
        usage.unreported_calls = row.4;
        Ok(usage)
    }
    pub fn reserve_import_request(
        &mut self,
        input: &ExtractionInput,
        provider: ImportProvider,
    ) -> Result<String> {
        self.checkpoint_active(input)?;
        if provider == ImportProvider::Local {
            return Err(Error::Validation(
                "Local work does not reserve a paid call.",
            ));
        }
        if self.import_pause_reason()?.is_some() {
            return Err(Error::Validation(
                "Imports are paused. Check the provider connection or quota before resuming.",
            ));
        }
        self.db.execute(
            "INSERT OR IGNORE INTO import_budget(source_id) VALUES(?)",
            [&input.source_id],
        )?;
        let usage = self.source_usage(&input.source_id)?;
        if (provider == ImportProvider::OpenAi && usage.openai_calls >= usage.openai_limit)
            || (provider == ImportProvider::TypeSafe
                && usage.typesafe_calls >= usage.typesafe_limit)
            || usage.input_tokens.saturating_add(usage.output_tokens) >= usage.token_limit
        {
            return Err(Error::Validation(
                "This file reached its analysis allowance. Saved steps are kept. Review usage before allowing more calls.",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.db.execute(
            "INSERT INTO import_request(id,source_id,provider) VALUES(?,?,?)",
            params![id, input.source_id, provider.key()],
        )?;
        Ok(id)
    }
    /// Absolute per-call usage, not a delta. Repeated notifications cannot double count.
    pub fn record_import_usage(
        &mut self,
        item: u64,
        run: &str,
        call: &str,
        input: u64,
        output: u64,
    ) -> Result<bool> {
        let changed=self.db.execute("UPDATE import_request SET input_tokens=max(coalesce(input_tokens,0),?),output_tokens=max(coalesce(output_tokens,0),?) WHERE id=? AND source_id=(SELECT source_id FROM collection_item WHERE local_id=? AND kind='document') AND EXISTS(SELECT 1 FROM import_progress p JOIN document_evaluation e ON e.source_id=p.source_id WHERE p.source_id=import_request.source_id AND p.run_id=? AND e.state='running')",params![input.min(i64::MAX as u64) as i64,output.min(i64::MAX as u64) as i64,call,sql_id(item)?,run])?;
        Ok(changed == 1)
    }
    /// A separately labeled UI action, never called by retries or crash recovery.
    pub fn extend_import_allowance(&mut self, item: u64) -> Result<()> {
        let running:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM document_evaluation e JOIN collection_item i ON i.source_id=e.source_id WHERE i.local_id=? AND e.state='running')",[sql_id(item)?],|r|r.get(0))?;
        if running {
            return Err(Error::Validation(
                "Stop this analysis before changing its allowance.",
            ));
        }
        self.db.execute("INSERT OR IGNORE INTO import_budget(source_id) SELECT source_id FROM collection_item WHERE local_id=? AND kind='document'",[sql_id(item)?])?;
        self.db.execute("UPDATE import_budget SET openai_limit=openai_limit+12,typesafe_limit=typesafe_limit+24,token_limit=token_limit+180000 WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=? AND kind='document')",[sql_id(item)?])?;
        Ok(())
    }
    pub fn import_step_cache(
        &self,
        input: &ExtractionInput,
        pipeline: &str,
        step: &str,
    ) -> Result<Option<Value>> {
        self.checkpoint_active(input)?;
        let value:Option<String>=self.db.query_row("SELECT output_json FROM import_step_cache WHERE source_id=? AND pipeline=? AND batch_key=? AND step=?",params![input.source_id,pipeline,crate::extraction_fingerprint(input),step],|r|r.get(0)).optional()?;
        Ok(value.and_then(|s| serde_json::from_str(&s).ok()))
    }
    /// Intermediate JSON is untrusted, encrypted working data, never a confirmed fact.
    pub fn save_import_step_cache(
        &mut self,
        input: &ExtractionInput,
        pipeline: &str,
        step: &str,
        output: &Value,
    ) -> Result<()> {
        self.checkpoint_active(input)?;
        let json = serde_json::to_string(output).map_err(|_| Error::Format)?;
        if json.len() > 512 * 1024
            || !["interpret", "extract", "verify_decision", "audit"].contains(&step)
        {
            return Err(Error::Validation("Invalid import checkpoint."));
        }
        self.db.execute("INSERT INTO import_step_cache VALUES(?,?,?,?,?) ON CONFLICT(source_id,pipeline,batch_key,step) DO UPDATE SET output_json=excluded.output_json",params![input.source_id,pipeline,crate::extraction_fingerprint(input),step,json])?;
        Ok(())
    }
    pub fn import_steps(&self, item: u64) -> Result<[StepProgress; 5]> {
        let mut steps = [StepProgress::default(); 5];
        let mut stmt=self.db.prepare("SELECT stage,current,total FROM import_step_progress WHERE source_id=(SELECT source_id FROM collection_item WHERE local_id=? AND kind='document')")?;
        for row in stmt.query_map([sql_id(item)?], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })? {
            let (stage, current, total) = row?;
            if let Some(index) = ImportStage::from_key(&stage).index() {
                steps[index] = StepProgress {
                    current: current.min(total),
                    total,
                };
            }
        }
        Ok(steps)
    }
}
