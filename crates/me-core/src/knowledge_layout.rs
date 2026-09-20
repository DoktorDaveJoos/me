//! Deterministic, incremental packing. Historical positions are constraints, not
//! starting guesses for a force simulation. A viewport never influences layout.
use crate::{Error, HexPosition, KnowledgeGroup, KnowledgeKind, KnowledgeMap, Result};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};

const DIRECTIONS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
#[derive(Clone)]
struct Saved {
    group: String,
    position: HexPosition,
}
#[derive(Default)]
struct Cluster {
    q: i64,
    r: i64,
    count: i64,
    anchor: HexPosition,
}
impl Cluster {
    fn add(&mut self, p: HexPosition) {
        if self.count == 0 {
            self.anchor = p;
        }
        self.q += i64::from(p.q);
        self.r += i64::from(p.r);
        self.count += 1;
    }
    fn center(&self) -> HexPosition {
        HexPosition {
            q: (self.q / self.count.max(1)) as i32,
            r: (self.r / self.count.max(1)) as i32,
        }
    }
}
fn ring(center: HexPosition, radius: i32) -> Vec<HexPosition> {
    if radius == 0 {
        return vec![center];
    }
    let mut p = HexPosition {
        q: center.q - radius,
        r: center.r + radius,
    };
    let mut result = Vec::with_capacity((radius * 6) as usize);
    for (q, r) in DIRECTIONS {
        for _ in 0..radius {
            result.push(p);
            p.q += q;
            p.r += r;
        }
    }
    result
}
fn new_cluster(occupied: &BTreeMap<HexPosition, String>) -> HexPosition {
    for radius in 0.. {
        for p in ring(HexPosition::default(), radius) {
            let p = HexPosition {
                q: p.q * 4,
                r: p.r * 4,
            };
            if (0..=2)
                .flat_map(|r| ring(p, r))
                .all(|p| !occupied.contains_key(&p))
            {
                return p;
            }
        }
    }
    unreachable!()
}
fn candidate(
    group: &str,
    center: HexPosition,
    neighbors: &[HexPosition],
    occupied: &BTreeMap<HexPosition, String>,
) -> HexPosition {
    let target = if neighbors.is_empty() {
        center
    } else {
        HexPosition {
            q: (neighbors.iter().map(|p| i64::from(p.q)).sum::<i64>() / neighbors.len() as i64)
                as i32,
            r: (neighbors.iter().map(|p| i64::from(p.r)).sum::<i64>() / neighbors.len() as i64)
                as i32,
        }
    };
    let mut choices = Vec::new();
    let mut first = None;
    for radius in 0.. {
        for p in ring(target, radius) {
            if occupied.contains_key(&p) {
                continue;
            }
            first.get_or_insert(radius);
            let (mut same, mut foreign) = (0, 0);
            for n in ring(p, 1) {
                if let Some(g) = occupied.get(&n) {
                    if g == group {
                        same += 1;
                    } else {
                        foreign += 1;
                    }
                }
            }
            let related = if neighbors.is_empty() {
                0
            } else {
                neighbors.iter().map(|n| p.distance(*n)).sum::<i64>() * 16 / neighbors.len() as i64
            };
            let score =
                related + p.distance(center) * 4 + foreign * 24 - same * 2 + (same - 4).max(0) * 4;
            choices.push((score, p.r, p.q, p));
        }
        if first.is_some_and(|r| radius >= r + 2) {
            break;
        }
    }
    choices.sort_by_key(|(score, r, q, _)| (*score, *r, *q));
    choices[0].3
}
pub(crate) fn reconcile(db: &mut Connection, graph: &mut KnowledgeMap) -> Result<()> {
    let tx = db.transaction()?;
    let mut saved = tx
        .prepare("SELECT node_id,cluster_id,q,r FROM knowledge_position ORDER BY node_id")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Saved {
                    group: r.get(1)?,
                    position: HexPosition {
                        q: r.get(2)?,
                        r: r.get(3)?,
                    },
                },
            ))
        })?
        .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
    let active = graph
        .nodes
        .iter()
        .map(|n| n.id.clone())
        .collect::<BTreeSet<_>>();
    for node in &graph.nodes {
        if saved.contains_key(&node.id) {
            continue;
        }
        for alias in &node.aliases {
            if active.contains(alias) {
                continue;
            }
            if let Some(old) = saved.remove(alias) {
                tx.execute(
                    "UPDATE knowledge_position SET node_id=? WHERE node_id=?",
                    params![node.id, alias],
                )?;
                tx.execute(
                    "UPDATE knowledge_view SET selected_node=? WHERE selected_node=?",
                    params![node.id, alias],
                )?;
                saved.insert(node.id.clone(), old);
                break;
            }
        }
    }
    let removed = saved
        .keys()
        .filter(|id| !active.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    for id in removed {
        saved.remove(&id);
        tx.execute("DELETE FROM knowledge_position WHERE node_id=?", [id])?;
    }
    tx.execute("UPDATE knowledge_view SET selected_node=NULL WHERE selected_node IS NOT NULL AND NOT EXISTS(SELECT 1 FROM knowledge_position p WHERE p.node_id=knowledge_view.selected_node)",[])?;
    let mut occupied = BTreeMap::new();
    let mut groups = BTreeMap::<String, Cluster>::new();
    for s in saved.values() {
        occupied.insert(s.position, s.group.clone());
        groups.entry(s.group.clone()).or_default().add(s.position);
    }
    let mut neighbors = BTreeMap::<&str, BTreeSet<&str>>::new();
    for e in &graph.edges {
        neighbors.entry(&e.from).or_default().insert(&e.to);
        neighbors.entry(&e.to).or_default().insert(&e.from);
    }
    let mut order = (0..graph.nodes.len()).collect::<Vec<_>>();
    order.sort_by_key(|i| {
        let n = &graph.nodes[*i];
        (
            !matches!(n.kind, KnowledgeKind::Document | KnowledgeKind::Credential),
            n.group.clone(),
            n.id.clone(),
        )
    });
    for index in order {
        let node = &graph.nodes[index];
        if saved.contains_key(&node.id) {
            continue;
        }
        let linked = neighbors
            .get(node.id.as_str())
            .into_iter()
            .flatten()
            .filter_map(|id| saved.get(*id))
            .collect::<Vec<_>>();
        // Inherit a saved source's cluster even if virtual filing later changes.
        let group = if matches!(
            node.kind,
            KnowledgeKind::Fact | KnowledgeKind::Suggestion | KnowledgeKind::Entity
        ) {
            linked
                .first()
                .map(|s| s.group.clone())
                .unwrap_or_else(|| node.group.clone())
        } else {
            node.group.clone()
        };
        let positions = linked.iter().map(|s| s.position).collect::<Vec<_>>();
        let anchor = groups.get(&group).map(Cluster::center).unwrap_or_else(|| {
            positions
                .first()
                .copied()
                .unwrap_or_else(|| new_cluster(&occupied))
        });
        let position = candidate(&group, anchor, &positions, &occupied);
        if position.q.abs() > 1_000_000 || position.r.abs() > 1_000_000 {
            return Err(Error::Validation(
                "The knowledge map has exceeded its supported extent.",
            ));
        }
        tx.execute(
            "INSERT INTO knowledge_position(node_id,cluster_id,q,r) VALUES(?,?,?,?)",
            params![node.id, group, position.q, position.r],
        )?;
        occupied.insert(position, group.clone());
        groups.entry(group.clone()).or_default().add(position);
        saved.insert(node.id.clone(), Saved { group, position });
        graph.added += 1;
    }
    let mut labels = BTreeMap::<String, String>::new();
    for n in &graph.nodes {
        labels
            .entry(n.group.clone())
            .or_insert_with(|| n.group_label.clone());
    }
    for n in &mut graph.nodes {
        let s = &saved[&n.id];
        n.position = s.position;
        n.group = s.group.clone();
        n.group_label = labels
            .get(&n.group)
            .cloned()
            .unwrap_or_else(|| n.group_label.clone());
    }
    graph.groups = groups
        .into_iter()
        .map(|(id, g)| KnowledgeGroup {
            label: labels
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "Related details".into()),
            id,
            anchor: g.anchor,
            count: g.count as usize,
        })
        .collect();
    graph
        .groups
        .sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rings_are_complete_unique_and_at_the_requested_distance() {
        for radius in 0..20 {
            let points = ring(HexPosition { q: 17, r: -13 }, radius);
            assert_eq!(
                points.len(),
                if radius == 0 { 1 } else { radius as usize * 6 }
            );
            assert_eq!(points.len(), points.iter().collect::<BTreeSet<_>>().len());
            assert!(
                points
                    .iter()
                    .all(|p| p.distance(HexPosition { q: 17, r: -13 }) == i64::from(radius))
            );
        }
    }
}
