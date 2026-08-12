//! Epic → child-issue tree, derived on read from the `epic:` frontmatter
//! back-reference. Read-only: it never mutates anything under `issues/`.
//!
//! The parent/child relation is the same one the context bundle resolves
//! (`context::build` walks child → parent; this module walks the reverse,
//! parent → children). A child is any issue whose `epic:` equals the
//! parent's slug. Epics can nest: a child that is itself an epic gets its
//! own children expanded, so the result is a genuine tree, not a single
//! level. A `visited` set breaks any accidental `epic:` cycle so the walk
//! always terminates.
//!
//! `TreeNode` is deliberately lightweight — slug, title, status, type,
//! priority, and nested `children` — rather than the full `Issue`, because
//! this is a navigation view, not a detail view (`show` is the detail
//! surface). The nested `children` array is the one place the flat-object
//! `--json` contract gives way to structure: a tree cannot be flat, and
//! the issue's `--json` variant is specified to emit the tree structurally.
//!
//! Both entry points index the parent→children relation once (`children_by_parent`)
//! and traverse the index, so building a tree over `N` issues is `O(N)`, not
//! the `O(N²)` a per-node linear rescan would cost. Recursion depth is capped
//! at [`MAX_DEPTH`]: issue files are user-editable, so a pathologically deep
//! `epic:` chain must not be able to stack-overflow the CLI — beyond the cap a
//! node is emitted as a leaf rather than expanded further.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::models::Issue;

/// Maximum epic-nesting depth expanded before a node is left as a leaf.
/// Real epic hierarchies are a handful of levels deep; this only ever trips
/// on malformed/adversarial data, where it trades a truncated view for a
/// guaranteed-terminating, stack-safe walk. Also bounds the depth of the
/// tree the human renderer and `descendant_count` later recurse over.
pub const MAX_DEPTH: usize = 256;

/// One node in the epic tree: an issue plus its (recursively expanded)
/// child issues. Field names reuse the shared vocabulary (`slug`,
/// `title`, `status`, `type`, `priority`) so consumers parse a node the
/// same way they parse a `show`/`ls` element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeNode {
    pub slug: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    /// Direct children (issues whose `epic:` is this node's slug), sorted
    /// by slug. Empty for a leaf.
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    fn leaf(issue: &Issue) -> Self {
        TreeNode {
            slug: issue.slug.clone(),
            title: issue.title.clone(),
            status: issue.status.clone(),
            priority: issue.priority.clone(),
            issue_type: issue.issue_type.clone(),
            children: Vec::new(),
        }
    }
}

/// Index the parent→children relation in a single `O(N)` pass: map each
/// `epic:` slug to the issues that back-reference it. A self-reference
/// (`epic: <own slug>`) is dropped so an issue is never its own child.
/// Each child vector is sorted by slug so output is deterministic
/// regardless of input order (`load()` sorts already, but the module does
/// not depend on that invariant).
fn children_by_parent(issues: &[Issue]) -> HashMap<&str, Vec<&Issue>> {
    let mut map: HashMap<&str, Vec<&Issue>> = HashMap::new();
    for issue in issues {
        if let Some(parent) = issue.epic.as_deref() {
            if parent != issue.slug {
                map.entry(parent).or_default().push(issue);
            }
        }
    }
    for kids in map.values_mut() {
        kids.sort_by(|a, b| a.slug.cmp(&b.slug));
    }
    map
}

/// Build the tree rooted at `root_slug`. Returns `None` when no issue
/// carries that slug (the caller maps that to the shared `not-found`
/// error). The root need not be `type: epic` — the children relation is
/// what matters, so `epic tree <task>` deliberately renders the subtree
/// rooted at any explicitly-named issue — but in practice it is an epic.
pub fn build(issues: &[Issue], root_slug: &str) -> Option<TreeNode> {
    let root = issues.iter().find(|i| i.slug == root_slug)?;
    let children = children_by_parent(issues);
    let mut visited: HashSet<&str> = HashSet::new();
    Some(build_node(root, &children, &mut visited, 0))
}

