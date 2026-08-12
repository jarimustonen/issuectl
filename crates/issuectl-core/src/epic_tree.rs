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

use std::collections::HashSet;

use serde::Serialize;

use crate::models::Issue;

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

/// Build the tree rooted at `root_slug`. Returns `None` when no issue
/// carries that slug (the caller maps that to the shared `not-found`
/// error). The root need not be `type: epic` — the children relation is
/// what matters — but in practice it is.
pub fn build(issues: &[Issue], root_slug: &str) -> Option<TreeNode> {
    let root = issues.iter().find(|i| i.slug == root_slug)?;
    let mut visited: HashSet<&str> = HashSet::new();
    Some(build_node(issues, root, &mut visited))
}

/// Build a forest of every top-level epic: each `type: epic` issue whose
/// own `epic:` back-reference is absent or dangling (so a nested epic is
/// not also surfaced as its own root — it appears under its parent).
/// Roots are sorted by slug. Used by `epic tree` with no slug argument.
pub fn build_forest(issues: &[Issue]) -> Vec<TreeNode> {
    let mut roots: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.issue_type == "epic" && !has_present_epic_parent(issues, i))
        .collect();
    roots.sort_by(|a, b| a.slug.cmp(&b.slug));
    roots
        .into_iter()
        .map(|root| {
            let mut visited: HashSet<&str> = HashSet::new();
            build_node(issues, root, &mut visited)
        })
        .collect()
}

/// True when `issue.epic` points at another issue that is present in the
/// set. A dangling or absent back-reference makes the issue a forest root.
fn has_present_epic_parent(issues: &[Issue], issue: &Issue) -> bool {
    match issue.epic.as_deref() {
        Some(parent) => issues.iter().any(|i| i.slug == parent),
        None => false,
    }
}

/// Recursively assemble a node and its children. `visited` guards against
/// an `epic:` cycle (`a.epic = b`, `b.epic = a`) so the walk terminates —
/// a slug already on the current path contributes a leaf, never a second
/// expansion.
fn build_node<'a>(
    issues: &'a [Issue],
    issue: &'a Issue,
    visited: &mut HashSet<&'a str>,
) -> TreeNode {
    let mut node = TreeNode::leaf(issue);
    if !visited.insert(issue.slug.as_str()) {
        // Already on the path above us — stop here to break the cycle.
        return node;
    }
    let mut children: Vec<&Issue> = issues
        .iter()
        .filter(|c| c.epic.as_deref() == Some(issue.slug.as_str()) && c.slug != issue.slug)
        .collect();
    children.sort_by(|a, b| a.slug.cmp(&b.slug));
    node.children = children
        .into_iter()
        .map(|c| build_node(issues, c, visited))
        .collect();
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
