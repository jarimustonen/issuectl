//! Shared query language for the CLI (`ls`, `search`), web filters
//! (`/api/issues?q=`), and future surfaces (saved queries, reports).
//!
//! Grammar (v1):
//!
//! ```text
//! query  := term (WS+ term)*
//! term   := '-'? atom
//! atom   := field ':' value | bareword
//! ```
//!
//! - `field:value` exact match (ASCII case-insensitive) on a frontmatter
//!   field (status, type, priority, assignee, owner, epic, label, slug,
//!   folder).
//! - `field:any` / `field:none` — present / absent.
//! - `text:"phrase"` — substring search across title, slug, and body.
//! - Bareword (no `field:` prefix) is treated as a `text:` term.
//! - Date fields (`updated`, `created`, `closed`) accept relative
//!   comparisons: `<-14d`, `<=-14d`, `>-30d`, `>=-30d`. Anchor is
//!   today (local timezone — matches how `created`/`updated` are
//!   written). `<` is strict, `<=` is inclusive; same for `>`/`>=`.
//!   Date fields also accept `any` / `none`. Use `<=0d` instead of
//!   `<=+0d` in `?q=` URLs — `+` URL-decodes to space.
//! - `-field:value` and `-bareword` negate.
//! - Multiple terms are AND-ed. No OR / parentheses in v1.
//!
//! Escape (in unquoted text):
//! - `\:` literal colon (not a field separator)
//! - `\\` literal backslash
//! - `\"` literal double-quote
//! - `\ ` literal space (token continuation)
//! - `\-` literal leading hyphen (only at token start; escapes negation)
//!
//! Inside `"..."` quoted strings, `\\` and `\"` are recognized;
//! everything else is literal.

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, Local, NaiveDate};

use crate::models::Issue;

/// Hard upper bound on parsed query input length. Cheap defense
/// against unbounded `?q=` payloads on the unauthenticated read
/// endpoint.
pub const MAX_QUERY_BYTES: usize = 4096;

