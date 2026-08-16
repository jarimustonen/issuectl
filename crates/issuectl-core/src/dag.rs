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
//!   surface under [`DagView::unscheduled`]. The reserved lane value
//!   [`UNLANED`] (`lane: unlaned`) is a *first-class parallel-safe marker*,
//!   the opposite of a normal shared lane: its members are treated as
//!   independent (they surface under [`DagView::unscheduled`] and are each
//!   their own head-of-line), so two `lane: unlaned` issues are both
//!   spawnable at once rather than serialized. It is distinct from an
//!   **absent** lane, which means "unclassified" — the row still echoes
//!   `lane: "unlaned"` so a caller can tell "confirmed parallel-safe" from
//!   "nobody has laned this yet".
//! - **Intra-lane order** is topological on `blocked_by`, then priority,
//!   then the optional coarse key [`Issue::lane_seq`], then `created`, then
//!   the slug lexical tie-break. `lane_seq` lets a human pin soft
//!   precedence ("throughput item before hardening item") without
//!   fabricating a `blocked_by` edge; absent → today's behaviour. Priority
//!   still dominates `lane_seq` (it is a tie-break within a priority band,
//!   not an override of it). The same key also settles the *presentation*
//!   order of the independent `unscheduled` bucket (absent-lane and
//!   `unlaned` rows) — harmless there since each such issue is its own
//!   head, but it keeps a one-at-a-time consumer deterministic.
//! - **Head-of-line** for a lane is the first not-done issue in the lane's
//!   deterministic order whose `blocked_by` dependencies are all done —
//!   i.e. the front *runnable* issue. This is **work-conserving**: if the
//!   earliest not-done issue is stuck behind an open (cross-lane) blocker,
//!   the head advances to the next runnable member rather than stalling
//!   the whole lane. `None` when the lane has no runnable issue (all done,
//!   or every not-done issue still has an open blocker).
//! - **Spawnable** = the issue is its lane's head-of-line ∧ its
//!   lane/collision tokens are not currently reserved. (Head-of-line
//!   already implies "not done" and "all blockers done".) `in-progress`
//!   is deliberately **not** excluded: an in-progress issue means
//!   *started, not done* — not "someone is on it right now". `dag` is
//!   intended to be consulted only when nothing is actively running
//!   ("what's next?"); under that caller precondition an in-progress issue
//!   is one nobody is currently working — a half-done, idle, *resumable*
//!   candidate that should surface rather than be hidden. This is a caller
//!   precondition, not a property this computation can verify: preventing
//!   two workers on the same issue is the **caller's** reservation/claim
//!   responsibility (see the TOCTOU caveat below), not the dag's. With no
//!   reservations supplied the reservation term is vacuously false, so a
//!   runnable head-of-line — in-progress or not — reports spawnable.
//!
//! **Contract caveat (TOCTOU).** `spawnable` is *per-issue eligibility
//! against the supplied reservation snapshot*, not a jointly-safe set:
//! two head-of-line issues in different lanes that share a collision
//! token can both read `spawnable: true` at once. The caller must claim
//! the lane/collision tokens **atomically** as it spawns (and feed the
//! new holds back via `--reservations`); a read-then-spawn without an
//! atomic claim races. issuectl computes eligibility; it does not
//! arbitrate concurrent spawns.
//!
//! Determinism: every list is ordered by an explicit key so two runs over
//! the same repo produce byte-identical output (cacheable by agents).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::models::Issue;
use crate::query;
use crate::schema::{status_class, Schema, StatusClass};

/// Reserved `lane` value marking an issue as **confirmed parallel-safe**:
/// independently spawnable and never serialized with siblings that share
/// it (the opposite of a normal shared lane). Distinct from an absent
/// lane, which means "unclassified".
pub const UNLANED: &str = "unlaned";

/// Currently-held scheduling tokens, supplied by the caller (e.g. an
/// orchestrator that knows which lanes/collision files its in-flight runs
/// hold). Lane names and collision tokens are kept in **separate**
/// namespaces — matching the JSON input shape — so a lane named `x` never
/// accidentally reserves an unrelated collision token also named `x`. An
/// issue is reserved when its lane is a held lane OR any of its collision
/// tokens is a held collision token.
#[derive(Debug, Clone, Default)]
pub struct Reservations {
    held_lanes: BTreeSet<String>,
    held_collisions: BTreeSet<String>,
}

/// Keys accepted in a reservation hold object, used for BOTH shapes: the
/// top-level object (a bare single hold) and each element of the
/// array-of-holds. `run_id` is accepted as an opaque tracking hint — it is
/// not type-checked or retained; the remaining keys carry the lane and
/// collision tokens. A single constant so the two shapes cannot drift apart
/// (an earlier split let the object shape reject `run_id` the array accepted).
const RESERVATION_KEYS: &[&str] = &["run_id", "lanes", "lane", "collision"];

impl Reservations {
    /// Build from an explicit set of held lane names (collision set empty).
    /// Test/ergonomic constructor.
    pub fn from_tokens<I: IntoIterator<Item = String>>(lanes: I) -> Self {
        Reservations {
            held_lanes: lanes.into_iter().collect(),
            held_collisions: BTreeSet::new(),
        }
    }

    /// Build from explicit held lane names and collision tokens.
    pub fn from_lanes_collisions<L, C>(lanes: L, collisions: C) -> Self
    where
        L: IntoIterator<Item = String>,
        C: IntoIterator<Item = String>,
    {
        Reservations {
            held_lanes: lanes.into_iter().collect(),
            held_collisions: collisions.into_iter().collect(),
        }
    }

