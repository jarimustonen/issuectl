//! Shared query language for the CLI (`ls`, `search`), web filters
//! (`/api/issues?q=`), and future surfaces (saved queries, reports).
//!
//! Grammar (v1):
//!
//! ```text
//! query  := term (WS+ term)*
//! term   := '-'? atom
//! atom   := field ':' value | bareword
//! value  := unquoted | '"' quoted '"' | dateExpr
//! ```
//!
//! - `field:value` exact match (case-insensitive) on a frontmatter field
//!   (status, type, priority, assignee, owner, epic, label, slug, folder).
//! - `field:any` / `field:none` — present / absent.
//! - `text:"phrase"` — substring search across title, slug, and body.
//! - Bareword (no `field:` prefix) is treated as a `text:` term.
//! - Date fields (`updated`, `created`, `closed`) accept relative
//!   comparisons: `<-14d`, `>=-30d`, `<=+0d`. Anchor is today (UTC).
//!   `<` and `<=` are both inclusive (≤); `>` and `>=` are both ≥.
//!   Date fields also accept `any` / `none`.
//! - `-field:value` and `-bareword` negate.
//! - Multiple terms are AND-ed. No OR / parentheses in v1.

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, NaiveDate, Utc};

use crate::models::Issue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldName {
    Status,
    Type,
    Priority,
    Assignee,
    Owner,
    Epic,
    Label,
    Slug,
    Folder,
    Updated,
    Created,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateCmp {
    /// `<` or `<=` — issue date ≤ anchor.
    Le,
    /// `>` or `>=` — issue date ≥ anchor.
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMatch {
    Equals(String),
    Present,
    Absent,
    DateRel { op: DateCmp, anchor: NaiveDate },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Field {
        field: FieldName,
        m: FieldMatch,
        negated: bool,
    },
    Text {
        needle: String,
        negated: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub terms: Vec<Term>,
}

impl Query {
    pub fn push(&mut self, t: Term) {
        self.terms.push(t);
    }

    /// True if the query contains any explicit term touching `field`.
    /// Used by the CLI to decide whether the implicit "open folder"
    /// default should still apply.
    pub fn mentions_field(&self, field: FieldName) -> bool {
        self.terms.iter().any(|t| matches!(t, Term::Field { field: f, .. } if *f == field))
    }
}

/// Parse a user-typed query string into a [`Query`].
pub fn parse(input: &str) -> Result<Query> {
    let tokens = tokenize(input)?;
    let mut terms = Vec::with_capacity(tokens.len());
    for tok in tokens {
        terms.push(parse_term(&tok)?);
    }
    Ok(Query { terms })
}

/// Evaluate a query against a single issue. Empty query matches everything.
pub fn matches(q: &Query, i: &Issue) -> bool {
    q.terms.iter().all(|t| eval_term(t, i))
}

// ── tokenizer ───────────────────────────────────────────────────────────────

fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut in_quotes = false;
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_quotes {
            if c == '"' {
                in_quotes = false;
            } else if c == '\\' && i + 1 < chars.len() {
                cur.push(chars[i + 1]);
                i += 1;
            } else {
                cur.push(c);
            }
        } else if c.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut cur));
                started = false;
            }
        } else if c == '"' {
            in_quotes = true;
            started = true;
        } else {
            cur.push(c);
            started = true;
        }
        i += 1;
    }
    if in_quotes {
        bail!("unterminated quoted string in query");
    }
    if started {
        tokens.push(cur);
    }
    Ok(tokens)
}

// ── parser ──────────────────────────────────────────────────────────────────

fn parse_term(raw: &str) -> Result<Term> {
    let (negated, rest) = match raw.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, raw),
    };
    if rest.is_empty() {
        bail!("empty query term");
    }

    if let Some((key, val)) = rest.split_once(':') {
        let key_lc = key.to_ascii_lowercase();

        if key_lc == "text" {
            if val.is_empty() {
                bail!("empty value for text:");
            }
            return Ok(Term::Text {
                needle: val.to_string(),
                negated,
            });
        }

        let field = parse_field_name(&key_lc)
            .map_err(|_| anyhow!("unknown query field: {key:?}"))?;

        if val.is_empty() {
            bail!("empty value for {key}:");
        }

        if val.eq_ignore_ascii_case("any") {
            return Ok(Term::Field {
                field,
                m: FieldMatch::Present,
                negated,
            });
        }
        if val.eq_ignore_ascii_case("none") {
            return Ok(Term::Field {
                field,
                m: FieldMatch::Absent,
                negated,
            });
        }

        if is_date_field(field) && starts_with_date_op(val) {
            return Ok(Term::Field {
                field,
                m: parse_date_match(val)?,
                negated,
            });
        }

        Ok(Term::Field {
            field,
            m: FieldMatch::Equals(val.to_string()),
            negated,
        })
    } else {
        Ok(Term::Text {
            needle: rest.to_string(),
            negated,
        })
    }
}