/// Build a forest of every top-level epic. A `type: epic` issue is a forest
/// root when it is not nested under another *present epic* — i.e. its
/// `epic:` is absent, points to itself, dangles, or points to a non-epic
/// issue (a non-epic parent has no tree to hang under, so the epic would
/// otherwise vanish). Epics tangled in a pure cycle (`a↔b`, no external
/// root) have no such root, so a final sweep adds the lowest-slug member of
/// each still-unreached epic component as a representative root rather than
/// silently dropping the whole component. Roots come out sorted by slug.
pub fn build_forest(issues: &[Issue]) -> Vec<TreeNode> {
    let children = children_by_parent(issues);
    let epic_slugs: HashSet<&str> = issues
        .iter()
        .filter(|i| i.issue_type == "epic")
        .map(|i| i.slug.as_str())
        .collect();

    // `issues` is slug-sorted, so both the root pass and the cycle sweep
    // visit epics in slug order — the forest is deterministic. `reached`
    // owns its slugs (built nodes own their strings), keyed for `&str`
    // lookups via `Borrow`.
    let mut forest = Vec::new();
    let mut reached: HashSet<String> = HashSet::new();

    // Pass 1: proper roots (epics not nested under another present epic).
    for issue in issues.iter().filter(|i| is_forest_root(i, &epic_slugs)) {
        let mut visited: HashSet<&str> = HashSet::new();
        let node = build_node(issue, &children, &mut visited, 0);
        mark_reached(&node, &mut reached);
        forest.push(node);
    }

    // Pass 2: sweep epics left unreached by any root — these form pure
    // cycles. Surface each component once via its lowest-slug member. The
    // `reached` check is inside the loop body (not an iterator filter) so
    // it can be updated as each swept component is marked.
    for issue in issues.iter() {
        if issue.issue_type != "epic" || reached.contains(issue.slug.as_str()) {
            continue;
        }
        let mut visited: HashSet<&str> = HashSet::new();
        let node = build_node(issue, &children, &mut visited, 0);
        mark_reached(&node, &mut reached);
        forest.push(node);
    }

    // Deterministic order independent of input order and of which pass
    // produced each root.
    forest.sort_by(|a, b| a.slug.cmp(&b.slug));
    forest
}

/// Whether an epic is a top-level forest root: only epics qualify, and one
/// is a root unless its `epic:` names another *present epic* (that is not
/// itself). Absent / self / dangling / non-epic parents all make it a root.
fn is_forest_root(issue: &Issue, epic_slugs: &HashSet<&str>) -> bool {
    if issue.issue_type != "epic" {
        return false;
    }
    match issue.epic.as_deref() {
        None => true,
        Some(parent) if parent == issue.slug => true,
        Some(parent) => !epic_slugs.contains(parent),
    }
}

/// Record every slug in an already-built subtree as reached, so the forest
/// cycle-sweep does not re-root an epic that already appears under a root.
fn mark_reached(node: &TreeNode, reached: &mut HashSet<String>) {
    reached.insert(node.slug.clone());
    for child in &node.children {
        mark_reached(child, reached);
    }
}

/// Recursively assemble a node and its children from the prebuilt `children`
/// index. `visited` is a path set: a slug already on the current path yields
/// a leaf (breaking an `epic:` cycle), and `depth` caps expansion at
/// [`MAX_DEPTH`] so a pathologically deep chain cannot overflow the stack.
fn build_node<'a>(
    issue: &'a Issue,
    children: &HashMap<&'a str, Vec<&'a Issue>>,
    visited: &mut HashSet<&'a str>,
    depth: usize,
) -> TreeNode {
    let mut node = TreeNode::leaf(issue);
    if depth >= MAX_DEPTH {
        // Depth cap reached — stop expanding, leave this node a leaf.
        return node;
    }
    if !visited.insert(issue.slug.as_str()) {
        // Already on the path above us — stop here to break the cycle.
        return node;
    }
    if let Some(kids) = children.get(issue.slug.as_str()) {
        node.children = kids
            .iter()
            .map(|c| build_node(c, children, visited, depth + 1))
            .collect();
    }
    visited.remove(issue.slug.as_str());
    node
}