    /// Parse the caller-supplied reservations JSON. Two shapes are
    /// accepted:
    ///
    /// - an object `{"run_id"?, "lanes"?, "lane"?, "collision"?}` (every key
    ///   optional; `lane` accepts a scalar or an array) — a bare single hold
    ///   behaves like a one-element array, or
    /// - an array of hold objects `[{"run_id"?, "lanes"?, "lane"?, "collision"?:[..]}, ..]`.
    ///
    /// Both shapes accept the same keys (see [`RESERVATION_KEYS`]); `run_id`
    /// is an opaque tracking hint, accepted but not type-checked or retained.
    ///
    /// Strict per the AI-first contract: an unrecognised top-level shape,
    /// an **unknown key** (a typo like `collisions` that would silently
    /// disable exclusion), a non-string token, an empty token, or a
    /// wrong-typed field is an error rather than a silent drop — so the
    /// caller can fix its output and retry.
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let mut lanes = BTreeSet::new();
        let mut collisions = BTreeSet::new();
        match v {
            serde_json::Value::Object(map) => {
                reject_unknown_keys(map, RESERVATION_KEYS, "reservations")?;
                collect_token_array(map.get("lanes"), "lanes", &mut lanes)?;
                if let Some(lane) = map.get("lane") {
                    collect_scalar_or_array(lane, "lane", &mut lanes)?;
                }
                collect_token_array(map.get("collision"), "collision", &mut collisions)?;
            }
            serde_json::Value::Array(holds) => {
                for (i, hold) in holds.iter().enumerate() {
                    let obj = hold.as_object().ok_or_else(|| {
                        format!("reservations[{i}] must be an object, got {hold}")
                    })?;
                    let ctx = format!("reservations[{i}]");
                    reject_unknown_keys(obj, RESERVATION_KEYS, &ctx)?;
                    collect_token_array(obj.get("lanes"), "lanes", &mut lanes)?;
                    if let Some(lane) = obj.get("lane") {
                        collect_scalar_or_array(lane, "lane", &mut lanes)?;
                    }
                    collect_token_array(obj.get("collision"), "collision", &mut collisions)?;
                }
            }
            other => {
                return Err(format!(
                    "reservations must be an object or an array of holds, got {other}"
                ));
            }
        }
        Ok(Reservations {
            held_lanes: lanes,
            held_collisions: collisions,
        })
    }

    /// True when the issue's own lane is a held lane, or any of its
    /// collision tokens is a held collision token.
    fn reserves(&self, lane: Option<&str>, collision: &[String]) -> bool {
        if let Some(l) = lane {
            if self.held_lanes.contains(l) {
                return true;
            }
        }
        collision.iter().any(|c| self.held_collisions.contains(c))
    }
}

fn reject_unknown_keys(
    map: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    ctx: &str,
) -> Result<(), String> {
    for k in map.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!(
                "{ctx}: unknown key {k:?} (allowed: {})",
                allowed.join(", ")
            ));
        }
    }
    Ok(())
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
    /// Subset of `blocked_by` that is not yet done and refers to an issue
    /// that exists in the repo (a real, still-gating dependency).
    pub blockers_open: Vec<String>,
    /// Subset of `blocked_by` whose target slug does not exist in the repo
    /// — a dangling reference that can never become done (repair the data
    /// or run `doctor`). Kept distinct from `blockers_open` so a caller can
    /// tell "waiting on real work" from "your graph is broken".
    pub blockers_missing: Vec<String>,
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
    /// Number of live issues in this serial lane. This is its scheduling
    /// depth: only one member, the head-of-line, can be spawnable at once.
    pub depth: usize,
    /// First *runnable* issue in `issues` order — not done and with every
    /// `blocked_by` dependency satisfied — or null when no member is
    /// currently runnable (all done, or every not-done member is still
    /// blocked). This is the work-conserving head, not merely the first
    /// not-done issue: a stuck front member is skipped.
    pub head_of_line: Option<String>,
    /// The lane's issues in scheduling order. Terminal (closing-status)
    /// members are excluded — a fully-done lane produces no `DagLane` at
    /// all, so every rendered row is live scheduling work.
    pub issues: Vec<DagIssue>,
}

/// The full scheduling-DAG view.
#[derive(Debug, Clone, Serialize)]
pub struct DagView {
    /// On-disk schema version of the loaded repo schema, surfaced per the
    /// AI-first contract.
    pub schema_version: u32,
    /// Number of rows currently eligible to spawn. This includes ordinary
    /// lane heads and independently-headed unscheduled / `unlaned` issues,
    /// and is derived from the computed `DagIssue::spawnable` predicates.
    pub spawnable_heads: usize,
    /// Whether a caller-supplied reservations set was applied.
    pub reservations_applied: bool,
    /// Lanes, ordered by lane name. Terminal (closing-status) issues are
    /// excluded from every lane (see [`DagLane::issues`]).
    pub lanes: Vec<DagLane>,
    /// Issues without a lane, each independent. Terminal (closing-status)
    /// issues are excluded — this is a scheduling view, and a closed issue
    /// can never be scheduled.
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
    // Universe of real slugs, for distinguishing a dangling `blocked_by`
    // ref (missing) from a real still-open dependency.
    let all_slugs: BTreeSet<&str> = issues.iter().map(|i| i.slug.as_str()).collect();

