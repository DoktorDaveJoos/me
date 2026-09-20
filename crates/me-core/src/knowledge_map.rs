//! Trusted local graph projection. Values stay in their canonical records; this
//! read model is never included in an agent tool, search scope or cloud request.
use crate::{Error, Result, Vault};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexPosition {
    pub q: i32,
    pub r: i32,
}
impl HexPosition {
    pub fn distance(self, other: Self) -> i64 {
        let dq = i64::from(self.q) - i64::from(other.q);
        let dr = i64::from(self.r) - i64::from(other.r);
        (dq.abs() + dr.abs() + (dq + dr).abs()) / 2
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeKind {
    Fact,
    Note,
    Document,
    Credential,
    Entity,
    Suggestion,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeStatus {
    Confirmed,
    Conflicting,
    NeedsReview,
    Unverified,
    Stored,
    Restricted,
}
impl KnowledgeStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Confirmed => "Confirmed",
            Self::Conflicting => "Conflicting values",
            Self::NeedsReview => "Confirm ownership",
            Self::Unverified => "Source unverified",
            Self::Stored => "Stored",
            Self::Restricted => "Content restricted",
        }
    }
    pub fn needs_review(self) -> bool {
        matches!(self, Self::NeedsReview | Self::Unverified)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeRelation {
    Evidence,
    EnteredByYou,
    Unconfirmed,
    Context,
    Related,
    Attachment,
    Revision,
}
impl KnowledgeRelation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Evidence => "Supported by",
            Self::EnteredByYou => "Entered by you",
            Self::Unconfirmed => "Found in · ownership unconfirmed",
            Self::Context => "Context only",
            Self::Related => "Related to",
            Self::Attachment => "Attachment of",
            Self::Revision => "Revision of",
        }
    }
}
#[derive(Clone, Debug)]
pub struct KnowledgeNode {
    pub id: String,
    pub kind: KnowledgeKind,
    pub status: KnowledgeStatus,
    pub label: String,
    pub value: String,
    pub context: String,
    pub item: Option<u64>,
    pub group: String,
    pub group_label: String,
    pub position: HexPosition,
    /// Old presentation identities that may transfer their cell upon confirmation.
    pub(crate) aliases: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct KnowledgeEdge {
    pub from: String,
    pub to: String,
    pub relation: KnowledgeRelation,
    pub location: String,
    pub quote: String,
}
impl KnowledgeEdge {
    pub fn label_from(&self, node: &str) -> &'static str {
        if node == self.from {
            return self.relation.label();
        }
        match self.relation {
            KnowledgeRelation::Evidence => "Supports",
            KnowledgeRelation::EnteredByYou => "Provides",
            KnowledgeRelation::Unconfirmed => "Contains candidate",
            KnowledgeRelation::Context => "Context for",
            KnowledgeRelation::Related => "Related to",
            KnowledgeRelation::Attachment => "Has attachment",
            KnowledgeRelation::Revision => "Has revision",
        }
    }
}
#[derive(Clone, Debug)]
pub struct KnowledgeGroup {
    pub id: String,
    pub label: String,
    pub anchor: HexPosition,
    pub count: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeViewport {
    pub center_q: f64,
    pub center_r: f64,
    pub zoom: f64,
    pub selected_node: Option<String>,
}
impl Default for KnowledgeViewport {
    fn default() -> Self {
        Self {
            center_q: 0.,
            center_r: 0.,
            zoom: 1.,
            selected_node: None,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct KnowledgeMap {
    pub nodes: Vec<KnowledgeNode>,
    pub edges: Vec<KnowledgeEdge>,
    pub groups: Vec<KnowledgeGroup>,
    pub viewport: KnowledgeViewport,
    pub added: usize,
}
impl KnowledgeMap {
    pub fn node(&self, id: &str) -> Option<&KnowledgeNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
}
fn text(value: &str) -> Result<String> {
    let v: Value = serde_json::from_str(value).map_err(|_| Error::Format)?;
    Ok(v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string()))
}
fn location(v: &Value) -> String {
    let v = v.get("source_locator").unwrap_or(v);
    v["page"]
        .as_u64()
        .map(|p| format!("Page {p}"))
        .or_else(|| v["section"].as_str().map(str::to_owned))
        .unwrap_or_default()
}
fn short_label(label: String, rules: &Value) -> String {
    rules["extracted_property"]
        .as_str()
        .and_then(crate::document_field_label)
        .map(str::to_owned)
        .unwrap_or(label)
}
fn node(
    id: String,
    kind: KnowledgeKind,
    label: String,
    value: String,
    item: Option<u64>,
    group: String,
    group_label: String,
) -> KnowledgeNode {
    KnowledgeNode {
        id,
        kind,
        status: KnowledgeStatus::Stored,
        label,
        value,
        context: String::new(),
        item,
        group,
        group_label,
        position: HexPosition::default(),
        aliases: Vec::new(),
    }
}
impl Vault {
    /// Build and reconcile on the vault worker. Existing cells never move on a
    /// refresh; new nodes alone are optimized against the saved neighborhood.
    pub fn knowledge_map(&mut self) -> Result<KnowledgeMap> {
        let mut graph = self.knowledge_projection()?;
        crate::knowledge_layout::reconcile(&mut self.db, &mut graph)?;
        graph.viewport = self.knowledge_viewport()?;
        Ok(graph)
    }
    pub fn knowledge_viewport(&self) -> Result<KnowledgeViewport> {
        Ok(self
            .db
            .query_row(
                "SELECT center_q,center_r,zoom,selected_node FROM knowledge_view WHERE singleton=1",
                [],
                |r| {
                    Ok(KnowledgeViewport {
                        center_q: r.get(0)?,
                        center_r: r.get(1)?,
                        zoom: r.get(2)?,
                        selected_node: r.get(3)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }
    pub fn save_knowledge_viewport(&mut self, view: &KnowledgeViewport) -> Result<()> {
        if !view.center_q.is_finite()
            || !view.center_r.is_finite()
            || !view.zoom.is_finite()
            || view.center_q.abs() > 1_000_000.
            || view.center_r.abs() > 1_000_000.
            || !(0.4..=2.).contains(&view.zoom)
        {
            return Err(Error::Validation("Invalid map position."));
        }
        let selected = if let Some(id) = &view.selected_node {
            self.db
                .query_row(
                    "SELECT node_id FROM knowledge_position WHERE node_id=?",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
        } else {
            None
        };
        self.db.execute("INSERT INTO knowledge_view VALUES(1,?,?,?,?) ON CONFLICT(singleton) DO UPDATE SET center_q=excluded.center_q,center_r=excluded.center_r,zoom=excluded.zoom,selected_node=excluded.selected_node", params![view.center_q,view.center_r,view.zoom,selected])?;
        Ok(())
    }
    fn knowledge_projection(&self) -> Result<KnowledgeMap> {
        let mut graph = KnowledgeMap::default();
        let mut sources = BTreeMap::<String, String>::new();
        let mut manual = BTreeMap::<String, String>::new();
        let mut nodes = BTreeMap::<String, KnowledgeNode>::new();
        let mut stmt = self
            .db
            .prepare("SELECT id,label,kind FROM entity WHERE deleted_at IS NULL")?;
        let entities = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, String>(1)?, r.get::<_, String>(2)?),
                ))
            })?
            .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
        drop(stmt);
        let mut properties = BTreeMap::<String, (String, String, String)>::new();
        let mut accepted_aliases = BTreeMap::<(String, String, String), Vec<String>>::new();
        let mut alias_stmt = self.db.prepare(
            "SELECT source_id,property_key,value,id FROM ai_proposal WHERE state='accepted'",
        )?;
        for row in alias_stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (source, property, value, id) = row?;
            accepted_aliases
                .entry((source, property, value))
                .or_default()
                .push(format!("proposal:{id}"));
        }
        drop(alias_stmt);
        let mut stmt = self.db.prepare("SELECT i.local_id,i.stable_id,i.kind,i.title,i.source_id,coalesce(a.value_json,'null'),s.sensitivity,s.retention,coalesce(f.path,''),coalesce(c.vault_name,''),coalesce(i.current_assertion_id,''),coalesce(c.category,'') FROM collection_item i JOIN source s ON s.id=i.source_id LEFT JOIN assertion a ON a.id=i.current_assertion_id LEFT JOIN document_folder f ON f.item_id=i.local_id LEFT JOIN credential_record c ON c.item_id=i.local_id WHERE i.deleted_at IS NULL AND (i.kind!='note' OR s.kind='note') AND s.retention NOT IN ('purge_requested','purged') ORDER BY i.local_id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, String>(11)?,
            ))
        })?;
        for row in rows {
            let (
                item,
                stable,
                kind,
                title,
                source,
                raw,
                sensitivity,
                retention,
                folder,
                vault,
                assertion,
                category,
            ) = row?;
            let id = format!("item:{stable}");
            let (kind, group, label, value) = match kind.as_str() {
                "note" => (
                    KnowledgeKind::Note,
                    "manual".to_owned(),
                    "Your details".to_owned(),
                    if sensitivity == "personal" && retention == "keep" {
                        text(&raw)?
                    } else {
                        String::new()
                    },
                ),
                "credential" => (
                    KnowledgeKind::Credential,
                    format!("credentials:{vault}"),
                    if vault.is_empty() {
                        "Logins".into()
                    } else {
                        format!("Logins · {vault}")
                    },
                    category,
                ),
                _ => (
                    KnowledgeKind::Document,
                    if folder.is_empty() {
                        id.clone()
                    } else {
                        format!("folder:{}", folder.split('/').next().unwrap_or(&folder))
                    },
                    if folder.is_empty() {
                        title.clone()
                    } else {
                        folder.split('/').next().unwrap_or(&folder).to_owned()
                    },
                    String::new(),
                ),
            };
            let mut n = node(
                id.clone(),
                kind,
                title,
                value,
                Some(item as u64),
                group,
                label,
            );
            n.status = if sensitivity != "personal" || retention != "keep" {
                KnowledgeStatus::Restricted
            } else if kind == KnowledgeKind::Note {
                KnowledgeStatus::Confirmed
            } else {
                KnowledgeStatus::Stored
            };
            if kind == KnowledgeKind::Credential {
                n.context = "Open this entry to view protected fields.".into();
            }
            if kind == KnowledgeKind::Note && !assertion.is_empty() {
                manual.entry(assertion).or_insert_with(|| id.clone());
            }
            sources.insert(source, id.clone());
            nodes.insert(id, n);
        }
        drop(stmt);
        // Include every live confirmed assertion and all of its retained evidence.
        // Property cardinality and scoped keys prevent monthly values becoming false conflicts.
        let mut stmt = self.db.prepare("SELECT a.id,a.subject_id,a.property_key,p.label,a.value_json,p.cardinality,p.rules_json,a.state,a.object_entity_id,e.source_id,e.locator_json FROM assertion_state a JOIN property_definition p ON p.key=a.property_key JOIN assertion_evidence e ON e.assertion_id=a.id JOIN source s ON s.id=e.source_id WHERE a.state IN ('accept','proposed') AND s.sensitivity='personal' AND s.retention='keep' AND EXISTS(SELECT 1 FROM collection_item i WHERE i.source_id=s.id AND i.deleted_at IS NULL AND i.kind!='credential') ORDER BY a.recorded_at,a.id,e.source_id,e.evidence_key")?;
        let mut entity_links = Vec::new();
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })? {
            let (
                assertion,
                subject,
                property,
                label,
                raw,
                cardinality,
                rules,
                state,
                object,
                source,
                locator,
            ) = row?;
            let Some(source_id) = sources.get(&source) else {
                continue;
            };
            let source_node = nodes[source_id].clone();
            let id = manual
                .get(&assertion)
                .cloned()
                .unwrap_or_else(|| format!("fact:{assertion}"));
            let rules: Value = serde_json::from_str(&rules).map_err(|_| Error::Format)?;
            let locator: Value = serde_json::from_str(&locator).map_err(|_| Error::Format)?;
            let value = object
                .as_deref()
                .and_then(|id| entities.get(id))
                .map(|(label, _)| label.clone())
                .unwrap_or(text(&raw)?);
            let n = nodes.entry(id.clone()).or_insert_with(|| {
                node(
                    id.clone(),
                    KnowledgeKind::Fact,
                    short_label(label, &rules),
                    value,
                    source_node.item,
                    source_node.group.clone(),
                    source_node.group_label.clone(),
                )
            });
            n.status = if state == "accept" {
                KnowledgeStatus::Confirmed
            } else {
                KnowledgeStatus::NeedsReview
            };
            n.context = rules["context_quote"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !id.starts_with("fact:") {
                n.aliases.push(format!("fact:{assertion}"));
            }
            if let Some(q) = locator["review_question_id"].as_str() {
                n.aliases.push(format!("question:{q}"));
            }
            properties.insert(id.clone(), (subject.clone(), property.clone(), cardinality));
            if state == "accept" {
                entity_links.push((id.clone(), subject, object));
            }
            let relation = if state != "accept" {
                KnowledgeRelation::Unconfirmed
            } else if matches!(
                locator["kind"].as_str(),
                Some("manual" | "manual_confirmation")
            ) {
                KnowledgeRelation::EnteredByYou
            } else {
                KnowledgeRelation::Evidence
            };
            if &id != source_id {
                graph.edges.push(KnowledgeEdge {
                    from: id.clone(),
                    to: source_id.clone(),
                    relation,
                    location: location(&locator),
                    quote: locator["quote"].as_str().unwrap_or_default().to_owned(),
                });
            }
            if let Some(context) = locator["context_source_id"]
                .as_str()
                .and_then(|s| sources.get(s))
                && context != &id
            {
                graph.edges.push(KnowledgeEdge {
                    from: id.clone(),
                    to: context.clone(),
                    relation: KnowledgeRelation::Context,
                    location: "Value confirmed by you; this document is context only.".into(),
                    quote: String::new(),
                });
            }
            // A verified proposal keeps its cell when it becomes a confirmed fact.
            if let Some(aliases) =
                accepted_aliases.get(&(source, property, nodes[&id].value.clone()))
            {
                nodes
                    .get_mut(&id)
                    .unwrap()
                    .aliases
                    .extend(aliases.iter().cloned());
            }
        }
        drop(stmt);
        // Questions explicitly carry no claimed model quotation as source evidence.
        for (table, prefix, pending, unverified) in [
            ("ai_proposal", "proposal", "proposed", false),
            ("ai_question", "question", "pending", true),
        ] {
            let sql = format!(
                "SELECT x.id,x.property_key,p.label,p.rules_json,x.value,x.source_id FROM {table} x JOIN property_definition p ON p.key=x.property_key JOIN source s ON s.id=x.source_id WHERE x.state=? AND s.sensitivity='personal' AND s.retention='keep' ORDER BY x.id"
            );
            let mut stmt = self.db.prepare(&sql)?;
            for row in stmt.query_map([pending], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })? {
                let (id, _property, label, rules, value, source) = row?;
                let Some(source_id) = sources.get(&source) else {
                    continue;
                };
                let source_node = &nodes[source_id];
                let rules: Value = serde_json::from_str(&rules).map_err(|_| Error::Format)?;
                let id = format!("{prefix}:{id}");
                let mut n = node(
                    id.clone(),
                    KnowledgeKind::Suggestion,
                    short_label(label, &rules),
                    value,
                    source_node.item,
                    source_node.group.clone(),
                    source_node.group_label.clone(),
                );
                n.status = if unverified {
                    KnowledgeStatus::Unverified
                } else {
                    KnowledgeStatus::NeedsReview
                };
                n.context = if unverified {
                    "This suggested value or its source could not be verified. Confirm it in Review.".into()
                } else {
                    "The source contains this value. Confirm that it belongs to you in Review."
                        .into()
                };
                graph.edges.push(KnowledgeEdge {
                    from: id.clone(),
                    to: source_id.clone(),
                    relation: if unverified {
                        KnowledgeRelation::Context
                    } else {
                        KnowledgeRelation::Unconfirmed
                    },
                    location: String::new(),
                    quote: String::new(),
                });
                nodes.insert(id, n);
            }
        }
        let mut values = BTreeMap::<(String, String), BTreeSet<String>>::new();
        for (id, (subject, property, cardinality)) in &properties {
            if cardinality == "one" && nodes[id].status == KnowledgeStatus::Confirmed {
                values
                    .entry((subject.clone(), property.clone()))
                    .or_default()
                    .insert(nodes[id].value.clone());
            }
        }
        for (id, (subject, property, cardinality)) in &properties {
            if cardinality == "one"
                && values
                    .get(&(subject.clone(), property.clone()))
                    .is_some_and(|v| v.len() > 1)
                && nodes[id].status == KnowledgeStatus::Confirmed
            {
                nodes.get_mut(id).unwrap().status = KnowledgeStatus::Conflicting;
            }
        }
        let profile: String = self
            .db
            .query_row("SELECT profile_id FROM vault_meta", [], |r| r.get(0))?;
        let mut stmt = self
            .db
            .prepare("SELECT source_id,entity_id FROM source_entity WHERE status='accepted'")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (s, e) = row?;
            if let Some(n) = sources.get(&s) {
                entity_links.push((n.clone(), e, None));
            }
        }
        for (from, subject, object) in entity_links {
            for entity in [Some(subject), object]
                .into_iter()
                .flatten()
                .filter(|e| e != &profile)
            {
                let Some((label, kind)) = entities.get(&entity) else {
                    continue;
                };
                let id = format!("entity:{entity}");
                let parent = &nodes[&from];
                let mut n = node(
                    id.clone(),
                    KnowledgeKind::Entity,
                    kind.clone(),
                    label.clone(),
                    None,
                    parent.group.clone(),
                    parent.group_label.clone(),
                );
                n.status = KnowledgeStatus::Confirmed;
                nodes.entry(id.clone()).or_insert(n);
                graph.edges.push(KnowledgeEdge {
                    from: from.clone(),
                    to: id,
                    relation: KnowledgeRelation::Related,
                    location: String::new(),
                    quote: String::new(),
                });
            }
        }
        let mut stmt = self.db.prepare(
            "SELECT parent_id,child_id,role FROM source_link ORDER BY parent_id,child_id,role",
        )?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (parent, child, role) = row?;
            if let (Some(parent), Some(child)) = (sources.get(&parent), sources.get(&child)) {
                graph.edges.push(KnowledgeEdge {
                    from: child.clone(),
                    to: parent.clone(),
                    relation: if role == "revision_of" {
                        KnowledgeRelation::Revision
                    } else {
                        KnowledgeRelation::Attachment
                    },
                    location: String::new(),
                    quote: String::new(),
                });
            }
        }
        graph.nodes = nodes.into_values().collect();
        let mut seen = BTreeSet::new();
        graph.edges.retain(|e| {
            e.from != e.to
                && seen.insert((
                    e.from.clone(),
                    e.to.clone(),
                    e.relation as u8,
                    e.location.clone(),
                    e.quote.clone(),
                ))
        });
        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleting_a_source_removes_cached_cells_and_selection_without_moving_survivors() {
        let temp = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&temp.path().join("vault"), "synthetic-map-test").unwrap();
        let item = vault.save_note(None, "One", "First value").unwrap();
        vault.save_note(None, "Two", "Second value").unwrap();
        let before = vault.knowledge_map().unwrap();
        let selected = before.nodes.iter().find(|n| n.item == Some(item)).unwrap();
        vault
            .save_knowledge_viewport(&KnowledgeViewport {
                selected_node: Some(selected.id.clone()),
                ..Default::default()
            })
            .unwrap();
        vault
            .db
            .execute(
                "UPDATE collection_item SET deleted_at='synthetic' WHERE local_id=?",
                [item as i64],
            )
            .unwrap();
        let after = vault.knowledge_map().unwrap();
        assert_eq!(after.nodes.len(), 1);
        assert_eq!(
            after.nodes[0].position,
            before.node(&after.nodes[0].id).unwrap().position
        );
        assert!(after.viewport.selected_node.is_none());
        assert_eq!(
            vault
                .db
                .query_row("SELECT count(*) FROM knowledge_position", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
