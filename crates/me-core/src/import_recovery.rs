//! Durable step results and conservative spend accounting. Reservations are never
//! refunded after an uncertain network outcome; retries cannot reset the budget.
use crate::{Error, ExtractionInput, ImportStage, Result, Vault, vault::sql_id};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
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
    /// The file's own allowance is used up.
    Budget,
    /// The OpenAI allowance of the import the file arrived with is used up.
    BatchBudget,
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
            Self::BatchBudget => "batch_budget",
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
            "batch_budget" => Self::BatchBudget,
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
    /// A paid call the vault refused to reserve: the file's allowance stop is a
    /// budget failure, its import batch's a batch budget failure, a paused queue a
    /// quota failure and anything else a storage failure.
    pub fn refused_reservation(error: &Error) -> Self {
        let kind = match error {
            Error::Validation(m) if *m == FILE_ALLOWANCE_REACHED => ImportErrorKind::Budget,
            Error::Validation(m) if *m == IMPORT_ALLOWANCE_REACHED => ImportErrorKind::BatchBudget,
            Error::Validation(m) if *m == IMPORTS_PAUSED => ImportErrorKind::Quota,
            _ => ImportErrorKind::Storage,
        };
        Self::new(ImportProvider::Local, kind, error.to_string())
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
    /// The import the file arrived with most recently; its OpenAI allowance gates
    /// the file's OpenAI calls too.
    pub batch: Option<BatchAllowance>,
}
/// The OpenAI allowance of one import batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchAllowance {
    pub id: String,
    pub openai_calls: i64,
    pub openai_allowance: i64,
}
impl BatchAllowance {
    pub fn exhausted(&self) -> bool {
        self.openai_calls >= self.openai_allowance
    }
}
/// Default per-file allowance. A full read needs one Classify request, two TypeSafe
/// requests per reader section and one verification request per 20 facts.
pub const FILE_OPENAI_CALLS: u32 = 12;
pub const FILE_TYPESAFE_CALLS: u32 = 64;
pub const FILE_TOKENS: u64 = 180_000;
/// What one explicit "allow more" adds to a file's allowance.
pub const EXTEND_OPENAI_CALLS: u32 = 12;
pub const EXTEND_TYPESAFE_CALLS: u32 = 24;
pub const EXTEND_TOKENS: u64 = 180_000;

impl Default for ImportUsage {
    fn default() -> Self {
        Self {
            openai_calls: 0,
            typesafe_calls: 0,
            openai_limit: FILE_OPENAI_CALLS,
            typesafe_limit: FILE_TYPESAFE_CALLS,
            input_tokens: 0,
            output_tokens: 0,
            token_limit: FILE_TOKENS,
            unreported_calls: 0,
            batch: None,
        }
    }
}

const FILE_ALLOWANCE_REACHED: &str = "This file reached its analysis allowance. Saved steps are kept. Review usage before allowing more calls.";
pub(crate) const IMPORT_ALLOWANCE_REACHED: &str = "This import reached its OpenAI allowance. Saved steps are kept. Allow more OpenAI calls for this import to continue.";
const IMPORTS_PAUSED: &str =
    "Imports are paused. Check the provider connection or quota before resuming.";