    // Partition by lane. Issues without a lane — and issues carrying the
    // parallel-safe `unlaned` sentinel — go to the unscheduled bucket,
    // where each is independent (its own head-of-line) rather than
    // serialized. The sentinel differs from an absent lane only in what
    // the row echoes (`lane: "unlaned"` vs `null`), so a caller can tell
    // "confirmed parallel-safe" from "unclassified".
    //
    // Terminal (closing-status) issues are excluded from the scheduling
    // view entirely — lanes and unscheduled alike. `dag` is a *scheduling*
    // view, and a done/wontfix/obsolete issue can never be scheduled, so
    // rendering it is noise that has misled readers into treating
    // shipped-and-closed work as open backlog. Excluding them everywhere
    // (rather than only from the unscheduled bucket) also keeps completed
    // work out of `order_lane`'s intra-lane topological sort: a closed
    // lane member left in the roster still carries its `blocked_by` edges
    // and priority into the Kahn ordering, where it can demote a
    // higher-priority *runnable* member out of head-of-line. The closing
    // classification is the schema-aware `done` set (so a project's
    // `status_classes` override is honoured), computed above over the full
    // issue set — closing issues stay in that set (and in `graph` /
    // `all_slugs`) for blocker resolution, so a done dependency still reads
    // as satisfied; they are only dropped from the rendered rows.
    let mut by_lane: BTreeMap<&str, Vec<&Issue>> = BTreeMap::new();
    let mut unscheduled: Vec<&Issue> = Vec::new();
    for i in issues {
        if done.contains(i.slug.as_str()) {
            continue;
        }
        match i.lane.as_deref() {
            Some(UNLANED) | None => unscheduled.push(i),
            Some(lane) => by_lane.entry(lane).or_default().push(i),
        }
    }

    let ctx = ComputeCtx {
        graph: &graph,
        done: &done,
        all_slugs: &all_slugs,
        reservations,
    };

    let lanes: Vec<DagLane> = by_lane
        .into_iter()
        .map(|(lane, members)| build_lane(lane, &order_lane(&members, &graph), &ctx))
        .collect();

    // Unscheduled issues are independent (no lane mutual-exclusion); order
    // them by the same tiebreak and render each as its own head-of-line.
    // Terminal issues were already filtered out of the bucket above, so
    // every unscheduled issue is a non-done, independent head; `spawnable`
    // still requires blockers to be satisfied.
    let unscheduled: Vec<DagIssue> = tiebreak_sorted(&unscheduled)
        .iter()
        .enumerate()
        .map(|(pos, i)| ctx.make_issue(i, pos, None, true))
        .collect();

    let spawnable_heads = lanes
        .iter()
        .flat_map(|lane| &lane.issues)
        .chain(unscheduled.iter())
        .filter(|issue| issue.spawnable)
        .count();

    DagView {
        schema_version: schema.version,
        spawnable_heads,
        reservations_applied: reservations.is_some(),
        lanes,
        unscheduled,
    }
}

/// Shared, read-only inputs threaded through per-issue row construction.
struct ComputeCtx<'a> {
    graph: &'a BTreeMap<String, Vec<String>>,
    done: &'a BTreeSet<&'a str>,
    all_slugs: &'a BTreeSet<&'a str>,
    reservations: Option<&'a Reservations>,
}

impl ComputeCtx<'_> {
    /// Split an issue's `blocked_by` into (still-open real deps, dangling
    /// missing refs). A blocker is `open` when it exists and is not done;
    /// `missing` when its slug is absent from the repo.
    fn partition_blockers(&self, blocked_by: &[String]) -> (Vec<String>, Vec<String>) {
        let mut open = Vec::new();
        let mut missing = Vec::new();
        for b in blocked_by {
            if !self.all_slugs.contains(b.as_str()) {
                missing.push(b.clone());
            } else if !self.done.contains(b.as_str()) {
                open.push(b.clone());
            }
        }
        (open, missing)
    }

    /// True when an issue is runnable now: not done and every `blocked_by`
    /// dependency is satisfied (none open, none dangling).
    fn is_runnable(&self, i: &Issue) -> bool {
        if self.done.contains(i.slug.as_str()) {
            return false;
        }
        let empty = Vec::new();
        let blocked_by = self.graph.get(&i.slug).unwrap_or(&empty);
        let (open, missing) = self.partition_blockers(blocked_by);
        open.is_empty() && missing.is_empty()
    }

    /// Build one row. `is_head` is decided by the caller (lane head vs.
    /// unscheduled-independent). `res_lane` is the lane that gates
    /// *reservation* — `Some(name)` for a real lane member, `None` for an
    /// unscheduled or `unlaned` issue (which is never reserved by lane).
    /// The row still echoes the issue's *own* `lane` (so an `unlaned`
    /// sentinel surfaces), which is why the two are threaded separately.
    ///
    /// `spawnable` = head ∧ runnable ∧ not reserved. The runnable check is
    /// redundant for a lane head (which is runnable by construction) but
    /// load-bearing for unscheduled issues. `in-progress` is deliberately
    /// NOT excluded: it means *started, not done*, and `dag` is consulted
    /// only when nothing is running, so an in-progress head is a resumable
    /// candidate that must surface. Preventing a double-spawn is the
    /// caller's reservation responsibility, not this computation's.
    fn make_issue(&self, i: &Issue, pos: usize, res_lane: Option<&str>, is_head: bool) -> DagIssue {
        let empty = Vec::new();
        let blocked_by = self.graph.get(&i.slug).unwrap_or(&empty).clone();
        let (blockers_open, blockers_missing) = self.partition_blockers(&blocked_by);
        let collision = i.collision.clone().unwrap_or_default();
        let reserved = self
            .reservations
            .map(|r| r.reserves(res_lane, &collision))
            .unwrap_or(false);
        let spawnable =
            is_head && blockers_open.is_empty() && blockers_missing.is_empty() && !reserved;
        DagIssue {
            slug: i.slug.clone(),
            title: i.title.clone(),
            status: i.status.clone(),
            priority: i.priority.clone(),
            position: pos,
            spawnable,
            is_head_of_line: is_head,
            blocked_by,
            blockers_open,
            blockers_missing,
            reserved,
            lane: i.lane.clone(),
            collision,
        }
    }
}

