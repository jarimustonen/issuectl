//! Markdown task-list parser for Definition-of-Done evaluation.
//!
//! Issues converge on three canonical H2 sections that together form
//! the "DoD contract":
//!
//! - [`ACCEPTANCE_CRITERIA`] — the gate. Unchecked items here block or
//!   warn on a transition to a closing status, depending on schema's
//!   [`dod.strict`][crate::schema::DodConfig::strict] setting.
//! - [`TESTS_RUN`] — informational checklist of tests exercised.
//! - [`IMPLEMENTATION_NOTES`] — free-form prose, no checklist semantics.
//!
//! Parsing is intentionally simple and fence-aware: a `- [ ]` /
//! `- [x]` line inside a fenced code block is content, not a
//! checkbox. Bullet variants `* ` and `+ ` are also recognised
//! because that's what users actually type. CommonMark-strict
//! parsing is not the goal — the rule is keyed on the shape users
//! write when they want a checklist.
//!
//! This module is the single source of truth for task-list parsing.
//! `transitions::acceptance_criteria_message` and the
//! `issuectl ready` command both route through here so a change to
//! the rules updates every surface at once.

use crate::body_sections;

/// Canonical H2 section name: the acceptance-criteria gate. Items
/// here are checked by `evaluate` and reported by `issuectl ready`.
pub const ACCEPTANCE_CRITERIA: &str = "Acceptance Criteria";
/// Canonical H2 section name: the (informational) tests-run checklist.
pub const TESTS_RUN: &str = "Tests Run";
/// Canonical H2 section name: free-form implementation notes (no
/// checklist semantics — presence/absence only).
pub const IMPLEMENTATION_NOTES: &str = "Implementation Notes";

/// A single parsed task-list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkbox {
    /// Raw text after the `[x]` / `[ ]` marker, trimmed.
    pub text: String,
    /// True when the marker is `[x]` or `[X]`.
    pub checked: bool,
}

/// Result of evaluating one DoD section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionStatus {
    /// True when the named H2 section exists in the body.
    pub present: bool,
    /// Parsed checkboxes in source order. Empty when the section is
    /// absent OR present-but-has-no-task-list (e.g. free-form prose).
    pub checkboxes: Vec<Checkbox>,
}

impl SectionStatus {
    pub fn total(&self) -> usize {
        self.checkboxes.len()
    }
    pub fn checked(&self) -> usize {
        self.checkboxes.iter().filter(|c| c.checked).count()
    }
    pub fn unchecked(&self) -> usize {
        self.total() - self.checked()
    }
    /// True when the section has at least one task-list item and all
    /// items are checked. Returns false when the section is absent or
    /// has no task-list items at all — callers that need to
    /// distinguish "no list" from "incomplete list" should inspect
    /// `total()` / `present` directly.
    pub fn fully_checked(&self) -> bool {
        !self.checkboxes.is_empty() && self.unchecked() == 0
    }
    /// Items still unchecked, in source order.
    pub fn unchecked_items(&self) -> Vec<&Checkbox> {
        self.checkboxes.iter().filter(|c| !c.checked).collect()
    }
}

/// Combined DoD evaluation across the three canonical sections.
/// Cheap to compute — a body walk and a fence scan per section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DodReport {
    pub acceptance: SectionStatus,
    pub tests: SectionStatus,
    pub notes: SectionStatus,
}

impl DodReport {
    /// Walk `body` once and produce a report for the three canonical
    /// DoD sections. Sections that are absent yield empty
    /// [`SectionStatus`] entries (`present: false`).
    pub fn from_body(body: &str) -> Self {
        Self {
            acceptance: parse_section_checkboxes(body, ACCEPTANCE_CRITERIA),
            tests: parse_section_checkboxes(body, TESTS_RUN),
            notes: parse_section_checkboxes(body, IMPLEMENTATION_NOTES),
        }
    }
}