fn parse_field_name(s: &str) -> Result<FieldName> {
    Ok(match s {
        "status" => FieldName::Status,
        "type" => FieldName::Type,
        "priority" => FieldName::Priority,
        "assignee" => FieldName::Assignee,
        "owner" => FieldName::Owner,
        "epic" => FieldName::Epic,
        "label" => FieldName::Label,
        "slug" => FieldName::Slug,
        "folder" => FieldName::Folder,
        "updated" => FieldName::Updated,
        "created" => FieldName::Created,
        "closed" => FieldName::Closed,
        other => bail!("unknown field: {other}"),
    })
}

fn is_date_field(f: FieldName) -> bool {
    matches!(
        f,
        FieldName::Updated | FieldName::Created | FieldName::Closed
    )
}

fn starts_with_date_op(s: &str) -> bool {
    s.starts_with('<') || s.starts_with('>')
}

fn parse_date_match(val: &str) -> Result<FieldMatch> {
    let (op, rest) = if let Some(r) = val.strip_prefix("<=") {
        (DateCmp::Le, r)
    } else if let Some(r) = val.strip_prefix(">=") {
        (DateCmp::Ge, r)
    } else if let Some(r) = val.strip_prefix('<') {
        (DateCmp::Le, r)
    } else if let Some(r) = val.strip_prefix('>') {
        (DateCmp::Ge, r)
    } else {
        bail!("date filter must start with '<', '<=', '>', or '>='");
    };
    let rest = rest
        .strip_suffix('d')
        .ok_or_else(|| anyhow!("relative date offset must end in 'd' (e.g. '-14d')"))?;
    let days: i64 = rest
        .parse()
        .map_err(|_| anyhow!("invalid relative date offset: {rest:?}"))?;
    let anchor = Utc::now()
        .date_naive()
        .checked_add_signed(Duration::days(days))
        .ok_or_else(|| anyhow!("date offset out of range: {days}"))?;
    Ok(FieldMatch::DateRel { op, anchor })
}

// ── evaluator ───────────────────────────────────────────────────────────────

fn eval_term(t: &Term, i: &Issue) -> bool {
    match t {
        Term::Text { needle, negated } => eval_text(needle, i) ^ *negated,
        Term::Field { field, m, negated } => eval_field(*field, m, i) ^ *negated,
    }
}

fn eval_text(needle: &str, i: &Issue) -> bool {
    let n = needle.to_lowercase();
    if n.is_empty() {
        return true;
    }
    i.title.to_lowercase().contains(&n)
        || i.slug.to_lowercase().contains(&n)
        || i.body.to_lowercase().contains(&n)
}

fn eval_field(f: FieldName, m: &FieldMatch, i: &Issue) -> bool {
    match f {
        FieldName::Status => string_match(m, &i.status),
        FieldName::Type => string_match(m, &i.issue_type),
        FieldName::Priority => string_match(m, &i.priority),
        FieldName::Folder => string_match(m, &i.folder),
        FieldName::Slug => string_match(m, &i.slug),
        FieldName::Assignee => {
            let eff = i.effective_assignee();
            opt_string_match(m, if eff.is_empty() { None } else { Some(eff) })
        }
        FieldName::Owner => opt_string_match(m, i.owner.as_deref()),
        FieldName::Epic => opt_string_match(m, i.epic.as_deref()),
        FieldName::Label => label_match(m, i.labels.as_deref()),
        FieldName::Updated => date_match(m, i.updated.as_deref()),
        FieldName::Created => date_match(m, i.created.as_deref()),
        FieldName::Closed => date_match(m, i.closed.as_deref()),
    }
}