/// Deterministic tiebreak order: priority (high→low), then the optional
/// coarse key `lane_seq` (issues that set it sort ahead of those that
/// don't, ascending among setters), then `created` ascending (missing
/// dates last), then slug. Stable. `lane_seq` sits between priority and
/// `created` so a human precedence hint overrides the incidental
/// creation-order and lexical-slug tie-breaks without displacing priority.
fn tiebreak_sorted<'a>(members: &[&'a Issue]) -> Vec<&'a Issue> {
    let mut v = members.to_vec();
    v.sort_by(|a, b| {
        priority_rank(&a.priority)
            .cmp(&priority_rank(&b.priority))
            .then_with(|| a.lane_seq.is_none().cmp(&b.lane_seq.is_none()))
            .then_with(|| a.lane_seq.unwrap_or(0).cmp(&b.lane_seq.unwrap_or(0)))
            .then_with(|| a.created.is_none().cmp(&b.created.is_none()))
            .then_with(|| a.created.cmp(&b.created))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    v
}

/// Sort key for the ready-set: tiebreak fields in comparison order, with
/// the base index as the final total-order guarantee. `BTreeSet` pops the
/// smallest key, so a newly-released high-priority node correctly overtakes
/// an already-ready lower-priority node.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct OrderKey {
    priority: u8,
    // `lane_seq` sits directly below priority: an issue that sets it sorts
    // ahead of one that doesn't (`no_lane_seq` false < true), ascending
    // among setters, so a human precedence hint beats the incidental
    // `created`/slug tie-breaks below without displacing priority. Field
    // order here IS the comparison order (derived `Ord`).
    no_lane_seq: bool,
    lane_seq: i64,
    no_created: bool,
    created: Option<String>,
    slug: String,
}

fn order_key(i: &Issue) -> OrderKey {
    OrderKey {
        priority: priority_rank(&i.priority),
        no_lane_seq: i.lane_seq.is_none(),
        lane_seq: i.lane_seq.unwrap_or(0),
        no_created: i.created.is_none(),
        created: i.created.clone(),
        slug: i.slug.clone(),
    }
}

/// Order a lane's issues so that any *intra-lane* `blocked_by` dependency
/// precedes its dependent, tie-broken by [`order_key`]. Kahn's algorithm
/// over an **ordered ready set** — one node popped at a time — so releasing
/// a dependency re-evaluates the tiebreak globally (a freshly-unblocked
/// high-priority node overtakes an already-ready low-priority node). A
/// cycle (already flagged by `doctor`) degrades gracefully: leftover nodes
/// are appended in tiebreak order, so the render never panics or loops.
fn order_lane<'a>(members: &[&'a Issue], graph: &BTreeMap<String, Vec<String>>) -> Vec<&'a Issue> {
    let by_slug: BTreeMap<&str, &'a Issue> =
        members.iter().map(|i| (i.slug.as_str(), *i)).collect();
    let in_lane: BTreeSet<&str> = by_slug.keys().copied().collect();

    // Intra-lane indegree + dependents. Edges to same-lane nodes only;
    // cross-lane blockers gate spawnability, not order. Deduped defensively
    // (the shared graph already dedupes, but ordering correctness must not
    // rely on an upstream invariant).
    let mut indegree: BTreeMap<&str, usize> = by_slug.keys().map(|s| (*s, 0usize)).collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for i in members {
        let slug = i.slug.as_str();
        if let Some(deps) = graph.get(&i.slug) {
            let mut seen = BTreeSet::new();
            for dep in deps {
                let dep = dep.as_str();
                if dep != slug && in_lane.contains(dep) && seen.insert(dep) {
                    *indegree.get_mut(slug).unwrap() += 1;
                    dependents.entry(dep).or_default().push(slug);
                }
            }
        }
    }

    // Ready set keyed by the tiebreak; pop smallest one at a time.
    let mut ready: BTreeSet<(OrderKey, &str)> = BTreeSet::new();
    for (slug, deg) in &indegree {
        if *deg == 0 {
            ready.insert((order_key(by_slug[slug]), slug));
        }
    }

    let mut result: Vec<&'a Issue> = Vec::with_capacity(members.len());
    while let Some((_, slug)) = ready.pop_first() {
        result.push(by_slug[slug]);
        if let Some(deps) = dependents.get(slug) {
            for &d in deps {
                let c = indegree.get_mut(d).unwrap();
                *c -= 1;
                if *c == 0 {
                    ready.insert((order_key(by_slug[d]), d));
                }
            }
        }
    }

    // Cycle fallback: append any not-yet-emitted node in tiebreak order.
    if result.len() < members.len() {
        let emitted_set: BTreeSet<&str> = result.iter().map(|i| i.slug.as_str()).collect();
        for i in tiebreak_sorted(members) {
            if !emitted_set.contains(i.slug.as_str()) {
                result.push(i);
            }
        }
    }
    result
}

