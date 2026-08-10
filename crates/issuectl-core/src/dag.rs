//! Scheduling-DAG view (`issuectl dag`).
//!
//! Joins the per-issue `lane` / `collision` scheduling fields with the
//! existing `blocked_by` dependency graph and live `status`, and computes
//! **on read** — nothing here is persisted — a per-lane order, a
//! head-of-line, and spawnability. issuectl stays orchestrator-agnostic:
//! the one signal it cannot know alone (which lane/collision tokens an
//! in-flight run currently holds) is supplied by the caller as
//! [`Reservations`], never read out of orchestratectl.
//!
//! ## Model
//!
//! - A **lane** is a spawn-time mutual-exclusion group: at most one issue
//!   in a lane runs at a time. Issues without a lane are independent and
//!   surface under [`DagView::unscheduled`].
//! - **Head-of-line** for a lane is the first not-done issue in the lane's
//!   deterministic order — the front of the queue.
//! - **Spawnable** = head-of-line ∧ every `blocked_by` dependency is done
//!   ∧ the issue's lane/collision tokens are not currently reserved. With
//!   no reservations supplied the reservation term is vacuously false, so
//!   an unblocked head-of-line reports spawnable.
//!
//! Determinism: every list is ordered by an explicit key so two runs over
//! the same repo produce byte-identical output (cacheable by agents).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::models::Issue;
use crate::query;
use crate::schema::{status_class, Schema, StatusClass};

/// Currently-held scheduling tokens, supplied by the caller (e.g. an
/// orchestrator that knows which lanes/collision files its in-flight runs
/// hold). A single flat set: a lane name and a collision token share one
/// "hot-file" namespace, so an issue is reserved when *any* of its own
/// lane/collision tokens appears here.
#[derive(Debug, Clone, Default)]
pub struct Reservations {
    held: BTreeSet<String>,
}

impl Reservations {
    /// Build from an explicit set of held tokens.
    pub fn from_tokens<I: IntoIterator<Item = String>>(tokens: I) -> Self {
        Reservations {
            held: tokens.into_iter().collect(),
        }
    }

    /// Parse the caller-supplied reservations JSON. Two shapes are
    /// accepted and unioned into one held-token set:
    ///
    /// - an object `{"lanes": [..], "collision": [..]}` (either key
    ///   optional), or
    /// - an array of hold objects `[{"run_id"?, "lane"?, "collision"?:[..]}, ..]`.
    ///
    /// Strict per the AI-first contract: an unrecognised top-level shape,
    /// a non-string token, or a wrong-typed field is an error rather than
    /// a silent drop, so the caller can fix its output and retry.
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let mut held = BTreeSet::new();
        match v {
            serde_json::Value::Object(map) => {
                collect_token_array(map.get("lanes"), "lanes", &mut held)?;
                collect_token_array(map.get("collision"), "collision", &mut held)?;
                // A single `lane: "x"` scalar is tolerated for convenience.
                if let Some(lane) = map.get("lane") {
                    collect_scalar_or_array(lane, "lane", &mut held)?;
                }
            }
            serde_json::Value::Array(holds) => {
                for (i, hold) in holds.iter().enumerate() {
                    let obj = hold.as_object().ok_or_else(|| {
                        format!("reservations[{i}] must be an object, got {hold}")
                    })?;
                    if let Some(lane) = obj.get("lane") {
                        collect_scalar_or_array(lane, "lane", &mut held)?;
                    }
                    collect_token_array(obj.get("collision"), "collision", &mut held)?;
                    // `lanes` is also accepted inside a hold for symmetry.
                    collect_token_array(obj.get("lanes"), "lanes", &mut held)?;
                }
            }
            other => {
                return Err(format!(
                    "reservations must be an object or an array of holds, got {other}"
                ));
            }
        }
        Ok(Reservations { held })
    }

    /// True when the issue's own lane or any of its collision tokens is
    /// currently held.
    fn reserves(&self, lane: Option<&str>, collision: &[String]) -> bool {
        if self.held.is_empty() {
            return false;
        }
        if let Some(l) = lane {
            if self.held.contains(l) {
                return true;
            }
        }
        collision.iter().any(|c| self.held.contains(c))
    }
}

