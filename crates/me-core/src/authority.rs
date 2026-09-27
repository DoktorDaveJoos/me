//! Task-scoped capabilities and revision-bound action approval.
//! Issuance/approval APIs are trusted UI APIs, never exposed as agent tools.
use crate::{Error, FactResolution, Result, Vault, domain::bounded, vault::id};
use rusqlite::{OptionalExtension, params};
use serde_json::json;
use zeroize::Zeroizing;

#[derive(Clone, Debug)]
pub enum AccessResource {
    Fact { entity: String, property: String },
    Source(String),
    Credential { credential: String, origin: String },
    Action(String),
}
impl AccessResource {
    fn parts(&self) -> Result<(&'static str, String)> {
        Ok(match self {
            Self::Fact { entity, property } => ("fact", json!([entity, property]).to_string()),
            Self::Source(id) => ("source", id.clone()),
            Self::Credential { credential, origin } => {
                let u = url::Url::parse(origin)
                    .map_err(|_| Error::Validation("Invalid credential destination."))?;
                if u.scheme() != "https"
                    || !u.username().is_empty()
                    || u.password().is_some()
                    || u.query().is_some()
                    || u.fragment().is_some()
                    || u.path() != "/"
                {
                    return Err(Error::Validation(
                        "Use an exact HTTPS origin for credential use.",
                    ));
                }
                (
                    "credential",
                    json!([credential, u.origin().ascii_serialization()]).to_string(),
                )
            }
            Self::Action(id) => ("action", id.clone()),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessOperation {
    Read,
    CredentialUse,
    SecretReveal,
    Draft,
    Execute,
}
impl AccessOperation {
    fn key(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::CredentialUse => "credential_use",
            Self::SecretReveal => "secret_reveal",
            Self::Draft => "draft",
            Self::Execute => "execute",
        }
    }
}
/// Opaque process-local capability. It cannot be manufactured from agent JSON.
#[derive(Clone)]
pub struct AccessGrant {
    id: String,
    principal: String,
    task: String,
    expires: std::time::Instant,
}
impl AccessGrant {
    pub fn id(&self) -> &str {
        &self.id
    }
}
pub struct ActionDraft {
    pub operation: String,
    pub recipient: String,
    pub body: String,
    pub attachments: Vec<String>,
}
#[derive(Clone, Copy, Debug)]
pub enum ActionOutcome {
    Succeeded,
    Failed,
    Indeterminate,
}
pub struct BrokerCredential {
    pub fields: Vec<(String, Zeroizing<String>)>,
}

impl Vault {
    pub fn register_agent_principal(&mut self, label: &str) -> Result<String> {
        bounded(label, 200)?;
        let principal = id();
        self.db.execute(
            "INSERT INTO principal(id,label,kind) VALUES(?,?,'agent')",
            params![principal, label],
        )?;
        Ok(principal)
    }
    pub fn disable_principal(&mut self, principal: &str) -> Result<()> {
        if self.db.execute("UPDATE principal SET disabled_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND disabled_at IS NULL",[principal])?!=1{return Err(Error::Validation("Principal is unavailable."));}
        Ok(())
    }
    pub fn create_action_intent(
        &mut self,
        title: &str,
        entity: Option<&str>,
        source: Option<&str>,
        deadline: Option<&str>,
    ) -> Result<String> {
        bounded(title, 500)?;
        if let Some(e) = entity {
            crate::domain::canonical_entity(&self.db, e)?;
        }
        let due = if let Some(assertion) = deadline {
            let raw:String=self.db.query_row("SELECT value_json FROM assertion_state WHERE id=? AND state='accept' AND value_type='date'",[assertion],|r|r.get(0)).optional()?.ok_or(Error::Validation("A deadline needs an accepted date assertion."))?;
            let date: String = serde_json::from_str(&raw).map_err(|_| Error::Format)?;
            crate::domain::date(&date)?;
            Some(date)
        } else {
            None
        };
        let task = id();
        self.db.execute("INSERT INTO action_intent(id,title,entity_id,source_id,due_date,deadline_assertion_id,created_at) VALUES(?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![task,title,entity,source,due,deadline])?;
        Ok(task)
    }
    fn check_resource(&self, resource: &AccessResource) -> Result<()> {
        let exists=match resource{
   AccessResource::Fact{entity,property}=>{crate::domain::canonical_entity(&self.db,entity)?;self.db.query_row("SELECT EXISTS(SELECT 1 FROM property_definition WHERE key=?)",[property],|r|r.get::<_,bool>(0))?},
   AccessResource::Source(source)=>self.db.query_row("SELECT EXISTS(SELECT 1 FROM source WHERE id=? AND sensitivity='personal' AND retention='keep')",[source],|r|r.get(0))?,
   AccessResource::Credential{credential,..}=>self.db.query_row("SELECT EXISTS(SELECT 1 FROM collection_item i JOIN credential_identity c ON c.id=i.stable_id WHERE c.id=? AND i.deleted_at IS NULL)",[credential],|r|r.get(0))?,
   AccessResource::Action(task)=>self.db.query_row("SELECT EXISTS(SELECT 1 FROM action_intent WHERE id=? AND deleted_at IS NULL)",[task],|r|r.get(0))?
  };
        if exists {
            Ok(())
        } else {
            Err(Error::Validation("The requested resource is unavailable."))
        }
    }
    pub fn issue_access_grant(
        &mut self,
        principal: &str,
        task: &str,
        resource: &AccessResource,
        operation: AccessOperation,
        purpose: &str,
        seconds: u32,
    ) -> Result<AccessGrant> {
        bounded(purpose, 500)?;
        if !(1..=3600).contains(&seconds) {
            return Err(Error::Validation(
                "A grant must last between one second and one hour.",
            ));
        }
        let valid = match resource {
            AccessResource::Fact { .. } | AccessResource::Source(_) => {
                operation == AccessOperation::Read
            }
            AccessResource::Credential { .. } => matches!(
                operation,
                AccessOperation::CredentialUse | AccessOperation::SecretReveal
            ),
            AccessResource::Action(action) => {
                action == task
                    && matches!(operation, AccessOperation::Draft | AccessOperation::Execute)
            }
        };
        if !valid {
            return Err(Error::Validation(
                "This operation does not match the resource.",
            ));
        }
        self.check_resource(resource)?;
        let available:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM principal WHERE id=? AND disabled_at IS NULL) AND EXISTS(SELECT 1 FROM action_intent WHERE id=? AND deleted_at IS NULL)",params![principal,task],|r|r.get(0))?;
        if !available {
            return Err(Error::Validation("Principal or task unavailable."));
        }
        let (kind, resource_id) = resource.parts()?;
        let grant = id();
        self.db.execute(
            "INSERT INTO capability_grant VALUES(?,?,?,?,?,?,?,unixepoch(),unixepoch()+?,NULL,?)",
            params![
                grant,
                principal,
                task,
                kind,
                resource_id,
                operation.key(),
                purpose,
                seconds,
                self.device_identity()?
            ],
        )?;
        Ok(AccessGrant {
            id: grant,
            principal: principal.into(),
            task: task.into(),
            expires: std::time::Instant::now() + std::time::Duration::from_secs(u64::from(seconds)),
        })
    }
    pub fn revoke_access_grant(&mut self, grant: &AccessGrant) -> Result<()> {
        self.db.execute(
            "UPDATE capability_grant SET revoked_at=unixepoch() WHERE id=? AND revoked_at IS NULL",
            [&grant.id],
        )?;
        Ok(())
    }
    fn authorize(
        &mut self,
        grant: &AccessGrant,
        resource: &AccessResource,
        operation: AccessOperation,
    ) -> Result<()> {
        let (kind, resource_id) = resource.parts()?;
        let device = self.device_identity()?;
        let allowed:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM capability_grant g JOIN principal p ON p.id=g.principal_id JOIN action_intent t ON t.id=g.task_id WHERE g.id=? AND g.principal_id=? AND g.task_id=? AND g.resource_kind=? AND g.resource_id=? AND g.operation=? AND g.device_id=? AND g.revoked_at IS NULL AND g.issued_at<=unixepoch() AND g.expires_at>unixepoch() AND p.disabled_at IS NULL AND t.deleted_at IS NULL)",params![grant.id,grant.principal,grant.task,kind,resource_id,operation.key(),device],|r|r.get(0))?;
        let allowed = allowed
            && std::time::Instant::now() < grant.expires
            && self.check_resource(resource).is_ok();
        self.db.execute("INSERT INTO access_audit VALUES(?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?)",params![id(),grant.principal,grant.task,kind,resource_id,operation.key(),grant.id,if allowed{"allowed"}else{"denied"},device])?;
        if allowed {
            Ok(())
        } else {
            Err(Error::Validation(
                "This capability is expired, revoked or outside its scope.",
            ))
        }
    }
    pub fn read_granted_source(
        &mut self,
        grant: &AccessGrant,
        source: &str,
    ) -> Result<Vec<crate::SourceText>> {
        self.authorize(
            grant,
            &AccessResource::Source(source.into()),
            AccessOperation::Read,
        )?;
        let rows=self.db.prepare("SELECT id,ordinal,text FROM source_segment WHERE source_id=? ORDER BY ordinal LIMIT 10001")?.query_map([source],|r|Ok(crate::SourceText{segment_id:r.get(0)?,ordinal:r.get(1)?,text:r.get(2)?}))?.collect::<std::result::Result<Vec<_>,_>>()?;
        if rows.len() > 10000
            || rows.iter().map(|r| r.text.len()).sum::<usize>() > crate::MAX_DOCUMENT_TEXT
        {
            return Err(Error::Validation(
                "The authorized source exceeds the read limit.",
            ));
        }
        Ok(rows)
    }
    pub fn draft_granted_action(
        &mut self,
        grant: &AccessGrant,
        task: &str,
        expected: Option<&str>,
        draft: &ActionDraft,
    ) -> Result<String> {
        self.authorize(
            grant,
            &AccessResource::Action(task.into()),
            AccessOperation::Draft,
        )?;
        self.draft_action(task, expected, draft)
    }
    pub fn delete_action_intent(&mut self, task: &str) -> Result<()> {
        if self.db.execute("UPDATE action_intent SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND deleted_at IS NULL",[task])?!=1{return Err(Error::Validation("Task unavailable."));}
        Ok(())
    }
    pub fn read_granted_fact(
        &mut self,
        grant: &AccessGrant,
        entity: &str,
        property: &str,
        as_of: &str,
    ) -> Result<FactResolution> {
        self.authorize(
            grant,
            &AccessResource::Fact {
                entity: entity.into(),
                property: property.into(),
            },
            AccessOperation::Read,
        )?;
        let mut result = self.resolve_fact_at(entity, property, as_of)?;
        // A field grant never releases restricted evidence, even through a derived claim.
        result.facts.retain(|f|self.db.query_row("SELECT EXISTS(SELECT 1 FROM assertion_evidence e JOIN source s ON s.id=e.source_id WHERE e.assertion_id=? AND s.sensitivity='personal' AND s.retention='keep') AND NOT EXISTS(SELECT 1 FROM assertion_derivation d WHERE d.assertion_id=?) AND EXISTS(SELECT 1 FROM assertion a WHERE a.id=? AND a.subject_id=?)",params![f.id,f.id,f.id,entity],|r|r.get::<_,bool>(0)).unwrap_or(false));
        for fact in &mut result.facts {
            fact.subject = entity.to_owned();
        }
        let many: bool = self.db.query_row(
            "SELECT cardinality='many' FROM property_definition WHERE key=?",
            [property],
            |r| r.get(0),
        )?;
        result.status = crate::domain::resolution_status(&result.facts, many);
        Ok(result)
    }
    /// Broker-only API. Never serialize this result into model context or an MCP tool.
    /// The trusted platform adapter must revalidate the destination immediately before use.
    pub fn credential_for_broker(
        &mut self,
        grant: &AccessGrant,
        credential: &str,
        origin: &str,
    ) -> Result<BrokerCredential> {
        self.authorize(
            grant,
            &AccessResource::Credential {
                credential: credential.into(),
                origin: origin.into(),
            },
            AccessOperation::CredentialUse,
        )?;
        let item: i64 = self.db.query_row(
            "SELECT local_id FROM collection_item WHERE stable_id=? AND deleted_at IS NULL",
            [credential],
            |r| r.get(0),
        )?;
        let details = self.login_details(item as u64)?;
        Ok(BrokerCredential {
            fields: details
                .fields
                .into_iter()
                .filter(|f| f.editable)
                .map(|f| (f.key, f.value))
                .collect(),
        })
    }
    pub fn draft_action(
        &mut self,
        task: &str,
        expected: Option<&str>,
        draft: &ActionDraft,
    ) -> Result<String> {
        bounded(&draft.operation, 80)?;
        bounded(&draft.recipient, 1000)?;
        bounded(&draft.body, 128 * 1024)?;
        if draft.attachments.len() > 32 {
            return Err(Error::Validation("Too many attachments."));
        }
        self.check_resource(&AccessResource::Action(task.into()))?;
        let tx = self.db.transaction()?;
        let old: Option<String> = tx
            .query_row(
                "SELECT revision_id FROM action_head WHERE task_id=?",
                [task],
                |r| r.get(0),
            )
            .optional()?;
        if old.as_deref() != expected {
            return Err(Error::Validation(
                "The action changed. Review its current revision.",
            ));
        }
        for source in &draft.attachments {
            let permitted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM source WHERE id=? AND sensitivity='personal' AND retention='keep' AND object_id IS NOT NULL)",[source],|r|r.get(0))?;
            if !permitted {
                return Err(Error::Validation(
                    "An attachment is unavailable or restricted.",
                ));
            }
        }
        let revision = id();
        tx.execute("INSERT INTO action_revision VALUES(?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![revision,task,old,draft.operation,draft.recipient,draft.body,json!(draft.attachments).to_string()])?;
        tx.execute("INSERT INTO action_head VALUES(?,?) ON CONFLICT(task_id) DO UPDATE SET revision_id=excluded.revision_id",params![task,revision])?;
        tx.execute("UPDATE action_approval SET revoked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE revision_id IN (SELECT id FROM action_revision WHERE task_id=?) AND revoked_at IS NULL",[task])?;
        tx.commit()?;
        Ok(revision)
    }
    pub fn approve_action(&mut self, revision: &str) -> Result<String> {
        let actor = self.profile_entity_id()?;
        let approval = id();
        let changed=self.db.execute("INSERT INTO action_approval SELECT ?,r.id,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL FROM action_revision r JOIN action_head h ON h.revision_id=r.id JOIN action_intent t ON t.id=r.task_id WHERE r.id=? AND t.deleted_at IS NULL",params![approval,actor,revision])?;
        if changed != 1 {
            return Err(Error::Validation(
                "Only the current action revision can be approved.",
            ));
        }
        Ok(approval)
    }
    /// Reserve one local execution attempt. A remote executor/lease is deliberately absent.
    pub fn begin_action_attempt(
        &mut self,
        grant: &AccessGrant,
        revision: &str,
        approval: &str,
    ) -> Result<String> {
        let task: String = self.db.query_row(
            "SELECT task_id FROM action_revision WHERE id=?",
            [revision],
            |r| r.get(0),
        )?;
        self.authorize(
            grant,
            &AccessResource::Action(task),
            AccessOperation::Execute,
        )?;
        let attachments: String = self.db.query_row(
            "SELECT attachments_json FROM action_revision WHERE id=?",
            [revision],
            |r| r.get(0),
        )?;
        let attachments: Vec<String> =
            serde_json::from_str(&attachments).map_err(|_| Error::Format)?;
        for source in attachments {
            self.check_resource(&AccessResource::Source(source))?;
        }
        let attempt = id();
        let changed=self.db.execute("INSERT INTO action_attempt SELECT ?,r.id,a.id,?,?,'started',strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL FROM action_revision r JOIN action_head h ON h.revision_id=r.id JOIN action_approval a ON a.revision_id=r.id WHERE r.id=? AND a.id=? AND a.revoked_at IS NULL AND NOT EXISTS(SELECT 1 FROM action_attempt WHERE revision_id=r.id)",params![attempt,grant.id,self.device_identity()?,revision,approval])?;
        if changed != 1 {
            return Err(Error::Validation(
                "The action is stale, unapproved or already attempted.",
            ));
        }
        Ok(attempt)
    }
    pub fn finish_action_attempt(&mut self, attempt: &str, outcome: ActionOutcome) -> Result<()> {
        let state = match outcome {
            ActionOutcome::Succeeded => "succeeded",
            ActionOutcome::Failed => "failed",
            ActionOutcome::Indeterminate => "indeterminate",
        };
        if self.db.execute("UPDATE action_attempt SET state=?,finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND state='started' AND device_id=?",params![state,attempt,self.device_identity()?])?!=1{return Err(Error::Validation("This attempt is not active on this device."));}
        Ok(())
    }
}