fn build_lane(lane: &str, ordered: &[&Issue], ctx: &ComputeCtx<'_>) -> DagLane {
    // Work-conserving head-of-line: first not-done issue whose blockers are
    // all satisfied (runnable). Advances past a blocked front issue so a
    // stuck head does not starve the lane.
    let head = ordered
        .iter()
        .find(|i| ctx.is_runnable(i))
        .map(|i| i.slug.clone());

    let issues: Vec<DagIssue> = ordered
        .iter()
        .enumerate()
        .map(|(pos, i)| {
            let is_head = head.as_deref() == Some(i.slug.as_str());
            ctx.make_issue(i, pos, Some(lane), is_head)
        })
        .collect();

    DagLane {
        lane: lane.to_string(),
        depth: issues.len(),
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
            lane_seq: None,
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

    fn with_lane_seq(mut i: Issue, seq: i64) -> Issue {
        i.lane_seq = Some(seq);
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
        // The done member is excluded from the rendered lane, so only the
        // two live issues remain and `b-open` is the head.
        assert_eq!(l.issues.len(), 2, "closed member excluded");
        assert_eq!(l.head_of_line.as_deref(), Some("b-open"));
        let b = &l.issues[0];
        assert_eq!(b.slug, "b-open");
        assert!(b.is_head_of_line && b.spawnable);
        // c is behind the head in the same lane → not head, not spawnable.
        let c = &l.issues[1];
        assert_eq!(c.slug, "c-open");
        assert!(!c.is_head_of_line && !c.spawnable);
    }

    #[test]
    fn sole_blocked_issue_yields_no_head_of_line() {
        // Work-conserving: a lane whose only not-done issue is blocked has
        // no runnable head — head_of_line is None and the issue is neither
        // head nor spawnable, but its open blocker is surfaced.
        let issues = vec![
            mk("dep-x", "open", "normal"), // unscheduled blocker, still open
            with_lane(
                with_blocked_by(mk("a-head", "open", "normal"), &["dep-x"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "schema");
        assert_eq!(l.head_of_line, None);
        let a = &l.issues[0];
        assert!(!a.is_head_of_line);
        assert_eq!(a.blockers_open, vec!["dep-x".to_string()]);
        assert!(!a.spawnable, "open blocker blocks spawn");
    }

    #[test]
    fn work_conserving_head_advances_past_blocked_front() {
        // Front issue `a-front` (position 0) is blocked by a cross-lane
        // dep; `b-next` behind it is unblocked. Head-of-line must advance
        // to `b-next` instead of stalling the whole lane.
        let issues = vec![
            mk("dep-x", "open", "normal"),
            with_lane(
                with_blocked_by(mk("a-front", "open", "high"), &["dep-x"]),
                "schema",
            ),
            with_lane(mk("b-next", "open", "normal"), "schema"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "schema");
        // `a-front` is position 0 (high priority, no intra-lane dep), but
        // it is blocked, so the head advances to `b-next`.
        assert_eq!(l.issues[0].slug, "a-front");
        assert!(!l.issues[0].spawnable);
        assert_eq!(l.head_of_line.as_deref(), Some("b-next"));
        let b = l.issues.iter().find(|i| i.slug == "b-next").unwrap();
        assert!(b.is_head_of_line && b.spawnable);
    }

    #[test]
    fn newly_released_high_priority_overtakes_ready_low_priority() {
        // Kahn tie-break regression: `a-high` depends on `z-normal`;
        // `m-low` is independent. After `z-normal` is emitted, `a-high`
        // becomes ready and must overtake the already-ready `m-low`.
        let issues = vec![
            with_lane(
                with_blocked_by(mk("a-high", "open", "high"), &["z-normal"]),
                "schema",
            ),
            with_lane(mk("z-normal", "open", "normal"), "schema"),
            with_lane(mk("m-low", "open", "low"), "schema"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "schema")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["z-normal", "a-high", "m-low"]);
    }

    #[test]
    fn dangling_blocker_is_reported_missing_not_open() {
        let issues = vec![with_lane(
            with_blocked_by(mk("a-head", "open", "normal"), &["ghost-slug"]),
            "schema",
        )];
        let v = compute(&issues, &default_schema(), None);
        let a = &lane(&v, "schema").issues[0];
        assert_eq!(a.blockers_missing, vec!["ghost-slug".to_string()]);
        assert!(a.blockers_open.is_empty(), "not a real open dep");
        assert!(!a.spawnable, "dangling ref still blocks spawn");
        assert_eq!(lane(&v, "schema").head_of_line, None);
    }

    #[test]
    fn fully_done_lane_is_omitted_entirely() {
        // Every member closing → the lane is pure history with no
        // schedulable work, so it is dropped from the view rather than
        // rendered as a zombie lane full of done rows.
        let issues = vec![
            with_lane(mk("a-done", "done", "normal"), "schema"),
            with_lane(mk("b-done", "fixed", "normal"), "schema"),
        ];
        let v = compute(&issues, &default_schema(), None);
        assert!(
            v.lanes.iter().all(|l| l.lane != "schema"),
            "a fully-done lane produces no DagLane"
        );
        assert!(v.unscheduled.is_empty());
    }

    #[test]
    fn self_dependency_does_not_hang_and_blocks_spawn() {
        let issues = vec![with_lane(
            with_blocked_by(mk("a-self", "open", "normal"), &["a-self"]),
            "schema",
        )];
        let v = compute(&issues, &default_schema(), None);
        let a = &lane(&v, "schema").issues[0];
        // A self-blocker is a real (existing) open dep → blocks spawn.
        assert_eq!(a.blockers_open, vec!["a-self".to_string()]);
        assert!(!a.spawnable);
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
        let res =
            Reservations::from_lanes_collisions(["schema".to_string()], ["shared.rs".to_string()]);
        let v = compute(&issues, &default_schema(), Some(&res));
        assert!(v.reservations_applied);
        let a = &lane(&v, "schema").issues[0];
        assert!(a.reserved && !a.spawnable, "lane token reserved");
        let bb = &lane(&v, "main-rs").issues[0];
        assert!(bb.reserved && !bb.spawnable, "collision token reserved");
    }

    #[test]
    fn reservation_namespaces_are_separate() {
        // A held LANE named "shared" must NOT reserve an issue whose only
        // match is a COLLISION token "shared" (and vice versa).
        let held_lane_only =
            Reservations::from_lanes_collisions(["shared".to_string()], std::iter::empty());
        assert!(held_lane_only.reserves(Some("shared"), &[]), "lane matches");
        assert!(
            !held_lane_only.reserves(Some("other"), &["shared".to_string()]),
            "collision token must not match a held lane name"
        );

        let held_collision_only =
            Reservations::from_lanes_collisions(std::iter::empty(), ["shared".to_string()]);
        assert!(
            !held_collision_only.reserves(Some("shared"), &[]),
            "lane name must not match a held collision token"
        );
        assert!(held_collision_only.reserves(None, &["shared".to_string()]));
    }

    #[test]
    fn reservations_from_json_rejects_unknown_keys() {
        // A typo like `collisions` must error, not silently disable exclusion.
        let err = Reservations::from_json(&serde_json::json!({"collisions": ["x"]})).unwrap_err();
        assert!(err.contains("unknown key"), "got: {err}");
        assert!(Reservations::from_json(&serde_json::json!({"lanez": ["x"]})).is_err());
        assert!(
            Reservations::from_json(&serde_json::json!([{"lane": "x", "typo": 1}])).is_err(),
            "unknown key inside a hold must error"
        );
        // run_id is allowed inside a hold.
        assert!(
            Reservations::from_json(&serde_json::json!([{"run_id": "r", "lane": "x"}])).is_ok()
        );
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
    fn reservations_from_json_accepts_single_hold_object() {
        // A bare single-hold object carrying a tracking `run_id` behaves like a
        // one-element array — no wrapping `[..]` required to satisfy the parser.
        let obj = serde_json::json!({"run_id": "r1", "lane": "schema", "collision": ["a.rs"]});
        let r = Reservations::from_json(&obj).unwrap();
        assert!(r.reserves(Some("schema"), &[]));
        assert!(r.reserves(None, &["a.rs".to_string()]));
        assert!(!r.reserves(Some("other"), &["b.rs".to_string()]));

        // `run_id` is an opaque tracking hint — never collected as a lane or
        // collision token, so it must not reserve anything.
        assert!(!r.reserves(Some("r1"), &[]));
        assert!(!r.reserves(None, &["r1".to_string()]));

        // The object and array shapes agree for the same single hold.
        let arr = serde_json::json!([{"run_id": "r1", "lane": "schema", "collision": ["a.rs"]}]);
        let via_arr = Reservations::from_json(&arr).unwrap();
        assert_eq!(r.held_lanes, via_arr.held_lanes);
        assert_eq!(r.held_collisions, via_arr.held_collisions);
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

    // ── dag-lists-closed-issues (bug) ───────────────────────────────────

    #[test]
    fn closed_issue_is_excluded_from_unscheduled() {
        // Regression: `dag` is a scheduling view, but terminal-status issues
        // (done/wontfix/…) were dumped into the unscheduled bucket alongside
        // genuinely open work — misleading a reader into treating shipped,
        // closed work as open backlog. A closing-status unlaned issue must
        // not surface as unscheduled; the open one still does.
        let issues = vec![
            mk("a-shipped", "done", "normal"),
            mk("b-wontfix", "wontfix", "normal"),
            mk("c-open", "open", "normal"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let slugs: Vec<&str> = v.unscheduled.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["c-open"],
            "only the non-terminal issue is scheduled/unscheduled"
        );
    }

    #[test]
    fn closed_unlaned_sentinel_issue_is_excluded_from_unscheduled() {
        // The exclusion is by closing status, not by absent lane: a closed
        // issue carrying the `unlaned` sentinel is likewise dropped from the
        // unscheduled bucket.
        let issues = vec![
            with_lane(mk("a-done", "done", "normal"), UNLANED),
            with_lane(mk("b-open", "open", "normal"), UNLANED),
        ];
        let v = compute(&issues, &default_schema(), None);
        let slugs: Vec<&str> = v.unscheduled.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(slugs, vec!["b-open"]);
    }

    #[test]
    fn closed_lane_member_is_excluded_but_still_satisfies_intra_lane_blocker() {
        // A closed lane member is dropped from the rendered lane, yet still
        // counts as a satisfied dependency for a later member that was
        // blocked by it: the dependent is head and spawnable.
        let issues = vec![
            with_lane(mk("a-done", "done", "normal"), "schema"),
            with_lane(
                with_blocked_by(mk("b-open", "open", "normal"), &["a-done"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "schema");
        assert_eq!(l.issues.len(), 1, "closed member excluded from lane");
        assert_eq!(l.issues[0].slug, "b-open");
        assert_eq!(l.head_of_line.as_deref(), Some("b-open"));
        assert!(
            l.issues[0].blockers_open.is_empty() && l.issues[0].spawnable,
            "closed dep still counts as done"
        );
    }

    #[test]
    fn closed_lane_blocker_does_not_demote_high_priority_runnable_member() {
        // Regression: a closed lane member left in the roster carries its
        // `blocked_by` edge and priority into `order_lane`'s Kahn sort,
        // where it can push a higher-priority *runnable* member out of
        // head-of-line. `high-ready` is blocked by the closed `done-low`
        // (so it is runnable, since the blocker is done) and outranks the
        // independent `normal-ready`; it must be the head. If the closed
        // member were not excluded, the intra-lane edge would delay
        // `high-ready` behind `normal-ready`.
        let issues = vec![
            with_lane(mk("done-low", "done", "low"), "x"),
            with_lane(
                with_blocked_by(mk("high-ready", "open", "high"), &["done-low"]),
                "x",
            ),
            with_lane(mk("normal-ready", "open", "normal"), "x"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "x");
        assert_eq!(l.head_of_line.as_deref(), Some("high-ready"));
        assert_eq!(l.issues[0].slug, "high-ready", "high-ready leads the lane");
        assert!(l.issues[0].spawnable);
    }

    #[test]
    fn custom_closing_status_is_excluded_via_schema_override() {
        // The exclusion is schema-aware: a project can classify an
        // otherwise-unknown status as closing (`status_classes: { archived:
        // closing }`) and it must be dropped from the scheduling view, while
        // a status the project overrides *back* to active (`done: active`)
        // stays visible.
        let mut schema = default_schema();
        schema
            .status_classes
            .insert("archived".to_string(), StatusClass::Closing);
        schema
            .status_classes
            .insert("done".to_string(), StatusClass::Active);
        let issues = vec![
            mk("a-archived", "archived", "normal"),
            mk("b-done-but-active", "done", "normal"),
            mk("c-open", "open", "normal"),
        ];
        let v = compute(&issues, &schema, None);
        let slugs: Vec<&str> = v.unscheduled.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["b-done-but-active", "c-open"],
            "custom closing excluded; done-reclassified-active retained"
        );
    }

    #[test]
    fn empty_and_all_closed_repos_render_empty_view() {
        let empty = compute(&[], &default_schema(), None);
        assert!(empty.lanes.is_empty() && empty.unscheduled.is_empty());

        let all_closed = vec![
            mk("a-done", "done", "normal"),
            with_lane(mk("b-fixed", "fixed", "normal"), "x"),
        ];
        let v = compute(&all_closed, &default_schema(), None);
        assert!(
            v.lanes.is_empty() && v.unscheduled.is_empty(),
            "a repo of only closed issues renders an empty scheduling view"
        );
    }

    #[test]
    fn closed_unscheduled_issue_still_satisfies_a_blocker() {
        // Excluding closed issues from the unscheduled *display* must not
        // drop them from the `done` set used for blocker resolution: a lane
        // issue blocked by a closed unscheduled issue is still runnable.
        let issues = vec![
            mk("dep-done", "done", "normal"), // unscheduled + closed → hidden
            with_lane(
                with_blocked_by(mk("a-head", "open", "normal"), &["dep-done"]),
                "schema",
            ),
        ];
        let v = compute(&issues, &default_schema(), None);
        assert!(
            v.unscheduled.is_empty(),
            "the closed blocker is not shown as unscheduled"
        );
        let a = &lane(&v, "schema").issues[0];
        assert!(
            a.blockers_open.is_empty(),
            "closed dep still counts as done"
        );
        assert!(a.spawnable, "head is runnable — its blocker is satisfied");
    }

    // ── dag-inprogress-is-spawnable (design correction) ─────────────────

    #[test]
    fn depth_and_spawnable_head_count_include_in_progress_and_unlaned() {
        // Counts must derive from the rendered spawnable predicates: the
        // in-progress ordinary-lane head and both parallel-safe `unlaned`
        // rows are eligible, while the second ordinary-lane member is not.
        let issues = vec![
            with_lane(mk("a-underway", "in-progress", "normal"), "schema"),
            with_lane(mk("b-next", "open", "normal"), "schema"),
            with_lane(mk("c-safe", "open", "normal"), UNLANED),
            with_lane(mk("d-safe", "open", "normal"), UNLANED),
        ];
        let v = compute(&issues, &default_schema(), None);
        assert_eq!(lane(&v, "schema").depth, 2);
        assert_eq!(v.spawnable_heads, 3);
    }

    #[test]
    fn in_progress_head_is_spawnable() {
        // Design correction: `in-progress` means *started, not done* — not
        // "someone is on it right now". `dag` is consulted only when nothing
        // is running, so an in-progress head is an idle, resumable candidate
        // that MUST surface as spawnable. Preventing a double-spawn is the
        // caller's reservation responsibility, not the dag's.
        let issues = vec![with_lane(
            mk("a-underway", "in-progress", "normal"),
            "schema",
        )];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "schema");
        assert_eq!(l.head_of_line.as_deref(), Some("a-underway"));
        let a = &l.issues[0];
        assert!(a.is_head_of_line, "still the head-of-line");
        assert!(a.spawnable, "in-progress head is a resumable candidate");
    }

    #[test]
    fn in_progress_unscheduled_is_spawnable() {
        // Same correction on the unscheduled path: an unlaned in-progress
        // issue reports spawnable=true with no reservations supplied.
        let issues = vec![mk("a-underway", "in-progress", "normal")];
        let v = compute(&issues, &default_schema(), None);
        let a = &v.unscheduled[0];
        assert!(a.is_head_of_line);
        assert!(
            a.spawnable,
            "in-progress unscheduled issue is spawnable (resumable)"
        );
    }

    #[test]
    fn in_progress_head_keeps_following_lane_member_unspawnable() {
        // A serial lane is still a mutual-exclusion group: only the head is
        // spawnable. The head being `in-progress` no longer blocks its OWN
        // spawnability (it is resumable), but the member behind it is still
        // not head → still not spawnable. The lane's serialization is
        // unchanged; only the head's in-progress status stopped gating it.
        let issues = vec![
            with_lane(mk("a-underway", "in-progress", "normal"), "shared"),
            with_lane(mk("b-next", "open", "normal"), "shared"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "shared");
        assert_eq!(l.head_of_line.as_deref(), Some("a-underway"));
        let a = l.issues.iter().find(|i| i.slug == "a-underway").unwrap();
        let b = l.issues.iter().find(|i| i.slug == "b-next").unwrap();
        assert!(
            a.is_head_of_line && a.spawnable,
            "underway head is spawnable (resumable)"
        );
        assert!(
            !b.is_head_of_line && !b.spawnable,
            "lane serializes — non-head member must not spawn"
        );
    }

    #[test]
    fn in_progress_head_still_reserved_by_lane() {
        // Double-work prevention is the caller's job: when the caller feeds
        // back a reservation for the in-progress head's lane, it reads
        // reserved and therefore not spawnable — the mechanism that keeps a
        // second worker off it, distinct from any status-based exclusion.
        let issues = vec![with_lane(
            mk("a-underway", "in-progress", "normal"),
            "schema",
        )];
        let res = Reservations::from_tokens(["schema".to_string()]);
        let v = compute(&issues, &default_schema(), Some(&res));
        let a = &lane(&v, "schema").issues[0];
        assert!(
            a.reserved && !a.spawnable,
            "a claimed lane keeps the in-progress head off the spawnable set"
        );
    }

    // ── dag-stable-intralane-order (lane_seq) ───────────────────────────

    #[test]
    fn lane_seq_orders_before_slug_tiebreak() {
        // Two equal-priority, no-dependency lane members invert under slug
        // order; lane_seq pins the intended precedence regardless of slug.
        let issues = vec![
            with_lane_seq(
                with_lane(mk("z-throughput", "open", "normal"), "digest"),
                10,
            ),
            with_lane_seq(with_lane(mk("a-hardening", "open", "normal"), "digest"), 20),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "digest")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        // Without lane_seq, "a-hardening" sorts first (slug); the lower-seq
        // throughput item takes head instead.
        assert_eq!(order, vec!["z-throughput", "a-hardening"]);
        assert_eq!(
            lane(&v, "digest").head_of_line.as_deref(),
            Some("z-throughput")
        );
    }

    #[test]
    fn lane_seq_setter_sorts_ahead_of_unset_but_below_priority() {
        // A lane_seq setter sorts ahead of an equal-priority non-setter;
        // priority still dominates lane_seq (a high-priority issue with a
        // large lane_seq still leads).
        let issues = vec![
            with_lane(mk("a-unset", "open", "normal"), "lane"),
            with_lane_seq(with_lane(mk("z-set", "open", "normal"), "lane"), 5),
            with_lane_seq(with_lane(mk("m-high", "open", "high"), "lane"), 99),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "lane")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["m-high", "z-set", "a-unset"]);
    }

    #[test]
    fn absent_lane_seq_keeps_todays_order() {
        // No lane_seq anywhere ⇒ unchanged behaviour (priority, then slug).
        let issues = vec![
            with_lane(mk("b-two", "open", "normal"), "lane"),
            with_lane(mk("a-one", "open", "normal"), "lane"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "lane")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["a-one", "b-two"]);
    }

    #[test]
    fn negative_lane_seq_sorts_ahead() {
        // `lane_seq` is a signed key: a negative value sorts ahead of a
        // positive one (lower = earlier), giving a way to pin something
        // before the zero/positive band.
        let issues = vec![
            with_lane_seq(with_lane(mk("a-pos", "open", "normal"), "lane"), 5),
            with_lane_seq(with_lane(mk("b-neg", "open", "normal"), "lane"), -5),
            with_lane_seq(with_lane(mk("c-zero", "open", "normal"), "lane"), 0),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "lane")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["b-neg", "c-zero", "a-pos"]);
    }

    #[test]
    fn lane_seq_does_not_override_blocked_by() {
        // Dependency edges outrank lane_seq: even with a lower lane_seq on
        // the dependent, its blocker must still precede it.
        let issues = vec![
            with_lane_seq(
                with_lane(
                    with_blocked_by(mk("a-dependent", "open", "normal"), &["b-blocker"]),
                    "lane",
                ),
                1,
            ),
            with_lane_seq(with_lane(mk("b-blocker", "open", "normal"), "lane"), 99),
        ];
        let v = compute(&issues, &default_schema(), None);
        let order: Vec<&str> = lane(&v, "lane")
            .issues
            .iter()
            .map(|i| i.slug.as_str())
            .collect();
        assert_eq!(order, vec!["b-blocker", "a-dependent"]);
    }

    // ── dag-unlaned-parallel-sentinel ───────────────────────────────────

    #[test]
    fn unlaned_sentinel_members_are_parallel_spawnable() {
        // Two `lane: unlaned` issues are both independently spawnable — not
        // serialized like a normal shared lane. They surface as unscheduled
        // but echo lane: "unlaned" (distinct from an absent lane).
        let issues = vec![
            with_lane(mk("a-par", "open", "normal"), UNLANED),
            with_lane(mk("b-par", "open", "normal"), UNLANED),
        ];
        let v = compute(&issues, &default_schema(), None);
        assert!(
            v.lanes.iter().all(|l| l.lane != UNLANED),
            "the sentinel must not create a serial lane"
        );
        assert_eq!(v.unscheduled.len(), 2);
        for i in &v.unscheduled {
            assert!(
                i.is_head_of_line && i.spawnable,
                "{} should be parallel-spawnable",
                i.slug
            );
            assert_eq!(i.lane.as_deref(), Some(UNLANED), "row echoes the sentinel");
        }
    }

    #[test]
    fn normal_shared_lane_still_serializes() {
        // Contrast with unlaned: a normal shared lane serializes — only the
        // head is spawnable.
        let issues = vec![
            with_lane(mk("a-one", "open", "normal"), "shared"),
            with_lane(mk("b-two", "open", "normal"), "shared"),
        ];
        let v = compute(&issues, &default_schema(), None);
        let l = lane(&v, "shared");
        assert_eq!(
            l.issues.iter().filter(|i| i.spawnable).count(),
            1,
            "only the head of a normal shared lane spawns"
        );
    }

    #[test]
    fn absent_lane_is_unclassified_not_unlaned() {
        // An absent lane still means "unclassified": the row echoes
        // lane: null, distinct from the confirmed-parallel sentinel.
        let issues = vec![mk("a-loose", "open", "normal")];
        let v = compute(&issues, &default_schema(), None);
        assert_eq!(v.unscheduled[0].lane, None);
    }

    #[test]
    fn unlaned_collision_still_reserves() {
        // The parallel-safe sentinel does not exempt collision tokens: a
        // held collision token still blocks an unlaned issue's spawn.
        let mut a = with_lane(mk("a-par", "open", "normal"), UNLANED);
        a.collision = Some(vec!["shared.rs".to_string()]);
        let res =
            Reservations::from_lanes_collisions(std::iter::empty(), ["shared.rs".to_string()]);
        let v = compute(&[a], &default_schema(), Some(&res));
        let a = &v.unscheduled[0];
        assert!(
            a.reserved && !a.spawnable,
            "collision reservation still applies to unlaned"
        );
    }

    #[test]
    fn holding_unlaned_as_a_lane_does_not_reserve() {
        // `unlaned` is never a real lane, so a (nonsensical) reservation of
        // a lane literally named "unlaned" must not reserve the sentinel.
        let issues = vec![with_lane(mk("a-par", "open", "normal"), UNLANED)];
        let res = Reservations::from_tokens(["unlaned".to_string()]);
        let v = compute(&issues, &default_schema(), Some(&res));
        assert!(!v.unscheduled[0].reserved);
        assert!(v.unscheduled[0].spawnable);
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