/// Count of nodes below `node` (excludes the node itself). Handy for a
/// one-line summary line in the human view.
pub fn descendant_count(node: &TreeNode) -> usize {
    node.children.iter().map(|c| 1 + descendant_count(c)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Issue;

    fn issue(slug: &str, issue_type: &str, epic: Option<&str>) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: "open".to_string(),
            created: Some("2026-01-01".to_string()),
            status: "open".to_string(),
            updated: None,
            priority: "normal".to_string(),
            issue_type: issue_type.to_string(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: epic.map(|s| s.to_string()),
            related: None,
            labels: None,
            closed: None,
            closed_by: None,
            commits: None,
            lane: None,
            collision: None,
            lane_seq: None,
            title: format!("Title of {slug}"),
            body: String::new(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn build_returns_none_for_missing_root() {
        let issues = vec![issue("solo", "task", None)];
        assert!(build(&issues, "ghost").is_none());
    }

    #[test]
    fn build_assembles_epic_with_children_sorted_by_slug() {
        // Deliberately out of slug order in the input to prove sorting.
        let issues = vec![
            issue("the-epic", "epic", None),
            issue("zebra-child", "task", Some("the-epic")),
            issue("alpha-child", "bug", Some("the-epic")),
            issue("unrelated", "task", None),
        ];
        let tree = build(&issues, "the-epic").expect("root present");
        assert_eq!(tree.slug, "the-epic");
        assert_eq!(tree.issue_type, "epic");
        let child_slugs: Vec<_> = tree.children.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(child_slugs, vec!["alpha-child", "zebra-child"]);
        // Unrelated issue is not pulled in.
        assert!(tree.children.iter().all(|c| c.slug != "unrelated"));
        assert_eq!(descendant_count(&tree), 2);
    }

    #[test]
    fn build_nests_sub_epics() {
        let issues = vec![
            issue("root-epic", "epic", None),
            issue("sub-epic", "epic", Some("root-epic")),
            issue("leaf-of-sub", "task", Some("sub-epic")),
            issue("leaf-of-root", "task", Some("root-epic")),
        ];
        let tree = build(&issues, "root-epic").expect("root present");
        // Two direct children: leaf-of-root and sub-epic.
        assert_eq!(tree.children.len(), 2);
        let sub = tree
            .children
            .iter()
            .find(|c| c.slug == "sub-epic")
            .expect("sub-epic nested under root");
        assert_eq!(sub.children.len(), 1);
        assert_eq!(sub.children[0].slug, "leaf-of-sub");
        // Nested leaf counts toward the descendant total.
        assert_eq!(descendant_count(&tree), 3);
    }

    #[test]
    fn build_leaf_when_no_children() {
        let issues = vec![issue("lonely-epic", "epic", None)];
        let tree = build(&issues, "lonely-epic").expect("root present");
        assert!(tree.children.is_empty());
        assert_eq!(descendant_count(&tree), 0);
    }

    #[test]
    fn build_terminates_on_epic_cycle() {
        // a.epic = b, b.epic = a — a malformed cycle must not loop forever.
        let issues = vec![
            issue("cycle-a", "epic", Some("cycle-b")),
            issue("cycle-b", "epic", Some("cycle-a")),
        ];
        let tree = build(&issues, "cycle-a").expect("root present");
        // a → b → a (already on the path, stops as a leaf): the walk
        // terminates with one bounded repeat rather than looping.
        assert_eq!(tree.slug, "cycle-a");
        assert_eq!(tree.children.len(), 1);
        let b = &tree.children[0];
        assert_eq!(b.slug, "cycle-b");
        assert_eq!(b.children.len(), 1);
        assert_eq!(b.children[0].slug, "cycle-a");
        assert!(b.children[0].children.is_empty());
    }

    #[test]
    fn forest_lists_top_level_epics_only() {
        let issues = vec![
            issue("epic-two", "epic", None),
            issue("epic-one", "epic", None),
            issue("nested-epic", "epic", Some("epic-one")),
            issue("a-task", "task", Some("epic-two")),
        ];
        let forest = build_forest(&issues);
        let roots: Vec<_> = forest.iter().map(|n| n.slug.as_str()).collect();
        // Sorted, and the nested epic is not a root.
        assert_eq!(roots, vec!["epic-one", "epic-two"]);
        // But the nested epic appears under its parent.
        let epic_one = forest.iter().find(|n| n.slug == "epic-one").unwrap();
        assert!(epic_one.children.iter().any(|c| c.slug == "nested-epic"));
    }

    #[test]
    fn forest_treats_dangling_parent_as_root() {
        let issues = vec![issue("orphan-epic", "epic", Some("gone-missing"))];
        let forest = build_forest(&issues);
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].slug, "orphan-epic");
    }

    #[test]
    fn forest_surfaces_self_parented_epic() {
        // A self-parent (`a.epic = a`) must not make the epic vanish from
        // the forest — it is treated as a root.
        let issues = vec![issue("self-epic", "epic", Some("self-epic"))];
        let forest = build_forest(&issues);
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].slug, "self-epic");
        // Self-reference never becomes a child of itself.
        assert!(forest[0].children.is_empty());
    }

    #[test]
    fn forest_surfaces_epic_under_non_epic_parent() {
        // An epic whose `epic:` points at a *task* has no epic to hang
        // under, so it must appear as its own forest root rather than
        // disappearing (the task is never a forest root).
        let issues = vec![
            issue("host-task", "task", None),
            issue("stranded-epic", "epic", Some("host-task")),
        ];
        let forest = build_forest(&issues);
        let roots: Vec<_> = forest.iter().map(|n| n.slug.as_str()).collect();
        assert_eq!(roots, vec!["stranded-epic"]);
    }

    #[test]
    fn forest_surfaces_pure_epic_cycle_via_lowest_slug() {
        // a↔b with no external root: the whole component must not vanish.
        // The sweep surfaces it once, via its lowest-slug member.
        let issues = vec![
            issue("cycle-a", "epic", Some("cycle-b")),
            issue("cycle-b", "epic", Some("cycle-a")),
        ];
        let forest = build_forest(&issues);
        // Exactly one representative root, the lowest slug.
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].slug, "cycle-a");
        // And it terminates (cycle broken as a leaf), not looping forever.
        assert_eq!(forest[0].children[0].slug, "cycle-b");
    }

    #[test]
    fn build_caps_recursion_depth() {
        // A linear `epic:` chain longer than MAX_DEPTH must not overflow the
        // stack: expansion stops at the cap, leaving a bounded tree. Chain:
        // node-0 is the root, node-{k} has epic node-{k-1}.
        let len = MAX_DEPTH + 5;
        let mut issues: Vec<Issue> = Vec::with_capacity(len);
        issues.push(issue("node-00000", "epic", None));
        for k in 1..len {
            // Zero-pad so `load()`-style slug sorting matches numeric order.
            let slug = format!("node-{k:05}");
            let parent = format!("node-{:05}", k - 1);
            issues.push(issue(&slug, "epic", Some(&parent)));
        }
        let tree = build(&issues, "node-00000").expect("root present");
        // Deepest expanded node sits at depth == MAX_DEPTH; beyond that the
        // chain is left unexpanded, so exactly MAX_DEPTH descendants appear.
        assert_eq!(descendant_count(&tree), MAX_DEPTH);
    }

    #[test]
    fn json_shape_uses_shared_field_vocabulary() {
        let issues = vec![
            issue("json-epic", "epic", None),
            issue("json-child", "task", Some("json-epic")),
        ];
        let tree = build(&issues, "json-epic").unwrap();
        let v = serde_json::to_value(&tree).unwrap();
        assert_eq!(v["slug"], "json-epic");
        assert_eq!(v["type"], "epic");
        assert_eq!(v["status"], "open");
        assert_eq!(v["priority"], "normal");
        assert_eq!(v["children"][0]["slug"], "json-child");
        assert_eq!(v["children"][0]["type"], "task");
    }
}