fn collect_scalar_or_array(
    v: &serde_json::Value,
    field: &str,
    out: &mut BTreeSet<String>,
) -> Result<(), String> {
    match v {
        serde_json::Value::String(s) => {
            push_token(s, field, out)?;
            Ok(())
        }
        serde_json::Value::Array(_) => collect_token_array(Some(v), field, out),
        serde_json::Value::Null => Ok(()),
        other => Err(format!(
            "reservations `{field}` must be a string or array of strings, got {other}"
        )),
    }
}

fn collect_token_array(
    v: Option<&serde_json::Value>,
    field: &str,
    out: &mut BTreeSet<String>,
) -> Result<(), String> {
    match v {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                match item {
                    serde_json::Value::String(s) => push_token(s, field, out)?,
                    other => {
                        return Err(format!(
                            "reservations `{field}` element must be a string, got {other}"
                        ));
                    }
                }
            }
            Ok(())
        }
        Some(other) => Err(format!(
            "reservations `{field}` must be an array of strings, got {other}"
        )),
    }
}

fn push_token(s: &str, field: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("reservations `{field}` contains an empty token"));
    }
    out.insert(t.to_string());
    Ok(())
}

/// One issue as rendered in the DAG view. Reuses the shared field
/// vocabulary (`slug`, `title`, `status`, `priority`).
#[derive(Debug, Clone, Serialize)]
pub struct DagIssue {
    pub slug: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    /// Zero-based position in the lane's (or unscheduled bucket's) order.
    pub position: usize,
    /// Canonical `blocked_by` projection (sorted, deduped, bare slugs).
    pub blocked_by: Vec<String>,
    /// Subset of `blocked_by` that is not yet done (still gating).
    pub blockers_open: Vec<String>,
    pub is_head_of_line: bool,
    pub spawnable: bool,
    pub reserved: bool,
    /// Echoed scheduling fields (null when unset) so a single row is
    /// self-describing.
    pub lane: Option<String>,
    pub collision: Vec<String>,
}

/// One lane: its ordered issues and the computed head-of-line slug.
#[derive(Debug, Clone, Serialize)]
pub struct DagLane {
    pub lane: String,
    /// First not-done issue in `issues` order, or null when the lane is
    /// fully done.
    pub head_of_line: Option<String>,
    pub issues: Vec<DagIssue>,
}

/// The full scheduling-DAG view.
#[derive(Debug, Clone, Serialize)]
pub struct DagView {
    /// On-disk schema version surfaced per the AI-first contract.
    pub schema_version: u32,
    /// Whether a caller-supplied reservations set was applied.
    pub reservations_applied: bool,
    /// Lanes, ordered by lane name.
    pub lanes: Vec<DagLane>,
    /// Issues without a lane, each independent.
    pub unscheduled: Vec<DagIssue>,
}

/// Priority rank for ordering: high sorts before normal before low, and
/// any unknown priority sorts last. Lower number = earlier.
fn priority_rank(p: &str) -> u8 {
    match p {
        "high" => 0,
        "normal" => 1,
        "low" => 2,
        _ => 3,
    }
}

/// Compute the scheduling-DAG view over `issues`, classifying done-ness
/// via `schema` (so a project's `status_classes` override is honoured)
/// and applying optional caller `reservations`.
pub fn compute(issues: &[Issue], schema: &Schema, reservations: Option<&Reservations>) -> DagView {
    // done(slug) — a dependency is "satisfied" once its issue is closing.
    let done: BTreeSet<&str> = issues
        .iter()
        .filter(|i| status_class(schema, &i.status) == StatusClass::Closing)
        .map(|i| i.slug.as_str())
        .collect();

    // blocked_by graph (sorted/deduped/bare-slug) via the shared helper.
    let graph = query::build_blocked_by_graph(issues);
    let empty: Vec<String> = Vec::new();

    // Partition by lane. Issues without a lane go to the unscheduled bucket.
    let mut by_lane: BTreeMap<&str, Vec<&Issue>> = BTreeMap::new();
    let mut unscheduled: Vec<&Issue> = Vec::new();
    for i in issues {
        match i.lane.as_deref() {
            Some(lane) => by_lane.entry(lane).or_default().push(i),
            None => unscheduled.push(i),
        }
    }

    let lanes = by_lane
        .into_iter()
        .map(|(lane, members)| {
            let ordered = order_lane(&members, &graph);
            build_lane(lane, &ordered, &graph, &empty, &done, reservations)
        })
        .collect();

    // Unscheduled issues are independent; order them by the same tiebreak
    // (no intra-lane topo needed) and render each as its own head-of-line
    // when not done.
    let ordered_unscheduled = tiebreak_sorted(&unscheduled);
    let unscheduled = ordered_unscheduled
        .iter()
        .enumerate()
        .map(|(pos, i)| {
            let blocked_by = graph.get(&i.slug).unwrap_or(&empty).clone();
            let blockers_open = open_blockers(&blocked_by, &done);
            let collision = i.collision.clone().unwrap_or_default();
            let reserved = reservations
                .map(|r| r.reserves(i.lane.as_deref(), &collision))
                .unwrap_or(false);
            // An unscheduled issue has no lane queue in front of it, so it
            // is its own head-of-line whenever it is not done.
            let is_head = !done.contains(i.slug.as_str());
            DagIssue {
                slug: i.slug.clone(),
                title: i.title.clone(),
                status: i.status.clone(),
                priority: i.priority.clone(),
                position: pos,
                spawnable: is_head && blockers_open.is_empty() && !reserved,
                is_head_of_line: is_head,
                blocked_by,
                blockers_open,
                reserved,
                lane: None,
                collision,
            }
        })
        .collect();

    DagView {
        schema_version: crate::schema::SUPPORTED_SCHEMA_VERSION,
        reservations_applied: reservations.is_some(),
        lanes,
        unscheduled,
    }
}

