//! Resolve stage: turn a classified document and its selected slot values into
//! entities, links and automatically accepted assertions. Everything written here
//! is marked as a policy decision with confidence, so review state stays visible.
//! Resolution is idempotent and independent of import order.
use crate::{
    AssertionDraft, Candidate, CandidateValue, EntityKind, Error, FactValue, Period, Result,
    Validity, Vault,
    doc_types::{self, DocType, SlotValidity, Target, ValueKind},
    domain::{canonical_entity, insert_assertion, insert_assertion_superseding},
    vault::{hash, id, journal},
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Recorded on every automatic decision. Changing resolution rules changes it.
pub const GRAPH_POLICY: &str = "import-graph-v1";
/// Distinct documents naming the same non-anchor person before proposing a member.
pub const HOUSEHOLD_PROPOSAL_MENTIONS: usize = 3;
/// Starting threshold below which an automatic value is worth a quick check.
pub const DEFAULT_CHECK_THRESHOLD: f64 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceSource {
    Checksum,
    Typesafe,
    OpenaiGrounded,
    User,
}
impl ConfidenceSource {
    fn key(self) -> &'static str {
        match self {
            Self::Checksum => "checksum",
            Self::Typesafe => "typesafe",
            Self::OpenaiGrounded => "openai_grounded",
            Self::User => "user",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlotContent {
    /// A verbatim local candidate.
    Candidate(Box<Candidate>),
    /// A closed-set category judged from the document.
    Category { key: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotValue {
    pub slot: String,
    pub content: SlotContent,
    /// Recurrence of a money value when the type leaves it open.
    #[serde(default)]
    pub period: Option<Period>,
    pub confidence: f64,
    pub source: ConfidenceSource,
    /// The value passed a checksum; its meaning was judged separately.
    #[serde(default)]
    pub value_checked: bool,
    /// Accepted, but worth a quick look.
    #[serde(default)]
    pub check: bool,
}

/// Output of Classify and Extract for one document; input of Resolve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentGraph {
    pub doc_type: String,
    /// Canonical anchor entity the document is about, if any.
    pub subject: Option<String>,
    pub subject_confidence: f64,
    pub values: Vec<SlotValue>,
    /// Model identifiers that produced the decisions, for audit.
    pub models: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GraphOutcome {
    pub entities_created: usize,
    pub assertions: usize,
    pub checks: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Unreviewed,
    CheckSuggested,
    Reviewed,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuickCheck {
    pub assertion: String,
    pub property: String,
    pub label: String,
    pub value: String,
    pub reason: String,
    pub source_title: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct QuickCheckGroup {
    pub entity: String,
    pub entity_label: String,
    pub entity_kind: String,
    pub items: Vec<QuickCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HouseholdMember {
    pub entity: String,
    pub name: String,
    pub role: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityAnchors {
    pub self_entity: String,
    pub self_name: String,
    pub aliases: Vec<String>,
    pub birth_date: Option<String>,
    pub household: Vec<HouseholdMember>,
    pub setup_complete: bool,
}
impl IdentityAnchors {
    /// Every name and alias with the entity it identifies.
    pub fn names(&self) -> Vec<(String, String)> {
        let mut out = vec![(self.self_entity.clone(), self.self_name.clone())];
        out.extend(
            self.aliases
                .iter()
                .map(|a| (self.self_entity.clone(), a.clone())),
        );
        out.extend(
            self.household
                .iter()
                .map(|m| (m.entity.clone(), m.name.clone())),
        );
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HouseholdProposal {
    pub name: String,
    pub documents: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MergeCandidate {
    pub a: String,
    pub a_label: String,
    pub b: String,
    pub b_label: String,
    pub kind: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ValueConflict {
    pub first: String,
    pub first_value: String,
    pub second: String,
    pub second_value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConstellationNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub links: usize,
    /// Something about this entity is worth a quick look.
    pub check: bool,
    /// The user or a household member.
    pub anchor: bool,
}
/// The growing graph at a glance: the most connected entities, their links, the
/// document families that stay collapsed, and the latest automatic discovery.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Constellation {
    pub nodes: Vec<ConstellationNode>,
    pub edges: Vec<(String, String)>,
    pub clusters: Vec<(String, usize)>,
    pub latest: Option<String>,
}

/// Lowercased, diacritic-folded and punctuation-free name for comparisons.
pub fn normalize_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars().flat_map(char::to_lowercase) {
        match ch {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// Organization identity key: normalized name without common legal-form suffixes.
pub fn normalize_organization(name: &str) -> String {
    const FORMS: &[&str] = &[
        "gmbh", "mbh", "ag", "se", "kg", "kgaa", "ohg", "ev", "e v", "co", "und", "inc", "ltd",
        "llc", "plc", "sa", "bv", "nv",
    ];
    let words: Vec<String> = normalize_name(name)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let mut end = words.len();
    while end > 1 && FORMS.contains(&words[end - 1].as_str()) {
        end -= 1;
    }
    words[..end].join(" ")
}
fn normalize_identifier(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
/// The day after an ISO date; `None` for malformed input.
pub fn next_day(date: &str) -> Option<String> {
    let mut parts = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) + 1);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

struct Resolved<'a> {
    kind: &'static DocType,
    values: BTreeMap<&'a str, &'a SlotValue>,
}
impl Resolved<'_> {
    fn date(&self, slot: &str) -> Option<String> {
        match &self.values.get(slot)?.content {
            SlotContent::Candidate(c) => match &c.value {
                CandidateValue::Date(d) => Some(d.clone()),
                _ => None,
            },
            SlotContent::Category { .. } => None,
        }
    }
    fn text(&self, slot: &str) -> Option<String> {
        self.values.get(slot).map(|v| display_content(&v.content))
    }
    fn validity(&self, rule: SlotValidity) -> Validity {
        match rule {
            SlotValidity::Timeless => Validity::Timeless,
            SlotValidity::DocumentPeriod => {
                match (self.date("period_start"), self.date("period_end")) {
                    (Some(from), Some(end)) if end >= from => Validity::Interval {
                        to: next_day(&end),
                        from,
                    },
                    _ => Validity::Unknown,
                }
            }
            SlotValidity::From(slot) => {
                match self.date(slot).or_else(|| self.date("document_date")) {
                    Some(from) => Validity::Interval { from, to: None },
                    None => Validity::Unknown,
                }
            }
        }
    }
}
fn display_content(content: &SlotContent) -> String {
    match content {
        SlotContent::Category { key } => key.clone(),
        SlotContent::Candidate(c) => match &c.value {
            CandidateValue::Text(s) | CandidateValue::Identifier(s) | CandidateValue::Date(s) => {
                s.clone()
            }
            CandidateValue::Money { amount, currency } => format!("{amount} {currency}"),
            CandidateValue::Mrz(m) => m.document_number.clone(),
        },
    }
}

fn fact_value(value_type: &str, v: &SlotValue, fixed: Option<Period>) -> Option<FactValue> {
    let text = display_content(&v.content);
    Some(match (value_type, &v.content) {
        ("date", SlotContent::Candidate(c)) => match &c.value {
            CandidateValue::Date(d) => FactValue::Date(d.clone()),
            _ => return None,
        },
        ("money", SlotContent::Candidate(c)) => match &c.value {
            CandidateValue::Money { amount, currency } => FactValue::Money {
                amount: amount.clone(),
                currency: currency.clone(),
                period: fixed.or(v.period),
            },
            _ => return None,
        },
        ("identifier", _) => {
            let compact = normalize_identifier(&text);
            if compact.is_empty() {
                return None;
            }
            FactValue::Identifier(compact)
        }
        ("text", _) => FactValue::Text(text.trim().to_owned()),
        _ => return None,
    })
}

fn locator(c: &Candidate) -> Value {
    json!({"kind":"candidate","segment_id":c.segment_id,"start":c.start,"end":c.end,"quote":c.text,"line":c.line})
}

/// Latest decision on an assertion: (action, actor).
fn latest_decision(tx: &Transaction<'_>, assertion: &str) -> Result<Option<(String, String)>> {
    Ok(tx
        .query_row(
            "SELECT action,actor FROM decision WHERE assertion_id=? ORDER BY local_seq DESC LIMIT 1",
            [assertion],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}
fn policy_decision(
    tx: &Transaction<'_>,
    assertion: &str,
    action: &str,
    reason: &str,
) -> Result<()> {
    tx.execute("INSERT INTO decision(id,assertion_id,action,actor,policy_version,reason_code,recorded_at) VALUES(?,?,?,'policy',?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),assertion,action,GRAPH_POLICY,reason])?;
    Ok(())
}
/// Accepts an assertion by policy unless a person already decided on it.
fn auto_accept(tx: &Transaction<'_>, assertion: &str) -> Result<bool> {
    match latest_decision(tx, assertion)? {
        Some((_, actor)) if actor == "user" => Ok(false),
        Some((action, _)) if action == "accept" => Ok(true),
        _ => {
            policy_decision(tx, assertion, "accept", "import_auto_accept")?;
            Ok(true)
        }
    }
}
fn upsert_review(
    tx: &Transaction<'_>,
    assertion: &str,
    confidence: f64,
    source: ConfidenceSource,
    value_checked: bool,
    check: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    let confidence = confidence.clamp(0., 1.);
    let existing: Option<(f64, Option<String>)> = tx
        .query_row(
            "SELECT confidence,check_reason FROM assertion_review WHERE assertion_id=?",
            [assertion],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            tx.execute("INSERT INTO assertion_review(assertion_id,confidence,confidence_source,value_checked,check_reason,model,recorded_at) VALUES(?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![assertion,confidence,source.key(),value_checked,check,model])?;
        }
        // Stronger corroborating evidence raises confidence; a conflict flag stays.
        Some((old, reason)) if confidence > old => {
            let reason = match reason.as_deref() {
                Some("conflict") => Some("conflict"),
                _ => check,
            };
            tx.execute("UPDATE assertion_review SET confidence=?,confidence_source=?,value_checked=max(value_checked,?),check_reason=?,model=? WHERE assertion_id=?",params![confidence,source.key(),value_checked,reason,model,assertion])?;
        }
        Some(_) => {}
    }
    Ok(())
}
fn entity_kind(tx: &Transaction<'_>, entity: &str) -> Result<String> {
    Ok(tx.query_row("SELECT kind FROM entity WHERE id=?", [entity], |r| r.get(0))?)
}
fn find_by_identifier(
    tx: &Transaction<'_>,
    namespace: &str,
    value: &str,
    kind: EntityKind,
) -> Result<Option<String>> {
    let ids = tx
        .prepare(
            "SELECT entity_id FROM entity_identifier WHERE namespace=? AND value=? ORDER BY id",
        )?
        .query_map(params![namespace, value], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut found = BTreeSet::new();
    for entity in ids {
        if let Ok(e) = canonical_entity(tx, &entity)
            && entity_kind(tx, &e)? == kind.key()
        {
            found.insert(e);
        }
    }
    Ok(found.into_iter().next())
}
fn add_identifier(tx: &Transaction<'_>, entity: &str, namespace: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO entity_identifier VALUES(?,?,?,?)",
        params![id(), entity, namespace, value],
    )?;
    Ok(())
}
fn create_entity(tx: &Transaction<'_>, kind: EntityKind, label: &str) -> Result<String> {
    let entity = id();
    let label: String = label.trim().chars().take(200).collect();
    let label = if label.is_empty() {
        "Unnamed".into()
    } else {
        label
    };
    tx.execute("INSERT INTO entity(id,kind,label,created_at) VALUES(?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![entity,kind.key(),label])?;
    Ok(entity)
}
fn list_assertion(
    tx: &Transaction<'_>,
    source: &str,
    assertion: &str,
    property: &str,
) -> Result<()> {
    let listed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM collection_item WHERE current_assertion_id=? AND deleted_at IS NULL)",
        [assertion],
        |r| r.get(0),
    )?;
    if listed {
        return Ok(());
    }
    let (subject, label): (String, String) = tx.query_row(
        "SELECT e.label,p.label FROM assertion a JOIN entity e ON e.id=a.subject_id JOIN property_definition p ON p.key=a.property_key WHERE a.id=?",
        [assertion],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let profile: String = tx.query_row("SELECT profile_id FROM vault_meta", [], |r| r.get(0))?;
    let subject_id: String = tx.query_row(
        "SELECT subject_id FROM assertion WHERE id=?",
        [assertion],
        |r| r.get(0),
    )?;
    let title = if subject_id == profile {
        label
    } else {
        format!("{subject} · {label}")
    };
    let stable = id();
    tx.execute("INSERT INTO collection_item(stable_id,title,kind,source_id,property_key,current_assertion_id) VALUES(?,?,'note',?,?,?)",params![stable,title,source,property,assertion])?;
    journal(
        tx,
        &stable,
        1,
        "import_graph_accepted",
        json!({"assertion_id":assertion,"source_id":source}),
    )?;
    Ok(())
}

/// Closes open-ended automatic intervals at the next start of a different value,
/// so a newer document supersedes an older one regardless of import order.
fn retimeline(tx: &Transaction<'_>, subject: &str, property: &str) -> Result<()> {
    let one: bool = tx.query_row(
        "SELECT cardinality='one' FROM property_definition WHERE key=?",
        [property],
        |r| r.get(0),
    )?;
    if !one {
        return Ok(());
    }
    let rows = tx
        .prepare("SELECT a.id,a.canonical_value,a.valid_from,a.valid_to FROM assertion_state a WHERE a.subject_id=? AND a.property_key=? AND a.state='accept' AND a.time_kind='interval' AND NOT EXISTS(SELECT 1 FROM decision d WHERE d.assertion_id=a.id AND d.actor='user') ORDER BY a.valid_from,a.id")?
        .query_map(params![subject, property], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (assertion, value, from, to) in &rows {
        if to.is_some() {
            continue;
        }
        let Some(next) = rows
            .iter()
            .filter(|(_, v, f, _)| f > from && v != value)
            .map(|(_, _, f, _)| f.clone())
            .min()
        else {
            continue;
        };
        let (value_type, raw): (String, String) = tx.query_row(
            "SELECT value_type,value_json FROM assertion WHERE id=?",
            [assertion],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let stored: Value = serde_json::from_str(&raw).map_err(|_| Error::Format)?;
        let value = stored_fact(&value_type, &stored)?;
        let draft = AssertionDraft {
            subject: subject.into(),
            property: property.into(),
            value,
            validity: Validity::Interval {
                from: from.clone(),
                to: Some(next),
            },
        };
        let run: Option<String> = tx.query_row(
            "SELECT extraction_run_id FROM assertion WHERE id=?",
            [assertion],
            |r| r.get(0),
        )?;
        let closed = insert_assertion_superseding(
            tx,
            &draft,
            "extraction",
            run.as_deref(),
            Some(assertion),
        )?;
        if &closed == assertion {
            continue;
        }
        tx.execute("INSERT OR IGNORE INTO assertion_evidence SELECT ?,source_id,evidence_key,locator_json FROM assertion_evidence WHERE assertion_id=?",params![closed,assertion])?;
        tx.execute("INSERT OR IGNORE INTO assertion_review(assertion_id,confidence,confidence_source,value_checked,check_reason,model,recorded_at) SELECT ?,confidence,confidence_source,value_checked,check_reason,model,recorded_at FROM assertion_review WHERE assertion_id=?",params![closed,assertion])?;
        policy_decision(tx, assertion, "retract", "superseded_by_newer_document")?;
        auto_accept(tx, &closed)?;
        tx.execute(
            "UPDATE collection_item SET current_assertion_id=? WHERE current_assertion_id=?",
            params![closed, assertion],
        )?;
    }
    Ok(())
}
fn stored_fact(value_type: &str, v: &Value) -> Result<FactValue> {
    let text = || v.as_str().map(str::to_owned).ok_or(Error::Format);
    Ok(match value_type {
        "text" => FactValue::Text(text()?),
        "identifier" => FactValue::Identifier(text()?),
        "date" => FactValue::Date(text()?),
        "decimal" => FactValue::Decimal(text()?),
        "entity" => FactValue::Entity(text()?),
        "boolean" => FactValue::Boolean(v.as_bool().ok_or(Error::Format)?),
        "integer" => FactValue::Integer(v.as_i64().ok_or(Error::Format)?),
        "money" => FactValue::Money {
            amount: v["amount"].as_str().ok_or(Error::Format)?.into(),
            currency: v["currency"].as_str().ok_or(Error::Format)?.into(),
            period: serde_json::from_value(v["period"].clone()).ok(),
        },
        "quantity" => FactValue::Quantity {
            amount: v["amount"].as_str().ok_or(Error::Format)?.into(),
            unit: v["unit"].as_str().ok_or(Error::Format)?.into(),
        },
        _ => FactValue::Json(v.clone()),
    })
}
/// Whether two validities overlap: ISO dates compare as text; unknown and
/// timeless validities overlap everything.
fn overlap(
    a: &(String, Option<String>, Option<String>),
    b: &(String, Option<String>, Option<String>),
) -> bool {
    let within = |date: &Option<String>, from: &Option<String>, to: &Option<String>| {
        date >= from
            && to
                .as_ref()
                .is_none_or(|t| date.as_ref().is_some_and(|d| d < t))
    };
    match (a.0.as_str(), b.0.as_str()) {
        ("unknown" | "timeless", _) | (_, "unknown" | "timeless") => true,
        ("point", "point") => a.1 == b.1,
        ("point", _) => within(&a.1, &b.1, &b.2),
        (_, "point") => within(&b.1, &a.1, &a.2),
        _ => {
            a.2.as_ref()
                .is_none_or(|t| b.1.as_ref().is_some_and(|f| f < t))
                && b.2
                    .as_ref()
                    .is_none_or(|t| a.1.as_ref().is_some_and(|f| f < t))
        }
    }
}
/// Flags overlapping incompatible values of a single-valued property. Only pairs
/// involving `focus` are compared (all pairs when `focus` is empty): linear per
/// document instead of quadratic across a large history.
fn mark_conflicts(
    tx: &Transaction<'_>,
    subject: &str,
    property: &str,
    focus: &BTreeSet<String>,
) -> Result<usize> {
    let one: bool = tx.query_row(
        "SELECT cardinality='one' FROM property_definition WHERE key=?",
        [property],
        |r| r.get(0),
    )?;
    if !one {
        return Ok(0);
    }
    type Row = (String, String, (String, Option<String>, Option<String>));
    let rows: Vec<Row> = tx
        .prepare("SELECT id,canonical_value,time_kind,valid_from,valid_to FROM assertion_state WHERE subject_id=? AND property_key=? AND state='accept' ORDER BY id")?
        .query_map(params![subject, property], |r| {
            Ok((r.get(0)?, r.get(1)?, (r.get(2)?, r.get(3)?, r.get(4)?)))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut flagged = BTreeSet::new();
    for (i, (a, va, ta)) in rows.iter().enumerate() {
        if !focus.is_empty() && !focus.contains(a) {
            continue;
        }
        let others = if focus.is_empty() {
            &rows[i + 1..]
        } else {
            &rows[..]
        };
        for (b, vb, tb) in others {
            if a != b && va != vb && overlap(ta, tb) {
                flagged.insert(a.clone());
                flagged.insert(b.clone());
            }
        }
    }
    let mut count = 0;
    for assertion in flagged {
        let user: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM decision WHERE assertion_id=? AND actor='user')",
            [&assertion],
            |r| r.get(0),
        )?;
        if !user {
            count += tx.execute(
                "UPDATE assertion_review SET check_reason='conflict' WHERE assertion_id=? AND coalesce(check_reason,'')<>'conflict'",
                [&assertion],
            )?;
        }
    }
    Ok(count)
}

pub(crate) fn resolve_document(
    tx: &Transaction<'_>,
    source: &str,
    graph: &DocumentGraph,
) -> Result<GraphOutcome> {
    let kind =
        doc_types::doc_type(&graph.doc_type).ok_or(Error::Validation("Unknown document type."))?;
    let doc = Resolved {
        kind,
        values: graph.values.iter().map(|v| (v.slot.as_str(), v)).collect(),
    };
    let mut outcome = GraphOutcome::default();
    let threshold = check_threshold(tx)?;
    let model = graph.models.join(",");
    let model = (!model.is_empty()).then_some(model);
    let run = hash(json!([source, GRAPH_POLICY, graph]).to_string().as_bytes());
    tx.execute("INSERT OR IGNORE INTO extraction_run(id,source_id,provider,model,pipeline_version,schema_version,started_at,status) VALUES(?,?,'typesafe',?,?,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),'succeeded')",params![run,source,model.clone().unwrap_or_else(||"local".into()),GRAPH_POLICY])?;
    let subject = match &graph.subject {
        Some(s) => Some(canonical_entity(tx, s)?),
        None => None,
    };
    // Entities per role, with the confidence of the values that established them.
    let mut roles: BTreeMap<&str, (String, f64)> = BTreeMap::new();
    for spec in doc.kind.entities {
        let backing: Vec<&SlotValue> = doc
            .kind
            .slots
            .iter()
            .filter(|s| s.target == Target::Role(spec.role))
            .filter_map(|s| doc.values.get(s.key).copied())
            .collect();
        if backing.is_empty() {
            continue;
        }
        let confidence = backing.iter().map(|v| v.confidence).fold(0., f64::max);
        let mut keys = Vec::new();
        for ident in spec.identifiers {
            let Some(value) = doc.text(ident.slot) else {
                continue;
            };
            let (namespace, value) = match ident.namespace {
                "address" => (
                    ident.namespace.to_owned(),
                    normalize_name(&value)
                        .replace("strasse", "str")
                        .replace(' ', ""),
                ),
                _ => (ident.namespace.to_owned(), normalize_identifier(&value)),
            };
            if value.is_empty() {
                continue;
            }
            let namespace = match ident.scope_slot.and_then(|s| doc.text(s)) {
                Some(scope) => format!(
                    "{namespace}@{}",
                    if ident.namespace == "address" {
                        normalize_identifier(&scope)
                    } else {
                        normalize_organization(&scope)
                    }
                ),
                None => namespace,
            };
            keys.push((namespace, value));
        }
        if spec.kind == EntityKind::Organization
            && let Some(name) = spec.label_slot.and_then(|s| doc.text(s))
        {
            let key = normalize_organization(&name);
            if !key.is_empty() {
                keys.push(("organization.name".into(), key));
            }
        }
        let mut entity = None;
        for (namespace, value) in &keys {
            if let Some(found) = find_by_identifier(tx, namespace, value, spec.kind)? {
                entity = Some(found);
                break;
            }
        }
        let entity = match entity {
            Some(e) => e,
            None => {
                outcome.entities_created += 1;
                let named = spec.label_slot.and_then(|s| doc.text(s));
                // A contract or account is named after its provider, but is not it.
                let label = match (named, spec.kind) {
                    (
                        Some(name),
                        EntityKind::InsuranceContract
                        | EntityKind::Contract
                        | EntityKind::Subscription
                        | EntityKind::BankAccount,
                    ) => format!("{} · {name}", spec.fallback_label),
                    (Some(name), _) => name,
                    (None, _) => spec.fallback_label.to_owned(),
                };
                create_entity(tx, spec.kind, &label)?
            }
        };
        for (namespace, value) in &keys {
            add_identifier(tx, &entity, namespace, value)?;
        }
        roles.insert(spec.role, (entity, confidence));
    }
    let target = |t: Target| -> Option<(String, f64)> {
        match t {
            Target::Subject => subject.clone().map(|s| (s, graph.subject_confidence)),
            Target::Role(r) => roles.get(r).cloned(),
        }
    };
    let mut touched: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut record = |tx: &Transaction<'_>,
                      draft: AssertionDraft,
                      confidence: f64,
                      source_kind: ConfidenceSource,
                      value_checked: bool,
                      check: bool,
                      evidence: Option<&Candidate>,
                      outcome: &mut GraphOutcome|
     -> Result<()> {
        let assertion = match insert_assertion(tx, &draft, "extraction", Some(&run)) {
            Ok(a) => a,
            // A value that the property cannot hold stays an observation only.
            Err(Error::Validation(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let locator =
            evidence.map_or_else(|| json!({"kind":"document","source_id":source}), locator);
        tx.execute(
            "INSERT OR IGNORE INTO assertion_evidence VALUES(?,?,?,?)",
            params![
                assertion,
                source,
                hash(locator.to_string().as_bytes()),
                locator.to_string()
            ],
        )?;
        if !auto_accept(tx, &assertion)? {
            return Ok(());
        }
        let check = (check || confidence < threshold).then_some("low_confidence");
        upsert_review(
            tx,
            &assertion,
            confidence,
            source_kind,
            value_checked,
            check,
            model.as_deref(),
        )?;
        outcome.assertions += 1;
        if draft.value.value_type() != "entity" {
            list_assertion(tx, source, &assertion, &draft.property)?;
        }
        touched
            .entry((draft.subject.clone(), draft.property.clone()))
            .or_default()
            .insert(assertion);
        Ok(())
    };
    for slot in doc.kind.slots {
        let (Some(property), Some(value)) = (slot.property, doc.values.get(slot.key)) else {
            continue;
        };
        let Some((subject, _)) = target(slot.target) else {
            continue;
        };
        let value_type: String = tx.query_row(
            "SELECT value_type FROM property_definition WHERE key=?",
            [property],
            |r| r.get(0),
        )?;
        let fixed = match slot.value {
            ValueKind::Money(p) => p,
            _ => None,
        };
        let Some(fact) = fact_value(&value_type, value, fixed) else {
            continue;
        };
        let candidate = match &value.content {
            SlotContent::Candidate(c) => Some((**c).clone()),
            SlotContent::Category { .. } => None,
        };
        if let Some(c) = &candidate {
            tx.execute("INSERT OR IGNORE INTO observation VALUES(?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id(),source,run,format!("{}:{}",graph.doc_type,slot.key),property,serde_json::to_string(&fact).map_err(|_|Error::Format)?,"",c.line,locator(c).to_string(),"supported"])?;
        }
        record(
            tx,
            AssertionDraft {
                subject,
                property: property.into(),
                value: fact,
                validity: doc.validity(slot.validity),
            },
            value.confidence,
            value.source,
            value.value_checked,
            value.check,
            candidate.as_ref(),
            &mut outcome,
        )?;
    }
    for spec in doc.kind.entities {
        let Some((entity, confidence)) = roles.get(spec.role).cloned() else {
            continue;
        };
        for (property, constant) in spec.constants {
            record(
                tx,
                AssertionDraft {
                    subject: entity.clone(),
                    property: (*property).into(),
                    value: FactValue::Text((*constant).into()),
                    validity: Validity::Timeless,
                },
                confidence,
                ConfidenceSource::Typesafe,
                false,
                false,
                None,
                &mut outcome,
            )?;
        }
    }
    for link in doc.kind.links {
        let (Some((from, fc)), Some((to, tc))) = (target(link.from), target(link.to)) else {
            continue;
        };
        record(
            tx,
            AssertionDraft {
                subject: from,
                property: link.property.into(),
                value: FactValue::Entity(to),
                validity: doc.validity(link.validity),
            },
            fc.min(tc),
            ConfidenceSource::Typesafe,
            false,
            false,
            None,
            &mut outcome,
        )?;
    }
    for ((subject, property), assertions) in touched {
        retimeline(tx, &subject, &property)?;
        // Closed replacements carry the conflict check for the closed assertions.
        let mut focus = assertions;
        focus.extend(
            tx.prepare(
                "SELECT id FROM assertion WHERE supersedes_id IN (SELECT value FROM json_each(?))",
            )?
            .query_map(
                [serde_json::to_string(&focus).map_err(|_| Error::Format)?],
                |r| r.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?,
        );
        mark_conflicts(tx, &subject, &property, &focus)?;
    }
    outcome.checks = tx.query_row("SELECT count(*) FROM assertion_review r JOIN assertion_evidence e ON e.assertion_id=r.assertion_id WHERE e.source_id=? AND r.check_reason IS NOT NULL",[source],|r|r.get::<_,i64>(0))? as usize;
    tx.execute(
        "UPDATE document_profile SET graph_state='resolved' WHERE source_id=?",
        [source],
    )?;
    Ok(outcome)
}

fn subject_is_self(db: &rusqlite::Connection, label: &str, profile: &str) -> bool {
    db.query_row("SELECT label FROM entity WHERE id=?", [profile], |r| {
        r.get::<_, String>(0)
    })
    .is_ok_and(|l| l == label)
}
fn check_threshold(db: &rusqlite::Connection) -> Result<f64> {
    let (reviewed, corrected): (i64, i64) = db.query_row(
        "SELECT count(*),coalesce(sum(corrected),0) FROM (SELECT corrected FROM review_outcome WHERE confidence>=? ORDER BY recorded_at DESC LIMIT 50)",
        [DEFAULT_CHECK_THRESHOLD],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if reviewed < 20 {
        DEFAULT_CHECK_THRESHOLD
    } else if corrected * 10 > reviewed {
        0.9
    } else if corrected == 0 {
        0.7
    } else {
        DEFAULT_CHECK_THRESHOLD
    })
}
fn review_state_tx(db: &rusqlite::Connection, assertion: &str) -> Result<ReviewState> {
    let actor: Option<String> = db
        .query_row(
            "SELECT actor FROM decision WHERE assertion_id=? ORDER BY local_seq DESC LIMIT 1",
            [assertion],
            |r| r.get(0),
        )
        .optional()?;
    if actor.as_deref() != Some("policy") {
        return Ok(ReviewState::Reviewed);
    }
    let reason: Option<String> = db
        .query_row(
            "SELECT check_reason FROM assertion_review WHERE assertion_id=?",
            [assertion],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(if reason.is_some() {
        ReviewState::CheckSuggested
    } else {
        ReviewState::Unreviewed
    })
}

impl Vault {
    /// Declared anchors used by Classify and Resolve.
    pub fn identity_anchors(&self) -> Result<IdentityAnchors> {
        let self_entity = self.profile_entity_id()?;
        let self_name: String =
            self.db
                .query_row("SELECT label FROM entity WHERE id=?", [&self_entity], |r| {
                    r.get(0)
                })?;
        let aliases = self
            .db
            .prepare("SELECT alias FROM entity_alias WHERE entity_id=? ORDER BY alias")?
            .query_map([&self_entity], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        let birth_date: Option<String> = self
            .db
            .query_row("SELECT json_extract(value_json,'$') FROM assertion_state WHERE subject_id=? AND property_key='person.birth_date' AND state='accept' ORDER BY recorded_at DESC LIMIT 1",[&self_entity],|r|r.get(0))
            .optional()?;
        let household = self
            .db
            .prepare("SELECT h.entity_id,e.label,h.role FROM household_member h JOIN entity e ON e.id=h.entity_id WHERE e.deleted_at IS NULL ORDER BY h.added_at,e.label")?
            .query_map([], |r| {
                Ok(HouseholdMember {
                    entity: r.get(0)?,
                    name: r.get(1)?,
                    role: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let setup_complete: bool = self.db.query_row(
            "SELECT completed FROM identity_setup WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        Ok(IdentityAnchors {
            self_entity,
            self_name,
            aliases,
            birth_date,
            household,
            setup_complete,
        })
    }
    /// "Who are you?": the self profile's name, former names and birth date.
    pub fn set_identity(
        &mut self,
        name: &str,
        aliases: &[String],
        birth_date: Option<&str>,
    ) -> Result<()> {
        crate::domain::bounded(name, 200)?;
        if let Some(d) = birth_date {
            crate::domain::date(d)?;
        }
        let profile = self.profile_entity_id()?;
        {
            let tx = self.db.transaction()?;
            tx.execute(
                "UPDATE entity SET label=? WHERE id=? AND label<>?",
                params![name.trim(), profile, name.trim()],
            )?;
            tx.execute("DELETE FROM entity_alias WHERE entity_id=?", [&profile])?;
            for alias in aliases.iter().map(|a| a.trim()).filter(|a| !a.is_empty()) {
                crate::domain::bounded(alias, 200)?;
                tx.execute(
                    "INSERT OR IGNORE INTO entity_alias VALUES(?,?,?)",
                    params![profile, alias, normalize_name(alias)],
                )?;
            }
            tx.commit()?;
        }
        if let Some(d) = birth_date {
            let current = self.identity_anchors()?.birth_date;
            if current.as_deref() != Some(d) {
                self.record_fact(&AssertionDraft {
                    subject: profile,
                    property: "person.birth_date".into(),
                    value: FactValue::Date(d.into()),
                    validity: Validity::Timeless,
                })?;
            }
        }
        Ok(())
    }
    pub fn add_household_member(&mut self, name: &str, role: &str) -> Result<String> {
        crate::domain::bounded(name, 200)?;
        if !["partner", "child", "parent", "other"].contains(&role) {
            return Err(Error::Validation("Choose partner, child, parent or other."));
        }
        let key = normalize_name(name);
        let tx = self.db.transaction()?;
        let existing: Option<String> = tx
            .query_row("SELECT h.entity_id FROM household_member h JOIN entity e ON e.id=h.entity_id WHERE e.deleted_at IS NULL AND lower(e.label)=lower(?)",[name.trim()],|r|r.get(0))
            .optional()?;
        let entity = match existing {
            Some(e) => e,
            None => {
                let e = create_entity(&tx, EntityKind::Person, name)?;
                tx.execute(
                    "INSERT INTO household_member VALUES(?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                    params![e, role],
                )?;
                e
            }
        };
        tx.execute(
            "UPDATE household_proposal SET state='accepted' WHERE normalized_name=?",
            [&key],
        )?;
        tx.commit()?;
        Ok(entity)
    }
    /// Removes the anchor role; the person entity and its facts remain.
    pub fn remove_household_member(&mut self, entity: &str) -> Result<()> {
        self.db
            .execute("DELETE FROM household_member WHERE entity_id=?", [entity])?;
        Ok(())
    }
    pub fn complete_identity_setup(&mut self) -> Result<()> {
        self.db.execute(
            "UPDATE identity_setup SET completed=1 WHERE singleton=1",
            [],
        )?;
        Ok(())
    }

    /// Cached output of a stage for identical content, if any.
    pub fn stage_output(&self, content: &str, stage: &str, version: &str) -> Result<Option<Value>> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT output_json FROM stage_run WHERE content_hash=? AND stage=? AND version=?",
                params![content, stage, version],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(|_| Error::Format))
            .transpose()
    }
    pub fn save_stage_output(
        &mut self,
        content: &str,
        stage: &str,
        version: &str,
        output: &Value,
        model: Option<&str>,
        input_tokens: Option<u64>,
    ) -> Result<()> {
        self.db.execute("INSERT INTO stage_run VALUES(?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(content_hash,stage,version) DO UPDATE SET output_json=excluded.output_json,model=excluded.model,input_tokens=excluded.input_tokens,recorded_at=excluded.recorded_at",params![content,stage,version,output.to_string(),model,input_tokens.map(|t|t as i64)])?;
        Ok(())
    }
    pub fn source_fingerprint(&self, source: &str) -> Result<String> {
        Ok(self.db.query_row(
            "SELECT content_fingerprint FROM source WHERE id=?",
            [source],
            |r| r.get(0),
        )?)
    }
    /// Stores the document-level classification and person mentions.
    #[allow(clippy::too_many_arguments)]
    pub fn save_document_profile(
        &mut self,
        source: &str,
        family: &str,
        family_confidence: f64,
        doc_type: Option<&str>,
        type_confidence: Option<f64>,
        subject: Option<&str>,
        subject_name: Option<&str>,
        model: Option<&str>,
    ) -> Result<doc_types::Tier> {
        if doc_types::family(family).is_none() {
            return Err(Error::Validation("Unknown document family."));
        }
        let kind = doc_type.and_then(doc_types::doc_type);
        if doc_type.is_some() && kind.is_none_or(|k| k.family != family) {
            return Err(Error::Validation("Unknown document type."));
        }
        let tier = kind.map_or(doc_types::Tier::Lazy, |k| k.tier);
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO document_profile(source_id,family,doc_type,family_confidence,type_confidence,tier,subject_entity_id,subject_name,graph_state,model,classified_at) VALUES(?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(source_id) DO UPDATE SET family=excluded.family,doc_type=excluded.doc_type,family_confidence=excluded.family_confidence,type_confidence=excluded.type_confidence,tier=excluded.tier,subject_entity_id=excluded.subject_entity_id,subject_name=excluded.subject_name,graph_state=CASE WHEN document_profile.graph_state='resolved' AND document_profile.doc_type IS excluded.doc_type THEN 'resolved' ELSE excluded.graph_state END,model=excluded.model",params![source,family,doc_type,family_confidence.clamp(0.,1.),type_confidence.map(|c|c.clamp(0.,1.)),tier.key(),subject,subject_name,if tier==doc_types::Tier::Eager {"classified"} else {"skipped"},model])?;
        if subject.is_none()
            && let Some(name) = subject_name.map(str::trim).filter(|n| !n.is_empty())
        {
            let key = normalize_name(name);
            tx.execute(
                "INSERT OR IGNORE INTO person_mention VALUES(?,?,?)",
                params![key, name, source],
            )?;
            let mentions: i64 = tx.query_row(
                "SELECT count(*) FROM person_mention WHERE normalized_name=?",
                [&key],
                |r| r.get(0),
            )?;
            if mentions as usize >= HOUSEHOLD_PROPOSAL_MENTIONS {
                tx.execute(
                    "INSERT OR IGNORE INTO household_proposal VALUES(?,?,'open')",
                    params![key, name],
                )?;
            }
        }
        tx.commit()?;
        Ok(tier)
    }
    pub fn document_profile(
        &self,
        source: &str,
    ) -> Result<Option<(String, Option<String>, String)>> {
        Ok(self
            .db
            .query_row(
                "SELECT family,doc_type,graph_state FROM document_profile WHERE source_id=?",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    }
    /// Counts of classified documents per family, for the dump overview.
    pub fn family_counts(&self) -> Result<Vec<(String, usize)>> {
        Ok(self
            .db
            .prepare("SELECT family,count(*) FROM document_profile GROUP BY family ORDER BY count(*) DESC,family")?
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?
            .collect::<std::result::Result<_, _>>()?)
    }
    /// Resolve: writes entities, links and automatic assertions for one document.
    pub fn apply_document_graph(
        &mut self,
        source: &str,
        graph: &DocumentGraph,
    ) -> Result<GraphOutcome> {
        let tx = self.db.transaction()?;
        let personal: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM source WHERE id=? AND sensitivity='personal' AND retention='keep')",
            [source],
            |r| r.get(0),
        )?;
        if !personal {
            return Err(Error::Validation(
                "This document is not available for analysis.",
            ));
        }
        let outcome = resolve_document(&tx, source, graph)?;
        tx.commit()?;
        Ok(outcome)
    }
    /// Nodes are entities, not documents; at most `limit`, anchors first.
    pub fn constellation(&self, limit: usize) -> Result<Constellation> {
        let profile = self.profile_entity_id()?;
        let rows = self.db.prepare("WITH accepted AS (SELECT id,subject_id,object_entity_id FROM assertion_state WHERE state='accept'), links(entity) AS (SELECT subject_id FROM accepted UNION ALL SELECT object_entity_id FROM accepted WHERE object_entity_id IS NOT NULL), counts AS (SELECT entity,count(*) AS n FROM links GROUP BY entity), flagged AS (SELECT DISTINCT a.subject_id AS entity FROM accepted a JOIN assertion_review r ON r.assertion_id=a.id WHERE r.check_reason IS NOT NULL) SELECT e.id,e.label,e.kind,coalesce(c.n,0),e.id IN (SELECT entity FROM flagged),e.id=?1 OR e.id IN (SELECT entity_id FROM household_member) FROM entity e LEFT JOIN counts c ON c.entity=e.id WHERE e.deleted_at IS NULL AND e.id NOT IN (SELECT from_id FROM entity_merge WHERE reversed_at IS NULL)")?
            .query_map([&profile], |r| {
                Ok(ConstellationNode {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    kind: r.get(2)?,
                    links: r.get::<_, i64>(3)? as usize,
                    check: r.get(4)?,
                    anchor: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut nodes: Vec<ConstellationNode> = rows
            .into_iter()
            .filter(|n| n.anchor || (n.links > 0 && n.kind != "person"))
            .collect();
        nodes.sort_by(|a, b| {
            (b.id == profile, b.anchor, b.links, &a.label).cmp(&(
                a.id == profile,
                a.anchor,
                a.links,
                &b.label,
            ))
        });
        nodes.truncate(limit.max(1));
        let ids: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let mut edges = BTreeSet::new();
        let pairs = self.db.prepare("SELECT subject_id,object_entity_id FROM assertion_state WHERE state='accept' AND object_entity_id IS NOT NULL")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (a, b) in pairs {
            let (Ok(a), Ok(b)) = (
                canonical_entity(&self.db, &a),
                canonical_entity(&self.db, &b),
            ) else {
                continue;
            };
            if a != b && ids.contains(a.as_str()) && ids.contains(b.as_str()) {
                edges.insert(if a < b { (a, b) } else { (b, a) });
            }
        }
        let clusters = self
            .family_counts()?
            .into_iter()
            .map(|(family, n)| {
                (
                    doc_types::family(&family).map_or(family.clone(), |f| f.label.to_owned()),
                    n,
                )
            })
            .collect();
        let latest: Option<(String, String, String, String)> = self.db.query_row("SELECT e.label,p.label,a.value_json,a.value_type FROM assertion a JOIN entity e ON e.id=a.subject_id JOIN property_definition p ON p.key=a.property_key WHERE a.origin='extraction' AND a.value_type<>'entity' AND (SELECT actor FROM decision d WHERE d.assertion_id=a.id ORDER BY d.local_seq DESC LIMIT 1)='policy' AND (SELECT action FROM decision d WHERE d.assertion_id=a.id ORDER BY d.local_seq DESC LIMIT 1)='accept' ORDER BY a.recorded_at DESC,a.rowid DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let latest = latest.map(|(subject, label, raw, _)| {
            let value = crate::domain::display_value(&raw);
            if subject_is_self(&self.db, &subject, &profile) {
                format!("Your {} · {value}", label.to_lowercase())
            } else {
                format!("{subject} · {label} · {value}")
            }
        });
        Ok(Constellation {
            nodes,
            edges: edges.into_iter().collect(),
            clusters,
            latest,
        })
    }
    pub fn review_state(&self, assertion: &str) -> Result<ReviewState> {
        review_state_tx(&self.db, assertion)
    }
    /// Automatic values worth a quick look, grouped per entity.
    pub fn quick_checks(&self) -> Result<Vec<QuickCheckGroup>> {
        let rows = self.db.prepare("SELECT a.id,a.subject_id,e.label,e.kind,a.property_key,p.label,a.value_json,r.check_reason,a.value_type,(SELECT s.title FROM assertion_evidence ev JOIN source s ON s.id=ev.source_id WHERE ev.assertion_id=a.id AND s.kind='file' LIMIT 1) FROM assertion_state a JOIN assertion_review r ON r.assertion_id=a.id JOIN entity e ON e.id=a.subject_id JOIN property_definition p ON p.key=a.property_key WHERE a.state='accept' AND r.check_reason IS NOT NULL AND e.deleted_at IS NULL AND (SELECT actor FROM decision d WHERE d.assertion_id=a.id ORDER BY d.local_seq DESC LIMIT 1)='policy' ORDER BY e.label,p.label,a.id")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut groups: Vec<QuickCheckGroup> = Vec::new();
        for (
            assertion,
            entity,
            entity_label,
            kind,
            property,
            label,
            raw,
            reason,
            value_type,
            title,
        ) in rows
        {
            let value = if value_type == "entity" {
                // Entity-valued facts show the linked entity's label.
                let linked: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                let linked = linked.as_str().unwrap_or_default().to_owned();
                self.db
                    .query_row("SELECT label FROM entity WHERE id=?", [&linked], |r| {
                        r.get(0)
                    })
                    .optional()?
                    .unwrap_or(linked)
            } else {
                crate::domain::display_value(&raw)
            };
            let item = QuickCheck {
                assertion,
                property,
                label,
                value,
                reason,
                source_title: title,
            };
            match groups.last_mut() {
                Some(g) if g.entity == entity => g.items.push(item),
                _ => groups.push(QuickCheckGroup {
                    entity,
                    entity_label,
                    entity_kind: kind,
                    items: vec![item],
                }),
            }
        }
        Ok(groups)
    }
    fn record_review(&mut self, assertion: &str, accept: bool, corrected: bool) -> Result<()> {
        let tx = self.db.transaction()?;
        let confidence: Option<f64> = tx
            .query_row(
                "SELECT confidence FROM assertion_review WHERE assertion_id=?",
                [assertion],
                |r| r.get(0),
            )
            .optional()?;
        let Some(confidence) = confidence else {
            return Err(Error::Validation("This value is not an automatic value."));
        };
        crate::vault::decision(&tx, assertion, if accept { "accept" } else { "retract" })?;
        tx.execute("UPDATE assertion_review SET check_reason=NULL,confidence_source='user',confidence=1 WHERE assertion_id=? AND ?",params![assertion,accept])?;
        tx.execute("INSERT INTO review_outcome VALUES(?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(assertion_id) DO UPDATE SET corrected=excluded.corrected,recorded_at=excluded.recorded_at",params![assertion,confidence,corrected])?;
        if !accept {
            tx.execute(
                "UPDATE collection_item SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE current_assertion_id=? AND kind='note' AND deleted_at IS NULL",
                [assertion],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    /// "Looks right": marks an automatic value as reviewed.
    pub fn confirm_assertion(&mut self, assertion: &str) -> Result<()> {
        self.record_review(assertion, true, false)
    }
    /// "Not right": retracts an automatic value; its evidence stays.
    pub fn reject_assertion(&mut self, assertion: &str) -> Result<()> {
        self.record_review(assertion, false, true)
    }
    /// Replaces an automatic value with the user's correction.
    pub fn correct_assertion(&mut self, assertion: &str, value: FactValue) -> Result<String> {
        let (subject, property, kind, from, to): (String, String, String, Option<String>, Option<String>) = self.db.query_row(
            "SELECT subject_id,property_key,time_kind,valid_from,valid_to FROM assertion WHERE id=?",
            [assertion],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let validity = match (kind.as_str(), from) {
            ("interval", Some(from)) => Validity::Interval { from, to },
            ("point", Some(date)) => Validity::Point { date },
            ("timeless", _) => Validity::Timeless,
            _ => Validity::Unknown,
        };
        self.record_review(assertion, false, true)?;
        self.record_fact(&AssertionDraft {
            subject,
            property,
            value,
            validity,
        })
    }
    /// Threshold below which automatic values get a quick check. It rises when
    /// reviewed confident values were often wrong and falls after clean reviews.
    pub fn check_threshold(&self) -> Result<f64> {
        check_threshold(&self.db)
    }
    pub fn household_proposals(&self) -> Result<Vec<HouseholdProposal>> {
        Ok(self.db.prepare("SELECT p.name,(SELECT count(*) FROM person_mention m WHERE m.normalized_name=p.normalized_name) FROM household_proposal p WHERE p.state='open' ORDER BY p.name")?
            .query_map([], |r| Ok(HouseholdProposal{name:r.get(0)?,documents:r.get::<_,i64>(1)? as usize}))?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn dismiss_household_proposal(&mut self, name: &str) -> Result<()> {
        self.db.execute(
            "UPDATE household_proposal SET state='dismissed' WHERE normalized_name=?",
            [normalize_name(name)],
        )?;
        Ok(())
    }
    /// Organizations sharing a leading name word: candidates for an alignment check.
    pub fn merge_candidates(&self, limit: usize) -> Result<Vec<MergeCandidate>> {
        let rows = self
            .db
            .prepare("SELECT e.id,e.label,e.kind FROM entity e WHERE e.kind='organization' AND e.deleted_at IS NULL AND NOT EXISTS(SELECT 1 FROM entity_merge m WHERE m.from_id=e.id AND m.reversed_at IS NULL) ORDER BY e.id")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut blocks: BTreeMap<String, Vec<&(String, String, String)>> = BTreeMap::new();
        for row in &rows {
            if let Some(first) = normalize_organization(&row.1).split(' ').next()
                && first.len() >= 3
            {
                blocks.entry(first.to_owned()).or_default().push(row);
            }
        }
        let mut out = Vec::new();
        for members in blocks.values() {
            for (i, a) in members.iter().enumerate() {
                for b in &members[i + 1..] {
                    let (a, b) = if a.0 < b.0 { (a, b) } else { (b, a) };
                    let decided: bool = self.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM merge_proposal WHERE entity_a=? AND entity_b=?)",
                        params![a.0, b.0],
                        |r| r.get(0),
                    )?;
                    if !decided {
                        out.push(MergeCandidate {
                            a: a.0.clone(),
                            a_label: a.1.clone(),
                            b: b.0.clone(),
                            b_label: b.1.clone(),
                            kind: a.2.clone(),
                        });
                    }
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
    /// Stores a TypeSafe alignment judgment. Only `open` proposals reach the user.
    pub fn record_merge_judgment(
        &mut self,
        a: &str,
        b: &str,
        score: f64,
        propose: bool,
    ) -> Result<()> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        self.db.execute(
            "INSERT OR IGNORE INTO merge_proposal VALUES(?,?,?,?,?)",
            params![
                id(),
                a,
                b,
                score.clamp(0., 1.),
                if propose { "open" } else { "dismissed" }
            ],
        )?;
        Ok(())
    }
    pub fn merge_proposals(&self) -> Result<Vec<MergeCandidate>> {
        Ok(self.db.prepare("SELECT p.entity_a,a.label,p.entity_b,b.label,a.kind FROM merge_proposal p JOIN entity a ON a.id=p.entity_a JOIN entity b ON b.id=p.entity_b WHERE p.state='open' AND a.deleted_at IS NULL AND b.deleted_at IS NULL ORDER BY p.score DESC")?
            .query_map([], |r| Ok(MergeCandidate{a:r.get(0)?,a_label:r.get(1)?,b:r.get(2)?,b_label:r.get(3)?,kind:r.get(4)?}))?
            .collect::<std::result::Result<_, _>>()?)
    }
    /// Accepting merges the two entities (reversible); dismissing keeps them apart.
    pub fn decide_merge_proposal(&mut self, a: &str, b: &str, accept: bool) -> Result<()> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        if accept {
            self.merge_entities(b, a)?;
        }
        self.db.execute(
            "UPDATE merge_proposal SET state=? WHERE entity_a=? AND entity_b=?",
            params![if accept { "accepted" } else { "dismissed" }, a, b],
        )?;
        Ok(())
    }
    /// Automatic values flagged as conflicting, for an equivalence check.
    pub fn open_value_conflicts(&self, limit: usize) -> Result<Vec<ValueConflict>> {
        let rows = self.db.prepare("SELECT a.id,a.subject_id,a.property_key,a.value_json,p.label FROM assertion_state a JOIN assertion_review r ON r.assertion_id=a.id JOIN property_definition p ON p.key=a.property_key WHERE a.state='accept' AND r.check_reason='conflict' AND a.value_type IN ('text','identifier') ORDER BY a.subject_id,a.property_key,a.id")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for (i, a) in rows.iter().enumerate() {
            for b in &rows[i + 1..] {
                if a.1 == b.1 && a.2 == b.2 && a.3 != b.3 {
                    out.push(ValueConflict {
                        first: a.0.clone(),
                        first_value: crate::domain::display_value(&a.3),
                        second: b.0.clone(),
                        second_value: crate::domain::display_value(&b.3),
                        label: a.4.clone(),
                    });
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
    /// Two differently written values mean the same thing: keep the first, retract
    /// the duplicate by policy and move its evidence. Both remain auditable.
    pub fn resolve_equivalent_values(&mut self, keep: &str, duplicate: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        if review_state_tx(&tx, duplicate)? == ReviewState::Reviewed {
            return Err(Error::Validation(
                "A reviewed value is not changed automatically.",
            ));
        }
        tx.execute("INSERT OR IGNORE INTO assertion_evidence SELECT ?,source_id,evidence_key,locator_json FROM assertion_evidence WHERE assertion_id=?",params![keep,duplicate])?;
        policy_decision(&tx, duplicate, "retract", "equivalent_value")?;
        tx.execute(
            "UPDATE collection_item SET deleted_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE current_assertion_id=? AND deleted_at IS NULL",
            [duplicate],
        )?;
        let (subject, property): (String, String) = tx.query_row(
            "SELECT subject_id,property_key FROM assertion WHERE id=?",
            [keep],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        tx.execute(
            "UPDATE assertion_review SET check_reason=NULL WHERE assertion_id=? AND check_reason='conflict'",
            [keep],
        )?;
        mark_conflicts(&tx, &subject, &property, &BTreeSet::new())?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