fn string_match(m: &FieldMatch, val: &str) -> bool {
    match m {
        FieldMatch::Equals(v) => val.eq_ignore_ascii_case(v),
        FieldMatch::Present => !val.is_empty(),
        FieldMatch::Absent => val.is_empty(),
        FieldMatch::DateRel { .. } => false,
    }
}

fn opt_string_match(m: &FieldMatch, val: Option<&str>) -> bool {
    match m {
        FieldMatch::Equals(v) => val.is_some_and(|s| s.eq_ignore_ascii_case(v)),
        FieldMatch::Present => val.is_some_and(|s| !s.is_empty()),
        FieldMatch::Absent => val.map(|s| s.is_empty()).unwrap_or(true),
        FieldMatch::DateRel { .. } => false,
    }
}

fn label_match(m: &FieldMatch, labels: Option<&[String]>) -> bool {
    let labels: &[String] = labels.unwrap_or(&[]);
    match m {
        FieldMatch::Equals(v) => labels.iter().any(|l| l.eq_ignore_ascii_case(v)),
        FieldMatch::Present => !labels.is_empty(),
        FieldMatch::Absent => labels.is_empty(),
        FieldMatch::DateRel { .. } => false,
    }
}

fn date_match(m: &FieldMatch, val: Option<&str>) -> bool {
    match m {
        FieldMatch::Equals(v) => val.is_some_and(|s| s.eq_ignore_ascii_case(v)),
        FieldMatch::Present => val.is_some_and(|s| !s.is_empty()),
        FieldMatch::Absent => val.map(|s| s.is_empty()).unwrap_or(true),
        FieldMatch::DateRel { op, anchor } => {
            let Some(s) = val else {
                return false;
            };
            let Some(date) = parse_date_prefix(s) else {
                return false;
            };
            match op {
                DateCmp::Le => date <= *anchor,
                DateCmp::Ge => date >= *anchor,
            }
        }
    }
}