/// Subset of `blocked_by` whose target is not done (or does not exist).
fn open_blockers(blocked_by: &[String], done: &BTreeSet<&str>) -> Vec<String> {
    blocked_by
        .iter()
        .filter(|b| !done.contains(b.as_str()))
        .cloned()
        .collect()
}

/// Deterministic tiebreak order: priority (high→low), then `created`
/// ascending (missing dates last), then slug. Stable.
fn tiebreak_sorted<'a>(members: &[&'a Issue]) -> Vec<&'a Issue> {
    let mut v = members.to_vec();
    v.sort_by(|a, b| {
        priority_rank(&a.priority)
            .cmp(&priority_rank(&b.priority))
            .then_with(|| a.created.is_none().cmp(&b.created.is_none()))
            .then_with(|| a.created.cmp(&b.created))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    v
}

/// Order a lane's issues so that any *intra-lane* `blocked_by` dependency
/// precedes its dependent, tie-broken by [`tiebreak_sorted`]. Kahn's
/// algorithm with the tiebreak as the ready-set selection key; a cycle
/// (already flagged by `doctor`) degrades gracefully — leftover nodes are
/// appended in tiebreak order, so the render never panics or loops.
fn order_lane<'a>(members: &[&'a Issue], graph: &BTreeMap<String, Vec<String>>) -> Vec<&'a Issue> {
    let in_lane: BTreeSet<&str> = members.iter().map(|i| i.slug.as_str()).collect();
    let base = tiebreak_sorted(members);

    // Intra-lane dependency count per slug (only edges to same-lane nodes
    // constrain ordering; cross-lane blockers gate spawnability, not order).
    let mut indegree: BTreeMap<String, usize> =
        base.iter().map(|i| (i.slug.clone(), 0usize)).collect();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for i in &base {
        if let Some(deps) = graph.get(&i.slug) {
            for dep in deps {
                if in_lane.contains(dep.as_str()) && dep != &i.slug {
                    *indegree.get_mut(&i.slug).unwrap() += 1;
                    dependents
                        .entry(dep.clone())
                        .or_default()
                        .push(i.slug.clone());
                }
            }
        }
    }

    // `base` is already in tiebreak order; walking it to pick the first
    // ready (indegree 0) node preserves the tiebreak among ready nodes.
    let mut result: Vec<&'a Issue> = Vec::with_capacity(base.len());
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    let by_slug: BTreeMap<&str, &'a Issue> = base.iter().map(|i| (i.slug.as_str(), *i)).collect();

    loop {
        let mut progressed = false;
        for i in &base {
            let slug = &i.slug;
            if emitted.contains(slug) {
                continue;
            }
            if indegree.get(slug).copied().unwrap_or(0) == 0 {
                result.push(by_slug[slug.as_str()]);
                emitted.insert(slug.clone());
                progressed = true;
                if let Some(deps) = dependents.get(slug) {
                    for d in deps.clone() {
                        if let Some(c) = indegree.get_mut(&d) {
                            *c = c.saturating_sub(1);
                        }
                    }
                }
            }
        }
        if !progressed {
            break;
        }
    }
    // Cycle fallback: append any not-yet-emitted node in tiebreak order.
    for i in &base {
        if !emitted.contains(&i.slug) {
            result.push(*i);
        }
    }
    result
}

