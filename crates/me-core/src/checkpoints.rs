//! Reusable, encrypted results for immutable source segments. No model output
//! becomes a proposal until the complete document run finishes validation.
use crate::{Error, ExtractionInput, ExtractionOutput, Result, Vault};
use rusqlite::{OptionalExtension, params};

pub fn extraction_fingerprint(input: &ExtractionInput) -> String {
    crate::vault::hash(
        serde_json::json!({"source":input.source_id,"title":input.title,"segments":input.segments})
            .to_string()
            .as_bytes(),
    )
}
impl Vault {
    pub(crate) fn checkpoint_active(&self, input: &ExtractionInput) -> Result<()> {
        let active: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM extraction_run r JOIN source s ON s.id=r.source_id WHERE r.id=? AND r.source_id=? AND r.status='running' AND s.sensitivity='personal' AND s.retention='keep')", params![input.run_id,input.source_id], |r| r.get(0))?;
        if !active {
            return Err(Error::Validation("This analysis is no longer active."));
        }
        Ok(())
    }
    pub fn extraction_checkpoint(
        &self,
        input: &ExtractionInput,
        pipeline: &str,
    ) -> Result<Option<ExtractionOutput>> {
        self.checkpoint_active(input)?;
        let json: Option<String> = self.db.query_row("SELECT output_json FROM extraction_checkpoint WHERE source_id=? AND pipeline=? AND batch_key=?", params![input.source_id,pipeline,extraction_fingerprint(input)], |r| r.get(0)).optional()?;
        // A damaged/incompatible cache is disposable; it must never block retries.
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }
    pub fn save_extraction_checkpoint(
        &mut self,
        input: &ExtractionInput,
        pipeline: &str,
        output: &ExtractionOutput,
    ) -> Result<()> {
        self.checkpoint_active(input)?;
        let grounded = crate::ground_extraction(input, output.clone());
        // Source-supported values whose only open question is ownership are kept:
        // restoring them re-derives the same ownership question for the user.
        if grounded
            .rejected
            .iter()
            .any(|r| r.code != "subject_unknown")
        {
            return Err(Error::Validation(
                "Unsupported intermediate results cannot be saved.",
            ));
        }
        let mut saved = grounded.output;
        saved
            .facts
            .extend(grounded.rejected.into_iter().map(|r| r.fact));
        self.db.execute("INSERT INTO extraction_checkpoint VALUES(?,?,?,?) ON CONFLICT(source_id,pipeline,batch_key) DO UPDATE SET output_json=excluded.output_json", params![input.source_id,pipeline,extraction_fingerprint(input),serde_json::to_string(&saved).map_err(|_| Error::Format)?])?;
        Ok(())
    }
}
