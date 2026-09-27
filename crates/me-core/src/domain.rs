//! Canonical entities, evidence-backed typed assertions and temporal resolution.
//! Mutation APIs are trusted application APIs, never agent tool arguments.
use crate::{
    Error, Result, Vault,
    vault::{hash, id},
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Person,
    Organization,
    Address,
    Vehicle,
    Property,
    BankAccount,
    InsuranceContract,
    Contract,
    Subscription,
    GovernmentId,
    Credential,
    Conversation,
    Payment,
}
/// Recurrence of a monetary amount. Absent means a one-off or total amount.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Day,
    Week,
    Month,
    Quarter,
    HalfYear,
    Year,
}
impl EntityKind {
    pub(crate) fn key(&self) -> String {
        serde_json::to_value(self).unwrap().as_str().unwrap().into()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalEntity {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FactValue {
    Text(String),
    Identifier(String),
    Date(String),
    Integer(i64),
    Decimal(String),
    /// `period` makes a recurring amount explicit, e.g. a premium per month.
    Money {
        amount: String,
        currency: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        period: Option<Period>,
    },
    Quantity {
        amount: String,
        unit: String,
    },
    Boolean(bool),
    Entity(String),
    Json(Value),
}
impl FactValue {
    pub fn value_type(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Identifier(_) => "identifier",
            Self::Date(_) => "date",
            Self::Integer(_) => "integer",
            Self::Decimal(_) => "decimal",
            Self::Money { .. } => "money",
            Self::Quantity { .. } => "quantity",
            Self::Boolean(_) => "boolean",
            Self::Entity(_) => "entity",
            Self::Json(_) => "json",
        }
    }
    fn normalized(&self) -> Result<Value> {
        Ok(match self {
            Self::Text(s) | Self::Identifier(s) => {
                bounded(s, 16000)?;
                json!(s)
            }
            Self::Date(s) => {
                date(s)?;
                json!(s)
            }
            Self::Integer(n) => json!(n),
            Self::Decimal(s) => json!(decimal(s)?),
            Self::Money {
                amount,
                currency,
                period,
            } => {
                if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
                    return Err(Error::Validation(
                        "Use a three-letter uppercase currency code.",
                    ));
                }
                let mut value = json!({"amount":decimal(amount)?,"currency":currency});
                if let Some(period) = period {
                    value["period"] = json!(period);
                }
                value
            }
            Self::Quantity { amount, unit } => {
                bounded(unit, 80)?;
                json!({"amount":decimal(amount)?,"unit":unit})
            }
            Self::Boolean(v) => json!(v),
            Self::Entity(v) => {
                uuid::Uuid::parse_str(v)
                    .map_err(|_| Error::Validation("Invalid entity identity."))?;
                json!(v)
            }
            Self::Json(v) => {
                if v.to_string().len() > 16000 {
                    return Err(Error::Validation("Structured value is too large."));
                }
                v.clone()
            }
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Validity {
    Unknown,
    Timeless,
    Point { date: String },
    Interval { from: String, to: Option<String> },
}
impl Validity {
    fn parts(&self) -> Result<(&'static str, Option<&str>, Option<&str>)> {
        Ok(match self {
            Self::Unknown => ("unknown", None, None),
            Self::Timeless => ("timeless", None, None),
            Self::Point { date: d } => {
                date(d)?;
                ("point", Some(d), None)
            }
            Self::Interval { from, to } => {
                date(from)?;
                if let Some(to) = to {
                    date(to)?;
                    if to <= from {
                        return Err(Error::Validation("The validity end must follow its start."));
                    }
                }
                ("interval", Some(from), to.as_deref())
            }
        })
    }
}
#[derive(Clone, Debug)]
pub struct PropertyDefinition {
    pub key: String,
    pub label: String,
    pub value_type: String,
    pub many: bool,
    /// Allowed subject kinds; empty allows any kind.
    pub subject_kinds: Vec<EntityKind>,
    /// Allowed kinds of an entity value; empty allows any kind.
    pub object_kinds: Vec<EntityKind>,
}
#[derive(Clone, Debug)]
pub struct AssertionDraft {
    pub subject: String,
    pub property: String,
    pub value: FactValue,
    pub validity: Validity,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedFact {
    pub id: String,
    pub subject: String,
    pub property: String,
    pub value: Value,
    pub validity: Validity,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Missing,
    Resolved,
    Multiple,
    Conflict,
    Uncertain,
}
#[derive(Clone, Debug, Serialize)]
pub struct FactResolution {
    pub status: ResolutionStatus,
    pub facts: Vec<ResolvedFact>,
}
/// Neighborhood of an entity for an agent task, resolved at one date.
#[derive(Clone, Debug, Serialize)]
pub struct EntityContext {
    pub root: String,
    pub as_of: String,
    pub entities: Vec<ContextEntity>,
    pub properties: Vec<ContextProperty>,
    /// The entity limit stopped expansion; the context is not complete.
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ContextEntity {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// Number of links from the root entity.
    pub distance: u8,
}
#[derive(Clone, Debug, Serialize)]
pub struct ContextProperty {
    pub subject: String,
    pub property: String,
    pub label: String,
    pub value_type: String,
    pub resolution: FactResolution,
}
#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    pub id: String,
    pub source_id: String,
    pub property_hint: String,
    pub value: Value,
    pub subject_quote: String,
    pub context_quote: String,
    pub verification: String,
}

pub(crate) fn display_value(raw: &str) -> String {
    let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    if let Some(s) = v.as_str() {
        s.to_owned()
    } else if let (Some(amount), Some(currency)) = (v["amount"].as_str(), v["currency"].as_str()) {
        match v["period"].as_str() {
            Some(period) => format!("{amount} {currency} per {}", period.replace('_', " ")),
            None => format!("{amount} {currency}"),
        }
    } else {
        v.to_string()
    }
}
fn list_fact(
    tx: &Transaction<'_>,
    source: &str,
    assertion: &str,
    draft: &AssertionDraft,
) -> Result<()> {
    let label: String = tx.query_row(
        "SELECT label FROM property_definition WHERE key=?",
        [&draft.property],
        |r| r.get(0),
    )?;
    let subject: String = tx.query_row(
        "SELECT label FROM entity WHERE id=?",
        [&draft.subject],
        |r| r.get(0),
    )?;
    tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,property_key,current_assertion_id) VALUES(?,?,'note',?,?,?)",params![id(),format!("{subject} · {label}"),source,draft.property,assertion])?;
    crate::vault::segment(
        tx,
        source,
        0,
        &format!(
            "{subject} {label} {}",
            display_value(&draft.value.normalized()?.to_string())
        ),
    )?;
    Ok(())
}
pub(crate) fn property_rules(p: &PropertyDefinition) -> Value {
    let mut rules = json!({});
    for (field, kinds) in [
        ("subject_kinds", &p.subject_kinds),
        ("object_kinds", &p.object_kinds),
    ] {
        if !kinds.is_empty() {
            let kinds: BTreeSet<_> = kinds.iter().map(EntityKind::key).collect();
            rules[field] = json!(kinds);
        }
    }
    rules
}
pub(crate) fn bounded(s: &str, max: usize) -> Result<()> {
    if s.trim().is_empty() || s.len() > max || s.contains('\0') {
        Err(Error::Validation("A required value is empty or too large."))
    } else {
        Ok(())
    }
}
pub(crate) fn date(s: &str) -> Result<()> {
    if s.len() != 10 || !s.is_ascii() || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return Err(Error::Validation("Use a valid YYYY-MM-DD date."));
    }
    crate::normalize_fact("person.birth_date", s).map(|_| ())
}
fn decimal(s: &str) -> Result<String> {
    if s.len() > 80 {
        return Err(Error::Validation("Invalid exact decimal."));
    }
    let (negative, raw) = s.strip_prefix('-').map_or((false, s), |s| (true, s));
    let parts: Vec<_> = raw.split('.').collect();
    if parts.is_empty()
        || parts.len() > 2
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::Validation(
            "Use an exact decimal without grouping or exponent.",
        ));
    }
    let whole = parts[0].trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = parts.get(1).map_or("", |s| s.trim_end_matches('0'));
    let sign = if negative && (whole != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    Ok(if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    })
}
pub(crate) fn canonical_entity(db: &rusqlite::Connection, entity: &str) -> Result<String> {
    let mut current = entity.to_owned();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current.clone()) || seen.len() > 64 {
            return Err(Error::Validation("Entity merge cycle or chain limit."));
        }
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM entity WHERE id=? AND deleted_at IS NULL)",
            [&current],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::Validation("The entity is unavailable."));
        }
        let next: Option<String> = db
            .query_row(
                "SELECT into_id FROM entity_merge WHERE from_id=? AND reversed_at IS NULL",
                [&current],
                |r| r.get(0),
            )
            .optional()?;
        match next {
            Some(next) => current = next,
            None => return Ok(current),
        }
    }
}
/// Recursive CTE naming ?1 and every entity currently merged into it.
const RELATED: &str = "WITH RECURSIVE related(id) AS (SELECT ?1 UNION SELECT m.from_id FROM entity_merge m JOIN related r ON m.into_id=r.id WHERE m.reversed_at IS NULL)";
pub const MAX_CONTEXT_DEPTH: u8 = 4;
pub const MAX_CONTEXT_ENTITIES: usize = 256;
/// Resolve accepted facts of a canonical entity. Entity values are returned in
/// their current canonical identity; links to unavailable entities are omitted.
pub(crate) fn resolve(
    db: &rusqlite::Connection,
    entity: &str,
    property: &str,
    as_of: &str,
) -> Result<FactResolution> {
    let (many, value_type): (bool, String) = db
        .query_row(
            "SELECT cardinality='many',value_type FROM property_definition WHERE key=?",
            [property],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::Validation("This property is not defined."))?;
    let rows = db.prepare(&format!("{RELATED} SELECT a.id,a.value_json,a.time_kind,a.valid_from,a.valid_to FROM assertion_state a JOIN entity e ON e.id=a.subject_id WHERE a.subject_id IN (SELECT id FROM related) AND a.property_key=?2 AND a.state='accept' AND e.deleted_at IS NULL ORDER BY a.id"))?
        .query_map(params![entity, property], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut facts = Vec::new();
    for (id, raw, kind, from, to) in rows {
        let validity = match kind.as_str() {
            "timeless" => Validity::Timeless,
            "unknown" => Validity::Unknown,
            "point" => {
                let d = from.ok_or(Error::Format)?;
                if d != as_of {
                    continue;
                }
                Validity::Point { date: d }
            }
            "interval" => {
                let from = from.ok_or(Error::Format)?;
                if from.as_str() > as_of || to.as_deref().is_some_and(|to| to <= as_of) {
                    continue;
                }
                Validity::Interval { from, to }
            }
            _ => return Err(Error::Format),
        };
        let mut value: Value = serde_json::from_str(&raw).map_err(|_| Error::Format)?;
        if value_type == "entity" {
            let object = value.as_str().ok_or(Error::Format)?;
            // A merged object is followed to its current identity; a deleted one is gone.
            let Ok(object) = canonical_entity(db, object) else {
                continue;
            };
            value = json!(object);
        }
        facts.push(ResolvedFact {
            id,
            subject: entity.to_owned(),
            property: property.into(),
            value,
            validity,
        });
    }
    let status = resolution_status(&facts, many);
    Ok(FactResolution { status, facts })
}
pub(crate) fn resolution_status(facts: &[ResolvedFact], many: bool) -> ResolutionStatus {
    let values: BTreeSet<_> = facts.iter().map(|f| f.value.to_string()).collect();
    if facts.is_empty() {
        ResolutionStatus::Missing
    } else if values.len() > 1 && !many {
        ResolutionStatus::Conflict
    } else if facts.iter().any(|f| f.validity == Validity::Unknown) {
        ResolutionStatus::Uncertain
    } else if values.len() > 1 {
        ResolutionStatus::Multiple
    } else {
        ResolutionStatus::Resolved
    }
}
pub(crate) fn insert_assertion(
    tx: &Transaction<'_>,
    draft: &AssertionDraft,
    origin: &str,
    run: Option<&str>,
) -> Result<String> {
    insert_assertion_superseding(tx, draft, origin, run, None)
}
/// As `insert_assertion`, recording which assertion the new one replaces.
pub(crate) fn insert_assertion_superseding(
    tx: &Transaction<'_>,
    draft: &AssertionDraft,
    origin: &str,
    run: Option<&str>,
    supersedes: Option<&str>,
) -> Result<String> {
    let subject = canonical_entity(tx, &draft.subject)?;
    let mut normalized = draft.value.normalized()?;
    if crate::STANDARD_FIELDS
        .iter()
        .any(|field| field.key == draft.property)
    {
        let value = normalized
            .as_str()
            .ok_or(Error::Validation("Invalid standard field value."))?;
        normalized = json!(crate::normalize_fact(&draft.property, value)?);
    }
    let object = if let FactValue::Entity(e) = &draft.value {
        let e = canonical_entity(tx, e)?;
        normalized = json!(e);
        Some(e)
    } else {
        None
    };
    let (expected, rules): (String, String) = tx
        .query_row(
            "SELECT value_type,rules_json FROM property_definition WHERE key=?",
            [&draft.property],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::Validation("Define this property before using it."))?;
    if expected != draft.value.value_type() {
        return Err(Error::Validation(
            "The value type does not match the property.",
        ));
    }
    let rules: Value = serde_json::from_str(&rules).map_err(|_| Error::Format)?;
    let allowed = |field: &str, entity: &str| -> Result<bool> {
        let Some(kinds) = rules[field].as_array() else {
            return Ok(true);
        };
        let kind: String =
            tx.query_row("SELECT kind FROM entity WHERE id=?", [entity], |r| r.get(0))?;
        Ok(kinds.iter().any(|k| k.as_str() == Some(&kind)))
    };
    if !allowed("subject_kinds", &subject)? {
        return Err(Error::Validation(
            "This property does not apply to this kind of entity.",
        ));
    }
    if let Some(object) = &object
        && !allowed("object_kinds", object)?
    {
        return Err(Error::Validation(
            "This property cannot link to this kind of entity.",
        ));
    }
    let (kind, from, to) = draft.validity.parts()?;
    let raw = normalized.to_string();
    let semantic = hash(
        json!([subject, draft.property, normalized, kind, from, to])
            .to_string()
            .as_bytes(),
    );
    if let Some(existing) = tx
        .query_row(
            "SELECT id FROM assertion WHERE semantic_key=?",
            [&semantic],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let assertion = id();
    tx.execute("INSERT INTO assertion(id,subject_id,property_key,value_type,value_json,object_entity_id,canonical_value,semantic_key,time_kind,valid_from,valid_to,recorded_at,origin,extraction_run_id,supersedes_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,?,?)",params![assertion,subject,draft.property,draft.value.value_type(),raw,object,raw,semantic,kind,from,to,origin,run,supersedes])?;
    Ok(assertion)
}
pub(crate) fn store_observation(
    tx: &Transaction<'_>,
    source: &str,
    run: &str,
    fact: &crate::ExtractedFact,
    supported: bool,
) -> Result<()> {
    let candidate = crate::review_questions::candidate_key(fact);
    let locator: Option<String> = tx
        .query_row(
            "SELECT locator_json FROM source_segment WHERE source_id=? AND id=?",
            params![source, fact.segment_id],
            |r| r.get(0),
        )
        .optional()?;
    let locator = json!({"source_locator":locator.and_then(|s|serde_json::from_str::<Value>(&s).ok()),if supported {"quote"} else {"claimed_quote"}:fact.quote});
    tx.execute("INSERT OR IGNORE INTO observation VALUES(?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),source,run,candidate,fact.property,json!(fact.value).to_string(),fact.subject_quote,fact.context_quote,locator.to_string(),if supported{"supported"}else{"unverified"}])?;
    Ok(())
}

impl Vault {
    pub(crate) fn assertion_periods_overlap(&self, first: &str, second: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT CASE WHEN a.time_kind IN ('unknown','timeless') OR b.time_kind IN ('unknown','timeless') THEN 1 WHEN a.time_kind='point' AND b.time_kind='point' THEN a.valid_from=b.valid_from WHEN a.time_kind='point' THEN a.valid_from>=b.valid_from AND (b.valid_to IS NULL OR a.valid_from<b.valid_to) WHEN b.time_kind='point' THEN b.valid_from>=a.valid_from AND (a.valid_to IS NULL OR b.valid_from<a.valid_to) ELSE (a.valid_to IS NULL OR b.valid_from<a.valid_to) AND (b.valid_to IS NULL OR a.valid_from<b.valid_to) END FROM assertion a, assertion b WHERE a.id=? AND b.id=?",params![first,second],|r|r.get(0))?)
    }
    pub(crate) fn assertion_validity_label(&self, assertion: &str) -> Result<String> {
        let (kind, from, to): (String, Option<String>, Option<String>) = self.db.query_row(
            "SELECT time_kind,valid_from,valid_to FROM assertion WHERE id=?",
            [assertion],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(match kind.as_str() {
            "interval" => format!(
                "Valid from {}{}",
                from.unwrap_or_default(),
                to.map(|s| format!(" until {s} (exclusive)"))
                    .unwrap_or_default()
            ),
            "point" => format!("On {}", from.unwrap_or_default()),
            _ => String::new(),
        })
    }
    pub fn restore_entity(&mut self, entity: &str) -> Result<()> {
        if self.db.execute(
            "UPDATE entity SET deleted_at=NULL WHERE id=? AND deleted_at IS NOT NULL",
            [entity],
        )? != 1
        {
            return Err(Error::Validation("This entity is not deleted."));
        }
        Ok(())
    }
    pub fn profile_entity_id(&self) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT profile_id FROM vault_meta", [], |r| r.get(0))?)
    }
    pub fn create_entity(&mut self, kind: EntityKind, label: &str) -> Result<String> {
        bounded(label, 200)?;
        let entity = id();
        self.db.execute("INSERT INTO entity(id,kind,label,created_at) VALUES(?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![entity,kind.key(),label.trim()])?;
        Ok(entity)
    }
    pub fn entities(&self) -> Result<Vec<PersonalEntity>> {
        Ok(self
            .db
            .prepare("SELECT id,kind,label,deleted_at IS NOT NULL FROM entity ORDER BY label,id")?
            .query_map([], |r| {
                Ok(PersonalEntity {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    label: r.get(2)?,
                    deleted: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn add_entity_identifier(
        &mut self,
        entity: &str,
        namespace: &str,
        value: &str,
    ) -> Result<()> {
        bounded(namespace, 120)?;
        bounded(value, 400)?;
        let e = canonical_entity(&self.db, entity)?;
        self.db.execute(
            "INSERT OR IGNORE INTO entity_identifier VALUES(?,?,?,?)",
            params![id(), e, namespace, value],
        )?;
        Ok(())
    }
    pub fn matching_entities(&self, namespace: &str, value: &str) -> Result<Vec<String>> {
        bounded(namespace, 120)?;
        bounded(value, 400)?;
        let ids = self
            .db
            .prepare("SELECT entity_id FROM entity_identifier WHERE namespace=? AND value=?")?
            .query_map(params![namespace, value], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = BTreeSet::new();
        for entity in ids {
            out.insert(canonical_entity(&self.db, &entity)?);
        }
        Ok(out.into_iter().collect())
    }
    pub fn merge_entities(&mut self, from: &str, into: &str) -> Result<String> {
        let from = canonical_entity(&self.db, from)?;
        let into = canonical_entity(&self.db, into)?;
        if from == into || from == self.profile_entity_id()? {
            return Err(Error::Validation("Cannot merge this entity."));
        }
        let a: String = self
            .db
            .query_row("SELECT kind FROM entity WHERE id=?", [&from], |r| r.get(0))?;
        let b: String = self
            .db
            .query_row("SELECT kind FROM entity WHERE id=?", [&into], |r| r.get(0))?;
        if a != b {
            return Err(Error::Validation(
                "Only entities of the same kind can be merged.",
            ));
        }
        let merge = id();
        self.db.execute("INSERT INTO entity_merge(id,from_id,into_id,actor_id,recorded_at) VALUES(?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![merge,from,into,self.profile_entity_id()?])?;
        Ok(merge)
    }
    pub fn reverse_entity_merge(&mut self, merge: &str) -> Result<()> {
        if self.db.execute("UPDATE entity_merge SET reversed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND reversed_at IS NULL",[merge])?!=1{return Err(Error::Validation("This merge is not active."));}
        Ok(())
    }
    pub fn delete_entity(&mut self, entity: &str) -> Result<()> {
        if entity == self.profile_entity_id()? {
            return Err(Error::Validation("The self profile cannot be deleted."));
        }
        let used:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM entity_merge WHERE reversed_at IS NULL AND (from_id=?1 OR into_id=?1))",[entity],|r|r.get(0))?;
        if used {
            return Err(Error::Validation(
                "Reverse this entity's merges before deleting it.",
            ));
        }
        if self.db.execute("UPDATE entity SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=? AND deleted_at IS NULL",[entity])?!=1{return Err(Error::Validation("The entity is unavailable."));}
        Ok(())
    }
    pub fn define_property(&mut self, p: &PropertyDefinition) -> Result<()> {
        bounded(&p.key, 180)?;
        bounded(&p.label, 200)?;
        if !p
            .key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._".contains(&b))
            || !p.key.contains('.')
            || ![
                "text",
                "identifier",
                "date",
                "integer",
                "decimal",
                "money",
                "quantity",
                "boolean",
                "entity",
                "json",
            ]
            .contains(&p.value_type.as_str())
        {
            return Err(Error::Validation("Invalid property definition."));
        }
        if !p.object_kinds.is_empty() && p.value_type != "entity" {
            return Err(Error::Validation(
                "Only entity properties can restrict linked kinds.",
            ));
        }
        let cardinality = if p.many { "many" } else { "one" };
        let old: Option<(String, String, String)> = self
            .db
            .query_row(
                "SELECT value_type,cardinality,rules_json FROM property_definition WHERE key=?",
                [&p.key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let kinds = |k: &[EntityKind]| k.iter().map(EntityKind::key).collect::<BTreeSet<_>>();
        if let Some((t, c, rules)) = old {
            // Omitted kinds leave an existing definition's constraints as they are.
            let rules: Value = serde_json::from_str(&rules).map_err(|_| Error::Format)?;
            let stored = |field: &str| -> BTreeSet<String> {
                rules[field]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|k| k.as_str().map(str::to_owned))
                    .collect()
            };
            let changed = |requested: &[EntityKind], field: &str| {
                !requested.is_empty() && kinds(requested) != stored(field)
            };
            if t != p.value_type
                || c != cardinality
                || changed(&p.subject_kinds, "subject_kinds")
                || changed(&p.object_kinds, "object_kinds")
            {
                return Err(Error::Validation(
                    "An existing property's meaning cannot be replaced.",
                ));
            }
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO property_definition VALUES(?,?,?,?,'confirm',?,1)",
            params![
                p.key,
                p.label,
                p.value_type,
                cardinality,
                property_rules(p).to_string()
            ],
        )?;
        Ok(())
    }
    /// User-authored evidence, kept distinct from extracted quotations.
    pub fn record_fact(&mut self, draft: &AssertionDraft) -> Result<String> {
        let tx = self.db.transaction()?;
        let assertion = insert_assertion(&tx, draft, "user", None)?;
        let source = id();
        let raw = draft.value.normalized()?.to_string();
        tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'note',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'personal','keep')",params![source,draft.property,hash(raw.as_bytes())])?;
        tx.execute(
            "INSERT INTO assertion_evidence VALUES(?,?,?,'{\"kind\":\"manual\"}')",
            params![assertion, source, id()],
        )?;
        crate::vault::decision(&tx, &assertion, "accept")?;
        list_fact(&tx, &source, &assertion, draft)?;
        tx.commit()?;
        Ok(assertion)
    }
    pub fn decide_fact(&mut self, assertion: &str, accept: bool) -> Result<()> {
        let tx = self.db.transaction()?;
        crate::vault::decision(&tx, assertion, if accept { "accept" } else { "retract" })?;
        tx.commit()?;
        Ok(())
    }
    pub fn observations(&self, source: &str) -> Result<Vec<Observation>> {
        Ok(self.db.prepare("SELECT id,source_id,property_hint,value_json,subject_quote,context_quote,verification FROM observation WHERE source_id=? ORDER BY recorded_at,id")?.query_map([source],|r|{let raw:String=r.get(3)?;Ok(Observation{id:r.get(0)?,source_id:r.get(1)?,property_hint:r.get(2)?,value:serde_json::from_str(&raw).unwrap_or(Value::Null),subject_quote:r.get(4)?,context_quote:r.get(5)?,verification:r.get(6)?})})?.collect::<std::result::Result<_,_>>()?)
    }
    /// Explicit reviewed mapping. The observation remains unchanged and auditable.
    pub fn resolve_observation(
        &mut self,
        observation: &str,
        draft: &AssertionDraft,
    ) -> Result<String> {
        let actor = self.profile_entity_id()?;
        let tx = self.db.transaction()?;
        let document:String=tx.query_row("SELECT o.source_id FROM observation o JOIN source s ON s.id=o.source_id WHERE o.id=? AND s.sensitivity='personal' AND s.retention='keep'",[observation],|r|r.get(0)).optional()?.ok_or(Error::Validation("This observation is unavailable."))?;
        // The user confirms semantic mapping, type and ownership. The original observation
        // is context, never fabricated verbatim evidence for a corrected/normalized value.
        let assertion = insert_assertion(&tx, draft, "user", None)?;
        let mapped:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM observation_resolution WHERE observation_id=? AND assertion_id=?)",params![observation,assertion],|r|r.get(0))?;
        if mapped {
            tx.commit()?;
            return Ok(assertion);
        }

        let source = id();
        let raw = draft.value.normalized()?.to_string();
        tx.execute("INSERT INTO source(id,kind,title,received_at,content_fingerprint,sensitivity,retention) VALUES(?,'note',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?,'personal','keep')",params![source,draft.property,hash(raw.as_bytes())])?;
        let locator=json!({"kind":"reviewed_mapping","observation_id":observation,"context_source_id":document}).to_string();
        tx.execute(
            "INSERT INTO assertion_evidence VALUES(?,?,?,?)",
            params![assertion, source, id(), locator],
        )?;
        tx.execute("INSERT OR IGNORE INTO observation_resolution VALUES(?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),observation,assertion,actor])?;
        crate::vault::decision(&tx, &assertion, "accept")?;
        list_fact(&tx, &source, &assertion, draft)?;
        tx.commit()?;
        Ok(assertion)
    }
    pub fn resolve_fact_at(
        &self,
        entity: &str,
        property: &str,
        as_of: &str,
    ) -> Result<FactResolution> {
        date(as_of)?;
        let entity = canonical_entity(&self.db, entity)?;
        resolve(&self.db, &entity, property, as_of)
    }
    /// Trusted read of the accepted graph around `root` as of a date: facts of
    /// every reached entity, following entity-valued facts in both directions.
    /// Entities at the depth or size limit are included without expanding them.
    pub fn entity_context_at(&self, root: &str, as_of: &str, depth: u8) -> Result<EntityContext> {
        date(as_of)?;
        if depth > MAX_CONTEXT_DEPTH {
            return Err(Error::Validation("Use a smaller context depth."));
        }
        let root = canonical_entity(&self.db, root)?;
        let mut context = EntityContext {
            root: root.clone(),
            as_of: as_of.into(),
            entities: Vec::new(),
            properties: Vec::new(),
            truncated: false,
        };
        let mut seen = BTreeSet::from([root.clone()]);
        let mut queue = std::collections::VecDeque::from([(root, 0u8)]);
        while let Some((entity, distance)) = queue.pop_front() {
            let (kind, label) =
                self.db
                    .query_row("SELECT kind,label FROM entity WHERE id=?", [&entity], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?;
            context.entities.push(ContextEntity {
                id: entity.clone(),
                kind,
                label,
                distance,
            });
            let mut neighbors = BTreeSet::new();
            let properties = self.db.prepare(&format!("{RELATED} SELECT DISTINCT a.property_key,p.label,p.value_type FROM assertion_state a JOIN property_definition p ON p.key=a.property_key WHERE a.subject_id IN (SELECT id FROM related) AND a.state='accept' ORDER BY a.property_key"))?.query_map([&entity], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<Vec<(String, String, String)>, _>>()?;
            for (property, label, value_type) in properties {
                let resolution = resolve(&self.db, &entity, &property, as_of)?;
                if resolution.status == ResolutionStatus::Missing {
                    continue;
                }
                if value_type == "entity" {
                    neighbors.extend(
                        resolution
                            .facts
                            .iter()
                            .filter_map(|f| f.value.as_str().map(str::to_owned)),
                    );
                }
                context.properties.push(ContextProperty {
                    subject: entity.clone(),
                    property,
                    label,
                    value_type,
                    resolution,
                });
            }
            // Incoming edges, e.g. the contract that insures a vehicle.
            let incoming = self.db.prepare(&format!("{RELATED} SELECT a.subject_id FROM assertion_state a WHERE a.object_entity_id IN (SELECT id FROM related) AND a.state='accept' AND (a.time_kind IN ('timeless','unknown') OR (a.time_kind='point' AND a.valid_from=?2) OR (a.time_kind='interval' AND a.valid_from<=?2 AND (a.valid_to IS NULL OR a.valid_to>?2)))"))?.query_map(params![entity, as_of], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            for subject in incoming {
                // Unavailable (deleted) subjects are not part of the current graph.
                if let Ok(subject) = canonical_entity(&self.db, &subject) {
                    neighbors.insert(subject);
                }
            }
            if distance == depth {
                continue;
            }
            for next in neighbors {
                if seen.contains(&next) {
                    continue;
                }
                if seen.len() >= MAX_CONTEXT_ENTITIES {
                    context.truncated = true;
                    break;
                }
                seen.insert(next.clone());
                queue.push_back((next, distance + 1));
            }
        }
        Ok(context)
    }
}