fn parse_date_prefix(s: &str) -> Option<NaiveDate> {
    let head = s.get(..10).unwrap_or(s);
    NaiveDate::parse_from_str(head, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Issue;

    fn mk(slug: &str) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: "open".to_string(),
            created: Some("2026-01-01".to_string()),
            status: "open".to_string(),
            updated: Some("2026-04-01".to_string()),
            priority: "normal".to_string(),
            issue_type: "bug".to_string(),
            reporter: None,
            assignee: Some("alice".to_string()),
            owner: None,
            epic: None,
            related: None,
            labels: Some(vec!["frontend".to_string()]),
            closed: None,
            commits: None,
            title: "Login redirect loop".to_string(),
            body: "User flock contention on flock(2) deadlock.".to_string(),
        }
    }

    #[test]
    fn empty_query_matches_all() {
        let q = parse("").unwrap();
        assert!(q.terms.is_empty());
        assert!(matches(&q, &mk("a-b")));
    }

    #[test]
    fn bareword_is_text() {
        let q = parse("redirect").unwrap();
        assert_eq!(q.terms.len(), 1);
        assert!(matches!(&q.terms[0], Term::Text { negated: false, .. }));
        assert!(matches(&q, &mk("a-b")));
    }

    #[test]
    fn quoted_phrase_text() {
        let q = parse(r#"text:"flock contention""#).unwrap();
        assert!(matches(&q, &mk("a-b")));
        let q2 = parse(r#"text:"will never match""#).unwrap();
        assert!(!matches(&q2, &mk("a-b")));
    }

    #[test]
    fn field_eq_and_negation() {
        let q = parse("type:bug").unwrap();
        assert!(matches(&q, &mk("a-b")));
        let q = parse("-type:bug").unwrap();
        assert!(!matches(&q, &mk("a-b")));
        let q = parse("type:feature").unwrap();
        assert!(!matches(&q, &mk("a-b")));
    }

    #[test]
    fn field_any_none() {
        let mut i = mk("a-b");
        i.assignee = None;
        let q = parse("assignee:none").unwrap();
        assert!(matches(&q, &i));
        let q = parse("assignee:any").unwrap();
        assert!(!matches(&q, &i));
        i.assignee = Some("bob".to_string());
        assert!(matches(&q, &i));
    }

    #[test]
    fn label_match() {
        let q = parse("label:frontend").unwrap();
        assert!(matches(&q, &mk("a-b")));
        let q = parse("-label:frontend").unwrap();
        assert!(!matches(&q, &mk("a-b")));
        let q = parse("label:wontfix").unwrap();
        assert!(!matches(&q, &mk("a-b")));
    }

    #[test]
    fn assignee_falls_back_to_owner() {
        let mut i = mk("a-b");
        i.assignee = None;
        i.owner = Some("carol".to_string());
        let q = parse("assignee:carol").unwrap();
        assert!(matches(&q, &i));
        let q = parse("owner:carol").unwrap();
        assert!(matches(&q, &i));
    }

    #[test]
    fn relative_date_filter() {
        let mut i = mk("a-b");
        let today = Utc::now().date_naive();
        let recent = today - Duration::days(3);
        i.updated = Some(recent.format("%Y-%m-%d").to_string());

        let q = parse("updated:>-14d").unwrap();
        assert!(matches(&q, &i), "recent should be ≥ today-14d");

        let q = parse("updated:<-14d").unwrap();
        assert!(!matches(&q, &i));

        let old = today - Duration::days(30);
        i.updated = Some(old.format("%Y-%m-%d").to_string());
        let q = parse("updated:<-14d").unwrap();
        assert!(matches(&q, &i));
    }

    #[test]
    fn date_inclusive_boundary() {
        let mut i = mk("a-b");
        let today = Utc::now().date_naive();
        let exactly = today - Duration::days(14);
        i.updated = Some(exactly.format("%Y-%m-%d").to_string());
        let q = parse("updated:<-14d").unwrap();
        assert!(matches(&q, &i), "boundary day should be inclusive");
        let q = parse("updated:>-14d").unwrap();
        assert!(matches(&q, &i), "boundary day should be inclusive");
    }

    #[test]
    fn implicit_and() {
        let q = parse("type:bug priority:normal label:frontend").unwrap();
        assert_eq!(q.terms.len(), 3);
        assert!(matches(&q, &mk("a-b")));
        let q = parse("type:bug priority:high").unwrap();
        assert!(!matches(&q, &mk("a-b")));
    }

    #[test]
    fn case_insensitive_field_value() {
        let q = parse("TYPE:Bug").unwrap();
        assert!(matches(&q, &mk("a-b")));
    }

    #[test]
    fn unknown_field_errors() {
        assert!(parse("nonsense:foo").is_err());
    }

    #[test]
    fn unterminated_quote_errors() {
        assert!(parse(r#"text:"oops"#).is_err());
    }

    #[test]
    fn negated_text_excludes() {
        let q = parse("-redirect").unwrap();
        assert!(!matches(&q, &mk("a-b")));
    }

    #[test]
    fn slug_field_matches() {
        let q = parse("slug:extremely-quiet-otter").unwrap();
        let mut i = mk("extremely-quiet-otter");
        assert!(matches(&q, &i));
        i.slug = "different-slug-here".to_string();
        assert!(!matches(&q, &i));
    }

    #[test]
    fn folder_field_matches() {
        let q = parse("folder:closed").unwrap();
        let mut i = mk("a-b");
        assert!(!matches(&q, &i));
        i.folder = "closed".to_string();
        assert!(matches(&q, &i));
    }

    #[test]
    fn quoted_field_value() {
        // Allow embedded spaces in field values via quotes — the lexer
        // joins the quoted run to the preceding `field:` prefix.
        let q = parse(r#"label:"two words""#).unwrap();
        let mut i = mk("a-b");
        i.labels = Some(vec!["two words".to_string()]);
        assert!(matches(&q, &i));
    }

    #[test]
    fn mentions_field_helper() {
        let q = parse("status:open type:bug").unwrap();
        assert!(q.mentions_field(FieldName::Status));
        assert!(q.mentions_field(FieldName::Type));
        assert!(!q.mentions_field(FieldName::Priority));
    }

    #[test]
    fn empty_value_errors() {
        assert!(parse("status:").is_err());
        assert!(parse("text:").is_err());
    }

    #[test]
    fn invalid_date_offset_errors() {
        assert!(parse("updated:<14").is_err());
        assert!(parse("updated:<abc d").is_err());
    }
}