fn build_lane(
    lane: &str,
    ordered: &[&Issue],
    graph: &BTreeMap<String, Vec<String>>,
    empty: &Vec<String>,
    done: &BTreeSet<&str>,
    reservations: Option<&Reservations>,
) -> DagLane {
    // Head-of-line = first not-done issue in order (front of the queue).
    let head = ordered
        .iter()
        .find(|i| !done.contains(i.slug.as_str()))
        .map(|i| i.slug.clone());

    let issues = ordered
        .iter()
        .enumerate()
        .map(|(pos, i)| {
            let blocked_by = graph.get(&i.slug).unwrap_or(empty).clone();
            let blockers_open = open_blockers(&blocked_by, done);
            let collision = i.collision.clone().unwrap_or_default();
            let reserved = reservations
                .map(|r| r.reserves(Some(lane), &collision))
                .unwrap_or(false);
            let is_head = head.as_deref() == Some(i.slug.as_str());
            DagIssue {
                slug: i.slug.clone(),
                title: i.title.clone(),
                status: i.status.clone(),
                priority: i.priority.clone(),
                position: pos,
                spawnable: is_head && blockers_open.is_empty() && !reserved,
                is_head_of_line: is_head,
                blocked_by,
                blockers_open,
                reserved,
                lane: Some(lane.to_string()),
                collision,
            }
        })
        .collect();

    DagLane {
        lane: lane.to_string(),
        head_of_line: head,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::default_schema;

    fn mk(slug: &str, status: &str, priority: &str) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: status.to_string(),
            created: Some("2026-01-01".to_string()),
            status: status.to_string(),
            updated: None,
            priority: priority.to_string(),
            issue_type: "task".to_string(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: None,
            closed: None,
            closed_by: None,
            lane: None,
            collision: None,
            commits: None,
            title: format!("Title {slug}"),
            body: String::new(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn with_lane(mut i: Issue, lane: &str) -> Issue {
        i.lane = Some(lane.to_string());
        i
    }

    fn with_blocked_by(mut i: Issue, deps: &[&str]) -> Issue {
        i.extra.insert(
            "blocked_by".into(),
            serde_json::json!(deps.iter().collect::<Vec<_>>()),
        );
        i
    }

    fn lane<'a>(v: &'a DagView, name: &str) -> &'a DagLane {
        v.lanes
            .iter()
            .find(|l| l.lane == name)
            .expect("lane present")
    }

    #[test]
    fn groups_by_lane_and_unscheduled() {
        let issues = vec![
            with_lane(mk("a-one", "open", "normal"), "schema"),
            with_lane(mk("b-two", "open", "normal"), "schema"),
            mk("c-loose", "open", "normal"),
        ];
        let v = compute(&issues, &default_schema(), None);
        assert_eq!(v.lanes.len(), 1);
        assert_eq!(lane(&v, "schema").issues.len(), 2);
        assert_eq!(v.unscheduled.len(), 1);
        assert_eq!(v.unscheduled[0].slug, "c-loose");
        assert_eq!(v.schema_version, crate::schema::SUPPORTED_SCHEMA_VERSION);
        assert!(!v.reservations_applied);
    }

    #[test]
    fn intra_lane_dependency_orders_before_priority_tiebreak() {
        // b depends on a; even though a is `normal` and b is `high`, a
        // must come first because the dependency edge outranks priority.
        let issues = vec![
            with_lane(mk("a-first", "open", "normal"), "schema"),
            with_lane(
                with_blocked_by(mk("b-second", "open", "high"), &["a-first"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "schema")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["a-first", "b-second"]);
    }

    #[test]
    fn head_of_line_is_first_not_done_and_spawnable_when_unblocked() {
        let issues = vec![
            with_lane(mk("a-done", "done", "normal"), "schema"),
            with_lane(mk("b-open", "open", "normal"), "schema"),
            with_lane(mk("c-open", "open", "normal"), "schema"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "schema");
        assert_eq!(l.head_of_line.as_deref(), Some("b-open"));
        let b = &l.issues[1];
        assert_eq!(b.slug, "b-open");
        assert!(b.is_head_of_line && b.spawnable);
        // c is behind the head in the same lane → not head, not spawnable.
        let c = &l.issues[2];
        assert!(!c.is_head_of_line && !c.spawnable);
    }

    #[test]
    fn head_with_open_blocker_is_head_but_not_spawnable() {
        // The head-of-line has a cross-lane blocker that is still open.
        let issues = vec![
            mk("dep-x", "open", "normal"), // unscheduled blocker, still open
            with_lane(
                with_blocked_by(mk("a-head", "open", "normal"), &["dep-x"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "schema");
        let a = &l.issues[0];
        assert!(a.is_head_of_line, "front of queue");
        assert_eq!(a.blockers_open, vec!["dep-x".to_string()]);
        assert!(!a.spawnable, "open blocker blocks spawn");
    }

    #[test]
    fn done_blocker_is_satisfied() {
        let issues = vec![
            mk("dep-done", "fixed", "normal"),
            with_lane(
                with_blocked_by(mk("a-head", "open", "normal"), &["dep-done"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        let a = &lane(&v, "schema").issues[0];
        assert!(a.blockers_open.is_empty());
        assert!(a.spawnable);
    }

    #[test]
    fn reservations_block_spawn_by_lane_or_collision() {
        let mut b = with_lane(mk("b-head", "open", "normal"), "main-rs");
        b.collision = Some(vec!["shared.rs".to_string()]);
        let issues = vec![with_lane(mk("a-head", "open", "normal"), "schema"), b];

        // Hold the schema lane and the shared.rs collision token.
        let res = Reservations::from_tokens(["schema".to_string(), "shared.rs".to_string()]);
        let v = compute(&issues, &default_schema(), Some(&res));
        assert!(v.reservations_applied);
        let a = &lane(&v, "schema").issues[0];
        assert!(a.reserved && !a.spawnable, "lane token reserved");
        let bb = &lane(&v, "main-rs").issues[0];
        assert!(bb.reserved && !bb.spawnable, "collision token reserved");
    }

    #[test]
    fn reservations_from_json_object_and_array_shapes() {
        let obj = serde_json::json!({"lanes": ["schema"], "collision": ["a.rs"]});
        let r = Reservations::from_json(&obj).unwrap();
        assert!(r.reserves(Some("schema"), &[]));
        assert!(r.reserves(None, &["a.rs".to_string()]));
        assert!(!r.reserves(Some("other"), &["b.rs".to_string()]));

        let arr = serde_json::json!([
            {"run_id": "r1", "lane": "main-rs", "collision": ["x.rs"]},
            {"lane": "schema"}
        ]);
        let r = Reservations::from_json(&arr).unwrap();
        assert!(r.reserves(Some("main-rs"), &[]));
        assert!(r.reserves(Some("schema"), &[]));
        assert!(r.reserves(None, &["x.rs".to_string()]));
    }

    #[test]
    fn reservations_from_json_rejects_bad_shapes() {
        assert!(Reservations::from_json(&serde_json::json!("nope")).is_err());
        assert!(Reservations::from_json(&serde_json::json!({"lanes": [42]})).is_err());
        assert!(Reservations::from_json(&serde_json::json!({"lanes": ["  "]})).is_err());
        assert!(Reservations::from_json(&serde_json::json!([1, 2])).is_err());
    }

    #[test]
    fn empty_reservations_never_reserve() {
        let r = Reservations::default();
        assert!(!r.reserves(Some("schema"), &["a.rs".to_string()]));
    }

    #[test]
    fn output_is_deterministic() {
        let issues = vec![
            with_lane(mk("z-last", "open", "normal"), "schema"),
            with_lane(mk("a-first", "open", "normal"), "schema"),
            mk("m-loose", "open", "normal"),
        ];
        let a = compute(&issues, &default_schema(), None);
        let b = compute(&issues, &default_schema(), None);
        let ja = serde_json::to_string(&a).unwrap();
        let jb = serde_json::to_string(&b).unwrap();
        assert_eq!(ja, jb);
        // Lanes sorted by name; within a lane, tiebreak by slug here.
        let order: Vec<&str> = lane(&a, "schema")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["a-first", "z-last"]);
    }

    #[test]
    fn cycle_does_not_hang_and_renders_all() {
        // a↔b mutual block within a lane: order_lane must not loop and
        // must still emit both nodes (doctor is what flags the cycle).
        let issues = vec![
            with_lane(
                with_blocked_by(mk("a-cyc", "open", "normal"), &["b-cyc"]),
                "schema",
            ),
            with_lane(
                with_blocked_by(mk("b-cyc", "open", "normal"), &["a-cyc"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        assert_eq!(lane(&v, "schema").issues.len(), 2);
    }
}