/// Parse the checkboxes of a single named H2 section. Section lookup
/// is fence-aware (delegates to
/// [`body_sections::extract_section_text`]); the in-section walk
/// then re-scans for fence boundaries so a `- [ ]` inside a fenced
/// example is not counted.
pub fn parse_section_checkboxes(body: &str, section: &str) -> SectionStatus {
    let Some(text) = body_sections::extract_section_text(body, section) else {
        return SectionStatus::default();
    };
    SectionStatus {
        present: true,
        checkboxes: parse_checkboxes(&text),
    }
}

/// Parse task-list lines from a block of markdown text. Fence-aware:
/// `- [ ]` lines inside a `` ``` `` / `~~~` block are treated as
/// content. Recognises `- `, `* `, and `+ ` bullets and `[ ]`, `[x]`,
/// `[X]` markers. Unrecognised marker characters (e.g. `[~]`) are
/// not counted — they are not checklist items.
pub fn parse_checkboxes(text: &str) -> Vec<Checkbox> {
    let mut out = Vec::new();
    let mut in_fence: Option<&'static str> = None;
    for raw in text.lines() {
        let trimmed = raw.trim_start();

        if let Some(marker) = in_fence {
            if trimmed.starts_with(marker) {
                in_fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            in_fence = Some("~~~");
            continue;
        }

        let bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "));
        let Some(rest) = bullet else { continue };
        let Some(after_open) = rest.strip_prefix('[') else {
            continue;
        };
        let Some((marker, body_text)) = after_open.split_once(']') else {
            continue;
        };
        if marker.chars().count() != 1 {
            continue;
        }
        let checked = match marker {
            "x" | "X" => true,
            " " => false,
            _ => continue,
        };
        out.push(Checkbox {
            text: body_text.trim().to_string(),
            checked,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_checklist() {
        let body = "\
## Acceptance Criteria

- [x] Done item
- [ ] Pending item
- [X] Also done

## Tests Run

- [ ] cargo test
";
        let r = DodReport::from_body(body);
        assert!(r.acceptance.present);
        assert_eq!(r.acceptance.total(), 3);
        assert_eq!(r.acceptance.checked(), 2);
        assert!(!r.acceptance.fully_checked());
        assert_eq!(r.tests.total(), 1);
        assert!(!r.notes.present);
    }

    #[test]
    fn ignores_checkboxes_inside_fenced_code_block() {
        let body = "\
## Acceptance Criteria

- [x] real

```
- [ ] not a real task
```

- [ ] also real
";
        let r = DodReport::from_body(body);
        assert_eq!(r.acceptance.total(), 2);
        assert_eq!(r.acceptance.checked(), 1);
    }

    #[test]
    fn star_and_plus_bullets_count() {
        let cbs = parse_checkboxes("* [x] one\n+ [ ] two\n- [x] three\n");
        assert_eq!(cbs.len(), 3);
        assert!(cbs[0].checked);
        assert!(!cbs[1].checked);
        assert_eq!(cbs[2].text, "three");
    }

    #[test]
    fn rejects_unknown_markers() {
        let cbs = parse_checkboxes("- [~] weird\n- [-] also weird\n- [ ] ok\n");
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].text, "ok");
    }

    #[test]
    fn fully_checked_requires_at_least_one_item() {
        let s = SectionStatus {
            present: true,
            checkboxes: Vec::new(),
        };
        assert!(!s.fully_checked());
    }

    #[test]
    fn missing_section_reports_absent() {
        let r = DodReport::from_body("# title\n\n## Description\n\nhi\n");
        assert!(!r.acceptance.present);
        assert_eq!(r.acceptance.total(), 0);
    }

    #[test]
    fn unchecked_items_returns_text() {
        let r = DodReport::from_body(
            "## Acceptance Criteria\n\n- [x] done\n- [ ] todo one\n- [ ] todo two\n",
        );
        let u = r.acceptance.unchecked_items();
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].text, "todo one");
    }
}