/// Hard upper bound on number of terms in a parsed query. Same
/// rationale as [`MAX_QUERY_BYTES`].
pub const MAX_QUERY_TERMS: usize = 64;

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
    /// `<` — issue date < anchor (strict).
    Lt,
    /// `<=` — issue date ≤ anchor.
    Le,
    /// `>` — issue date > anchor (strict).
    Gt,
    /// `>=` — issue date ≥ anchor.
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMatch {
    Equals(String),
    Present,
    Absent,
    /// Relative-date comparison. Stored as a *days offset from today*
    /// rather than a frozen anchor so a parsed `Query` is portable
    /// across time (saved queries, reports).
    DateRel { op: DateCmp, days: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Field {
        field: FieldName,
        m: FieldMatch,
        negated: bool,
    },
    Text {
        /// Already lowercased at parse time.
        needle_lc: String,
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

    /// True when any term targets `field` *and* asserts a value
    /// (i.e. it's not a negation). Used by callers that want to
    /// know whether the user explicitly scoped a dimension; a
    /// negated term excludes but does not scope.
    pub fn has_positive_field(&self, field: FieldName) -> bool {
        self.terms.iter().any(|t| {
            matches!(t,
                Term::Field { field: f, negated: false, .. } if *f == field
            )
        })
    }

    /// True when the query has at least one `text:` (or bareword) term.
    pub fn has_text_term(&self) -> bool {
        self.terms.iter().any(|t| matches!(t, Term::Text { .. }))
    }
}

/// Parse a user-typed query string into a [`Query`]. Returns an
/// error for length-cap, term-cap, malformed-syntax, or unknown
/// fields.
pub fn parse(input: &str) -> Result<Query> {
    if input.len() > MAX_QUERY_BYTES {
        bail!(
            "query too long: {} bytes (max {})",
            input.len(),
            MAX_QUERY_BYTES
        );
    }
    let raw = tokenize(input)?;
    if raw.len() > MAX_QUERY_TERMS {
        bail!(
            "too many query terms: {} (max {})",
            raw.len(),
            MAX_QUERY_TERMS
        );
    }
    let mut terms = Vec::with_capacity(raw.len());
    for t in raw {
        terms.push(build_term(t)?);
    }
    Ok(Query { terms })
}

/// Evaluate a query against a single issue using today's local date
/// as the anchor for relative-date terms. Empty query matches
/// everything.
pub fn matches(q: &Query, i: &Issue) -> bool {
    matches_at(q, i, Local::now().date_naive())
}

/// Like [`matches`], but takes the "today" anchor explicitly. Used
/// by tests and (eventually) saved-query evaluators that want a
/// stable clock.
pub fn matches_at(q: &Query, i: &Issue, today: NaiveDate) -> bool {
    if q.terms.is_empty() {
        return true;
    }
    // Lowercase title/slug/body once per issue when any text term is
    // present, instead of per-term. The Issue is read-only here so
    // we can't memoize across calls without changing the API; doing
    // it once per `matches` call is the cheap win.
    let text_lc = q.has_text_term().then(|| TextLc::new(i));
    q.terms
        .iter()
        .all(|t| eval_term(t, i, today, text_lc.as_ref()))
}

// ── tokenizer ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawToken {
    negated: bool,
    /// `Some(key)` for `field:value` form, `None` for bareword.
    key: Option<String>,
    /// Already-unescaped value (for fields) or bareword text.
    value: String,
}

fn tokenize(input: &str) -> Result<Vec<RawToken>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Skip leading whitespace.
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        let mut negated = false;
        let mut started = false;
        let mut in_quotes = false;
        let mut seen_colon = false;
        let mut before = String::new();
        let mut after = String::new();

        while i < chars.len() {
            let c = chars[i];

            if !in_quotes && c.is_whitespace() {
                break;
            }

            // Backslash escape: take the next char literally and
            // treat it as plain text regardless of context. Works
            // both inside and outside quoted strings (consistency
            // with most query languages).
            if c == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                push_char(next, seen_colon, &mut before, &mut after);
                started = true;
                i += 2;
                continue;
            }

            if c == '"' {
                in_quotes = !in_quotes;
                started = true;
                i += 1;
                continue;
            }

            if !in_quotes {
                if !started && c == '-' {
                    negated = true;
                    started = true;
                    i += 1;
                    continue;
                }
                if c == ':' && !seen_colon {
                    seen_colon = true;
                    started = true;
                    i += 1;
                    continue;
                }
            }

            push_char(c, seen_colon, &mut before, &mut after);
            started = true;
            i += 1;
        }

        if in_quotes {
            bail!("unterminated quoted string in query");
        }
        if !started {
            // Defensive: shouldn't happen given the leading-whitespace
            // skip above, but bail rather than emit a phantom token.
            break;
        }

        let token = if seen_colon {
            RawToken {
                negated,
                key: Some(before),
                value: after,
            }
        } else {
            RawToken {
                negated,
                key: None,
                value: before,
            }
        };
        tokens.push(token);
    }
    Ok(tokens)
}

fn push_char(c: char, after_colon: bool, before: &mut String, after: &mut String) {
    if after_colon {
        after.push(c);
    } else {
        before.push(c);
    }
}

// ── term builder ────────────────────────────────────────────────────────────