/// A file's usage and effective allowance. Limits are never below the current
/// defaults, so a row written with an earlier, lower default is not binding.
fn usage_of(db: &Connection, source: &str) -> Result<ImportUsage> {
    let mut usage = db
        .query_row(
            "SELECT max(openai_limit,?),max(typesafe_limit,?),max(token_limit,?) FROM import_budget WHERE source_id=?",
            params![FILE_OPENAI_CALLS, FILE_TYPESAFE_CALLS, FILE_TOKENS as i64, source],
            |r| {
                Ok(ImportUsage {
                    openai_limit: r.get(0)?,
                    typesafe_limit: r.get(1)?,
                    token_limit: r.get::<_, i64>(2)?.max(0) as u64,
                    ..Default::default()
                })
            },
        )
        .optional()?
        .unwrap_or_default();
    let row:(u32,u32,i64,i64,u32)=db.query_row("SELECT coalesce(sum(provider='openai'),0),coalesce(sum(provider='typesafe'),0),coalesce(sum(input_tokens),0),coalesce(sum(output_tokens),0),coalesce(sum(input_tokens IS NULL OR output_tokens IS NULL),0) FROM import_request WHERE source_id=?",[source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    usage.openai_calls = row.0;
    usage.typesafe_calls = row.1;
    usage.input_tokens = row.2.max(0) as u64;
    usage.output_tokens = row.3.max(0) as u64;
    usage.unreported_calls = row.4;
    usage.batch = match crate::intake::source_batch(db, source)? {
        Some(id) => db
            .query_row(
                "SELECT openai_calls,openai_allowance FROM import_batch WHERE id=?",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .map(|(openai_calls, openai_allowance)| BatchAllowance {
                id,
                openai_calls,
                openai_allowance,
            }),
        None => None,
    };
    Ok(usage)
}

/// A file's budget row with at least the current default allowance. A row with
/// an earlier, lower default is raised; an extended limit is never lowered.
fn ensure_budget(db: &Connection, source: &str) -> Result<()> {
    db.execute(
        "INSERT INTO import_budget(source_id,openai_limit,typesafe_limit,token_limit) VALUES(?,?,?,?) ON CONFLICT(source_id) DO UPDATE SET openai_limit=max(openai_limit,excluded.openai_limit),typesafe_limit=max(typesafe_limit,excluded.typesafe_limit),token_limit=max(token_limit,excluded.token_limit)",
        params![source, FILE_OPENAI_CALLS, FILE_TYPESAFE_CALLS, FILE_TOKENS as i64],
    )?;
    Ok(())
}

/// Reserves one paid call of `source` against its file allowance and, when the file
/// arrived with an import batch, against the batch too: an OpenAI call needs the
/// batch allowance, a TypeSafe request is counted. Nothing is charged when either
/// allowance refuses the call.
fn reserve_request(tx: &Transaction<'_>, source: &str, provider: ImportProvider) -> Result<String> {
    ensure_budget(tx, source)?;
    let usage = usage_of(tx, source)?;
    if (provider == ImportProvider::OpenAi && usage.openai_calls >= usage.openai_limit)
        || (provider == ImportProvider::TypeSafe && usage.typesafe_calls >= usage.typesafe_limit)
        || usage.input_tokens.saturating_add(usage.output_tokens) >= usage.token_limit
    {
        return Err(Error::Validation(FILE_ALLOWANCE_REACHED));
    }
    if let Some(batch) = crate::intake::source_batch(tx, source)? {
        let charged = match provider {
            ImportProvider::OpenAi => crate::intake::charge_batch_openai_call(tx, &batch)?,
            _ => {
                tx.execute(
                    "UPDATE import_batch SET typesafe_requests=typesafe_requests+1 WHERE id=?",
                    [&batch],
                )?;
                true
            }
        };
        if !charged {
            return Err(Error::Validation(IMPORT_ALLOWANCE_REACHED));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO import_request(id,source_id,provider) VALUES(?,?,?)",
        params![id, source, provider.key()],
    )?;
    Ok(id)
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
        usage_of(&self.db, source)
    }
    /// Refuses a paid call before any allowance is consulted: local work and a
    /// paused queue never reserve one.
    fn reservable(&self, provider: ImportProvider) -> Result<()> {
        if provider == ImportProvider::Local {
            return Err(Error::Validation(
                "Local work does not reserve a paid call.",
            ));
        }
        if self.import_pause_reason()?.is_some() {
            return Err(Error::Validation(IMPORTS_PAUSED));
        }
        Ok(())
    }
    pub fn reserve_import_request(
        &mut self,
        input: &ExtractionInput,
        provider: ImportProvider,
    ) -> Result<String> {
        self.checkpoint_active(input)?;
        self.reservable(provider)?;
        let tx = self.db.transaction()?;
        let id = reserve_request(&tx, &input.source_id, provider)?;
        tx.commit()?;
        Ok(id)
    }
    /// Refuses to start a paid step of `item` when the import it arrived with has
    /// no OpenAI calls left, with the same refusal a reservation would give.
    /// Nothing is reserved or charged.
    pub fn ensure_batch_allowance(&self, item: u64) -> Result<()> {
        if self
            .import_usage(item)?
            .batch
            .is_some_and(|b| b.exhausted())
        {
            return Err(Error::Validation(IMPORT_ALLOWANCE_REACHED));
        }
        Ok(())
    }
    /// Reservation for Classify and verification requests, bound to the running
    /// attempt `run` of `item` instead of a reader checkpoint. Same file and batch
    /// allowances, batch accounting and pause.
    pub fn reserve_graph_request(
        &mut self,
        item: u64,
        run: &str,
        provider: ImportProvider,
    ) -> Result<String> {
        let source: String = self.db.query_row("SELECT i.source_id FROM collection_item i JOIN import_progress p ON p.source_id=i.source_id JOIN document_evaluation e ON e.source_id=i.source_id WHERE i.local_id=? AND i.kind='document' AND p.run_id=? AND e.state='running'",params![sql_id(item)?,run],|r|r.get(0)).optional()?.ok_or(Error::Validation("This import attempt is no longer active."))?;
        self.reservable(provider)?;
        let tx = self.db.transaction()?;
        let id = reserve_request(&tx, &source, provider)?;
        tx.commit()?;
        Ok(id)
    }
    /// Absolute per-call usage, not a delta. Repeated notifications cannot double count.
    /// Reported TypeSafe input tokens also count toward the file's import batch.
    pub fn record_import_usage(
        &mut self,
        item: u64,
        run: &str,
        call: &str,
        input: u64,
        output: u64,
    ) -> Result<bool> {
        let input = input.min(i64::MAX as u64) as i64;
        let tx = self.db.transaction()?;
        let before: Option<(String, String, Option<i64>)> = tx
            .query_row(
                "SELECT source_id,provider,input_tokens FROM import_request WHERE id=?",
                [call],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let changed=tx.execute("UPDATE import_request SET input_tokens=max(coalesce(input_tokens,0),?),output_tokens=max(coalesce(output_tokens,0),?) WHERE id=? AND source_id=(SELECT source_id FROM collection_item WHERE local_id=? AND kind='document') AND EXISTS(SELECT 1 FROM import_progress p JOIN document_evaluation e ON e.source_id=p.source_id WHERE p.source_id=import_request.source_id AND p.run_id=? AND e.state='running')",params![input,output.min(i64::MAX as u64) as i64,call,sql_id(item)?,run])?;
        if changed == 1
            && let Some((source, provider, previous)) = before
            && provider == ImportProvider::TypeSafe.key()
            && input > previous.unwrap_or(0)
            && let Some(batch) = crate::intake::source_batch(&tx, &source)?
        {
            tx.execute(
                "UPDATE import_batch SET typesafe_input_tokens=typesafe_input_tokens+? WHERE id=?",
                params![input - previous.unwrap_or(0), batch],
            )?;
        }
        tx.commit()?;
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
        let Some(source): Option<String> = self
            .db
            .query_row(
                "SELECT source_id FROM collection_item WHERE local_id=? AND kind='document'",
                [sql_id(item)?],
                |r| r.get(0),
            )
            .optional()?
        else {
            return Ok(());
        };
        ensure_budget(&self.db, &source)?;
        self.db.execute("UPDATE import_budget SET openai_limit=openai_limit+?,typesafe_limit=typesafe_limit+?,token_limit=token_limit+? WHERE source_id=?",params![EXTEND_OPENAI_CALLS,EXTEND_TYPESAFE_CALLS,EXTEND_TOKENS as i64,source])?;
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
            || ![
                "interpret",
                "extract",
                "verify_decision",
                "audit",
                "sweep_audit",
            ]
            .contains(&step)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentClass;

    #[test]
    fn older_file_allowances_rise_to_the_current_defaults_without_lowering_extensions() {
        let t = tempfile::tempdir().unwrap();
        let mut v = Vault::create(&t.path().join("vault"), "synthetic-budget-password").unwrap();
        let mut items = Vec::new();
        for name in ["older.txt", "extended.txt"] {
            let file = t.path().join(name);
            std::fs::write(&file, format!("SYNTHETIC {name}")).unwrap();
            items.push(
                v.import_document(&file, name, DocumentClass::Personal)
                    .unwrap(),
            );
        }
        let (older, extended) = (items[0], items[1]);
        let source = |v: &Vault, item| v.document_texts(item).unwrap().0;
        // A row written with the earlier defaults, and one extended beyond today's.
        for (item, limits) in [(older, (12, 24, 180_000)), (extended, (36, 96, 540_000))] {
            v.db.execute(
                "INSERT INTO import_budget(source_id,openai_limit,typesafe_limit,token_limit) VALUES(?,?,?,?)",
                params![source(&v, item), limits.0, limits.1, limits.2],
            )
            .unwrap();
        }
        let limits = |v: &Vault, item| {
            let u = v.import_usage(item).unwrap();
            (u.openai_limit, u.typesafe_limit, u.token_limit)
        };
        let stored = |v: &Vault, item| -> (u32, u32, u64) {
            v.db.query_row(
                "SELECT openai_limit,typesafe_limit,token_limit FROM import_budget WHERE source_id=?",
                [source(v, item)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64)),
            )
            .unwrap()
        };
        let defaults = (FILE_OPENAI_CALLS, FILE_TYPESAFE_CALLS, FILE_TOKENS);
        // Shown at no less than today's defaults before any new call.
        assert_eq!(limits(&v, older), defaults);
        assert_eq!(limits(&v, extended), (36, 96, 540_000));
        // Enforced and stored that way once the file reserves a call.
        for (item, run) in [(older, "o"), (extended, "e")] {
            v.begin_evaluation(item, false).unwrap();
            v.begin_import_progress(item, run).unwrap();
            v.reserve_graph_request(item, run, ImportProvider::TypeSafe)
                .unwrap();
        }
        assert_eq!(stored(&v, older), defaults);
        assert_eq!(stored(&v, extended), (36, 96, 540_000));
        for _ in 1..FILE_TYPESAFE_CALLS {
            v.reserve_graph_request(older, "o", ImportProvider::TypeSafe)
                .unwrap();
        }
        assert!(
            v.reserve_graph_request(older, "o", ImportProvider::TypeSafe)
                .is_err()
        );
        // An explicit extension adds to the raised limit.
        v.finish_import_evaluation(older, "o", Some("SYNTHETIC stop"), None)
            .unwrap();
        v.extend_import_allowance(older).unwrap();
        assert_eq!(
            limits(&v, older).1,
            FILE_TYPESAFE_CALLS + EXTEND_TYPESAFE_CALLS
        );
    }
}