fn build_term(raw: RawToken) -> Result<Term> {
    let RawToken {
        negated,
        key,
        value,
    } = raw;

    match key {
        Some(key) => {
            let key_lc = key.to_ascii_lowercase();

            if key_lc == "text" {
                if value.is_empty() {
                    bail!("empty value for text:");
                }
                return Ok(Term::Text {
                    needle_lc: value.to_lowercase(),
                    negated,
                });
            }

            let field = parse_field_name(&key_lc)
                .map_err(|_| anyhow!("unknown query field: {key:?}"))?;

            if value.is_empty() {
                bail!("empty value for {key}:");
            }

            if value.eq_ignore_ascii_case("any") {
                return Ok(Term::Field {
                    field,
                    m: FieldMatch::Present,
                    negated,
                });
            }
            if value.eq_ignore_ascii_case("none") {
                return Ok(Term::Field {
                    field,
                    m: FieldMatch::Absent,
                    negated,
                });
            }

            if is_date_field(field) && starts_with_date_op(&value) {
                return Ok(Term::Field {
                    field,
                    m: parse_date_match(&value)?,
                    negated,
                });
            }

            Ok(Term::Field {
                field,
                m: FieldMatch::Equals(value),
                negated,
            })
        }
        None => {
            if value.is_empty() {
                bail!("empty query term");
            }
            Ok(Term::Text {
                needle_lc: value.to_lowercase(),
                negated,
            })
        }
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
        (DateCmp::Lt, r)
    } else if let Some(r) = val.strip_prefix('>') {
        (DateCmp::Gt, r)
    } else {
        bail!("date filter must start with '<', '<=', '>', or '>='");
    };
    let rest = rest
        .strip_suffix('d')
        .ok_or_else(|| anyhow!("relative date offset must end in 'd' (e.g. '-14d')"))?;
    let days: i64 = rest
        .parse()
        .map_err(|_| anyhow!("invalid relative date offset: {rest:?}"))?;
    Ok(FieldMatch::DateRel { op, days })
}

// ── evaluator ───────────────────────────────────────────────────────────────

struct TextLc {
    title: String,
    slug: String,
    body: String,
}

impl TextLc {
    fn new(i: &Issue) -> Self {
        Self {
            title: i.title.to_lowercase(),
            slug: i.slug.to_lowercase(),
            body: i.body.to_lowercase(),
        }
    }

    fn contains(&self, needle_lc: &str) -> bool {
        self.title.contains(needle_lc)
            || self.slug.contains(needle_lc)
            || self.body.contains(needle_lc)
    }
}

fn eval_term(t: &Term, i: &Issue, today: NaiveDate, text_lc: Option<&TextLc>) -> bool {
    match t {
        Term::Text { needle_lc, negated } => {
            // text_lc is Some whenever the query has text terms — by
            // construction in `matches_at`. Defensive fallback for
            // direct `eval_term` callers (none in tree).
            let hit = match text_lc {
                Some(lc) => lc.contains(needle_lc),
                None => i.title.to_lowercase().contains(needle_lc)
                    || i.slug.to_lowercase().contains(needle_lc)
                    || i.body.to_lowercase().contains(needle_lc),
            };
            hit ^ *negated
        }
        Term::Field { field, m, negated } => eval_field(*field, m, i, today) ^ *negated,
    }
}

fn eval_field(f: FieldName, m: &FieldMatch, i: &Issue, today: NaiveDate) -> bool {
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
        FieldName::Updated => date_match(m, i.updated.as_deref(), today),
        FieldName::Created => date_match(m, i.created.as_deref(), today),
        FieldName::Closed => date_match(m, i.closed.as_deref(), today),
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

fn date_match(m: &FieldMatch, val: Option<&str>, today: NaiveDate) -> bool {
    match m {
        FieldMatch::Equals(v) => {
            // Compare as dates if both sides parse cleanly; fall
            // back to raw string equality for non-date payloads.
            match (val.and_then(parse_date_prefix), parse_date_prefix(v)) {
                (Some(a), Some(b)) => a == b,
                _ => val.is_some_and(|s| s.eq_ignore_ascii_case(v)),
            }
        }
        FieldMatch::Present => val.is_some_and(|s| !s.is_empty()),
        FieldMatch::Absent => val.map(|s| s.is_empty()).unwrap_or(true),
        FieldMatch::DateRel { op, days } => {
            let Some(s) = val else {
                return false;
            };
            let Some(date) = parse_date_prefix(s) else {
                return false;
            };
            let anchor = match today.checked_add_signed(Duration::days(*days)) {
                Some(d) => d,
                None => return false,
            };
            match op {
                DateCmp::Lt => date < anchor,
                DateCmp::Le => date <= anchor,
                DateCmp::Gt => date > anchor,
                DateCmp::Ge => date >= anchor,
            }
        }
    }
}

fn parse_date_prefix(s: &str) -> Option<NaiveDate> {
    let head = s.get(..10)?;
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

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 8).unwrap()
    }

    #[test]
    fn empty_query_matches_all() {
        let q = parse("").unwrap();
        assert!(q.terms.is_empty());
        assert!(matches_at(&q, &mk("a-b"), today()));
    }

    #[test]
    fn bareword_is_text() {
        let q = parse("redirect").unwrap();
        assert_eq!(q.terms.len(), 1);
        assert!(matches_at(&q, &mk("a-b"), today()));
    }

    #[test]
    fn quoted_phrase_text() {
        let q = parse(r#"text:"flock contention""#).unwrap();
        assert!(matches_at(&q, &mk("a-b"), today()));
        let q2 = parse(r#"text:"will never match""#).unwrap();
        assert!(!matches_at(&q2, &mk("a-b"), today()));
    }

    #[test]
    fn field_eq_and_negation() {
        let q = parse("type:bug").unwrap();
        assert!(matches_at(&q, &mk("a-b"), today()));
        let q = parse("-type:bug").unwrap();
        assert!(!matches_at(&q, &mk("a-b"), today()));
        let q = parse("type:feature").unwrap();
        assert!(!matches_at(&q, &mk("a-b"), today()));
    }

    #[test]
    fn field_any_none() {
        let mut i = mk("a-b");
        i.assignee = None;
        let q = parse("assignee:none").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("assignee:any").unwrap();
        assert!(!matches_at(&q, &i, today()));
        i.assignee = Some("bob".to_string());
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn label_match() {
        let q = parse("label:frontend").unwrap();
        assert!(matches_at(&q, &mk("a-b"), today()));
        let q = parse("-label:frontend").unwrap();
        assert!(!matches_at(&q, &mk("a-b"), today()));
        let q = parse("label:wontfix").unwrap();
        assert!(!matches_at(&q, &mk("a-b"), today()));
    }

    #[test]
    fn assignee_falls_back_to_owner() {
        let mut i = mk("a-b");
        i.assignee = None;
        i.owner = Some("carol".to_string());
        let q = parse("assignee:carol").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("owner:carol").unwrap();
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn date_strict_vs_inclusive() {
        let mut i = mk("a-b");
        let today = today();
        let exactly = today - Duration::days(14);
        i.updated = Some(exactly.format("%Y-%m-%d").to_string());

        // Strict `<` excludes the boundary.
        let q = parse("updated:<-14d").unwrap();
        assert!(!matches_at(&q, &i, today));
        let q = parse("updated:>-14d").unwrap();
        assert!(!matches_at(&q, &i, today));
        // Inclusive forms include the boundary.
        let q = parse("updated:<=-14d").unwrap();
        assert!(matches_at(&q, &i, today));
        let q = parse("updated:>=-14d").unwrap();
        assert!(matches_at(&q, &i, today));
    }

    #[test]
    fn date_relative_window() {
        let mut i = mk("a-b");
        let today = today();
        i.updated = Some((today - Duration::days(3)).format("%Y-%m-%d").to_string());
        let q = parse("updated:>-14d").unwrap();
        assert!(matches_at(&q, &i, today), "3 days ago is > 14 days ago");
        let q = parse("updated:<-14d").unwrap();
        assert!(!matches_at(&q, &i, today));

        i.updated = Some((today - Duration::days(30)).format("%Y-%m-%d").to_string());
        let q = parse("updated:<-14d").unwrap();
        assert!(matches_at(&q, &i, today));
    }

    #[test]
    fn date_equals_normalizes_timestamp() {
        let mut i = mk("a-b");
        i.updated = Some("2026-05-07T12:34:56Z".to_string());
        let q = parse("updated:2026-05-07").unwrap();
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn implicit_and() {
        let q = parse("type:bug priority:normal label:frontend").unwrap();
        assert_eq!(q.terms.len(), 3);
        assert!(matches_at(&q, &mk("a-b"), today()));
        let q = parse("type:bug priority:high").unwrap();
        assert!(!matches_at(&q, &mk("a-b"), today()));
    }

    #[test]
    fn case_insensitive_field_value() {
        let q = parse("TYPE:Bug").unwrap();
        assert!(matches_at(&q, &mk("a-b"), today()));
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
        assert!(!matches_at(&q, &mk("a-b"), today()));
    }

    #[test]
    fn slug_field_matches() {
        let q = parse("slug:extremely-quiet-otter").unwrap();
        let mut i = mk("extremely-quiet-otter");
        assert!(matches_at(&q, &i, today()));
        i.slug = "different-slug-here".to_string();
        assert!(!matches_at(&q, &i, today()));
    }

    #[test]
    fn folder_field_matches() {
        let q = parse("folder:closed").unwrap();
        let mut i = mk("a-b");
        assert!(!matches_at(&q, &i, today()));
        i.folder = "closed".to_string();
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn quoted_field_value_with_spaces() {
        let q = parse(r#"label:"two words""#).unwrap();
        let mut i = mk("a-b");
        i.labels = Some(vec!["two words".to_string()]);
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn has_positive_field_helper() {
        let q = parse("status:open type:bug").unwrap();
        assert!(q.has_positive_field(FieldName::Status));
        assert!(q.has_positive_field(FieldName::Type));
        assert!(!q.has_positive_field(FieldName::Priority));
        // Negation does NOT count as a positive scope term.
        let q = parse("-status:wontfix").unwrap();
        assert!(!q.has_positive_field(FieldName::Status));
    }

    #[test]
    fn empty_value_errors() {
        assert!(parse("status:").is_err());
        assert!(parse("text:").is_err());
    }

    #[test]
    fn invalid_date_offset_errors() {
        assert!(parse("updated:<14").is_err());
    }

    #[test]
    fn escape_colon_keeps_token_as_text() {
        // \: should NOT split into a field — the whole token stays
        // a bareword "src/main.rs:15".
        let q = parse(r"src/main.rs\:15").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => {
                assert_eq!(needle_lc, "src/main.rs:15");
            }
            other => panic!("expected text term, got {other:?}"),
        }
    }

    #[test]
    fn escape_leading_dash() {
        let q = parse(r"\-foo").unwrap();
        match &q.terms[0] {
            Term::Text {
                needle_lc,
                negated: false,
            } => {
                assert_eq!(needle_lc, "-foo");
            }
            other => panic!("expected positive text term, got {other:?}"),
        }
    }

    #[test]
    fn escape_space_joins_tokens() {
        let q = parse(r"text:two\ words").unwrap();
        assert_eq!(q.terms.len(), 1);
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, "two words"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn escape_backslash() {
        let q = parse(r"text:a\\b").unwrap();
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, r"a\b"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn url_in_field_value_works_unescaped() {
        // `text:` already routes the value verbatim, so URLs work
        // without escaping.
        let q = parse(r#"text:"http://example.com""#).unwrap();
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, "http://example.com"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn query_too_long_errors() {
        let big = "a".repeat(MAX_QUERY_BYTES + 1);
        assert!(parse(&big).is_err());
    }

    #[test]
    fn too_many_terms_errors() {
        let s = (0..MAX_QUERY_TERMS + 1)
            .map(|n| format!("t{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(parse(&s).is_err());
    }

    #[test]
    fn date_rel_serializes_as_offset() {
        // The AST stores a days offset, not a frozen anchor. A
        // saved query parsed today still evaluates against the
        // caller-supplied `today` next week.
        let q = parse("updated:>-7d").unwrap();
        let m = match &q.terms[0] {
            Term::Field {
                m: FieldMatch::DateRel { op, days },
                ..
            } => (op, days),
            other => panic!("expected DateRel, got {other:?}"),
        };
        assert_eq!(*m.1, -7);
        assert_eq!(*m.0, DateCmp::Gt);
    }
}
