//! Shared query language for the CLI (`ls`, `search`) and future
//! surfaces (saved queries, reports).
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

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, NaiveDate};

use crate::clock::Clock;

use crate::models::Issue;

/// Hard upper bound on parsed query input length. Cheap defense
/// against unbounded `?q=` payloads on the unauthenticated read
/// endpoint.
pub const MAX_QUERY_BYTES: usize = 4096;

/// Hard upper bound on number of terms in a parsed query. Same
/// rationale as [`MAX_QUERY_BYTES`].
pub const MAX_QUERY_TERMS: usize = 64;

/// Hard upper bound on the magnitude of a relative date offset in
/// days. Sized at 1000 years — well outside any sane query and well
/// inside `chrono::Duration::days`'s panic threshold (≈ ±106 M
/// days), so callers can apply the offset without overflow checks.
pub const MAX_RELATIVE_DATE_DAYS: i64 = 365 * 1000;

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
    Reviewer,
    ReviewStatus,
    /// `blocked_by:<slug>` / `:any` / `:none` — read directly off the
    /// current issue's `blocked_by` array (no graph needed).
    BlockedBy,
    /// `blocks:<slug>` / `:any` / `:none` — derived at evaluation time
    /// from the repo-wide blocker graph (see `MatchCtx`). Requires a
    /// non-empty graph in the context; the plain `matches`/`matches_at`
    /// entry points return `false` for `Blocks` terms because they
    /// can't see other issues.
    Blocks,
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
    DateRel {
        op: DateCmp,
        days: i64,
    },
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

    /// True when the user explicitly constrained `field` with any
    /// non-negated term: `field:value`, `field:any`, `field:none`,
    /// or a relative-date comparison. A negated term (`-field:x`)
    /// is exclusion, not scope, so it returns false. Callers use
    /// this to decide whether the user has positively pinned a
    /// dimension and the implicit default should step out of the way.
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

/// Rewrite identity-style `field:me` terms in `q` to the resolved
/// `current_user`. Applies to the user-shaped fields (`reviewer`,
/// `assignee`, `owner`); `reviewer:me` is the canonical caller from
/// the v0.6.0 review-field work. Errors when `current_user` is
/// `None` and the query mentions `:me` — silently treating the
/// literal `"me"` as a username would let the user blindly miss
/// their own queue, and silently matching no issues would mask the
/// mis-configuration. Callers that don't know the current user
/// should skip the call entirely; `parse` leaves `:me` as a literal
/// `Equals("me")` value that simply matches nothing in practice.
pub fn resolve_me(q: &mut Query, current_user: Option<&str>) -> Result<()> {
    for t in &mut q.terms {
        if let Term::Field { field, m, .. } = t {
            if !matches!(
                field,
                FieldName::Reviewer | FieldName::Assignee | FieldName::Owner
            ) {
                continue;
            }
            if let FieldMatch::Equals(v) = m {
                if v.eq_ignore_ascii_case("me") {
                    let Some(u) = current_user else {
                        bail!(
                            "query uses `:me` but no current user is configured \
                             (set `git config user.name`)"
                        );
                    };
                    *m = FieldMatch::Equals(u.to_string());
                }
            }
        }
    }
    Ok(())
}

/// Per-evaluation context: today's date anchor plus an optional
/// repo-wide `slug -> blocked_by` map used to resolve `blocks:<slug>`
/// queries. Callers that don't care about `blocks:` (boards summary,
/// tests) build an empty graph; CLI list/search/bulk routes populate
/// the graph from the loaded issue set.
pub struct MatchCtx<'a> {
    pub today: NaiveDate,
    pub blocked_by_graph: &'a BTreeMap<String, Vec<String>>,
}

impl<'a> MatchCtx<'a> {
    pub fn new(today: NaiveDate, graph: &'a BTreeMap<String, Vec<String>>) -> Self {
        Self {
            today,
            blocked_by_graph: graph,
        }
    }

    /// Today's local-date anchor + the supplied graph. Convenience for
    /// CLI call sites that don't need a controlled clock.
    pub fn today(graph: &'a BTreeMap<String, Vec<String>>) -> Self {
        Self::new(crate::clock::SystemClock.today(), graph)
    }
}

/// Build the `slug -> blocked_by` graph the query layer needs to
/// resolve `blocks:<slug>` and the doctor uses to detect cycles. The
/// reverse `blocks` relationship is intentionally not stored in
/// frontmatter — it is derived here.
pub fn build_blocked_by_graph(issues: &[Issue]) -> BTreeMap<String, Vec<String>> {
    let mut g = BTreeMap::new();
    for i in issues {
        let deps = i.blocked_by();
        if !deps.is_empty() {
            g.insert(i.slug.clone(), deps);
        }
    }
    g
}

/// Evaluate a query against a single issue using today's local date
/// as the anchor for relative-date terms. Empty query matches
/// everything. `blocks:<slug>` terms always evaluate to false through
/// this entry point — use [`matches_with`] with a populated graph to
/// enable that filter.
pub fn matches(q: &Query, i: &Issue) -> bool {
    matches_at(q, i, crate::clock::SystemClock.today())
}

/// Like [`matches`], but takes the "today" anchor explicitly. Used
/// by tests and (eventually) saved-query evaluators that want a
/// stable clock. Same `blocks:` caveat as [`matches`].
pub fn matches_at(q: &Query, i: &Issue, today: NaiveDate) -> bool {
    static EMPTY: std::sync::OnceLock<BTreeMap<String, Vec<String>>> = std::sync::OnceLock::new();
    let ctx = MatchCtx {
        today,
        blocked_by_graph: EMPTY.get_or_init(BTreeMap::new),
    };
    matches_with(q, i, &ctx)
}

/// Full-context evaluator. Honours `blocks:<slug>` when the context
/// graph is populated.
pub fn matches_with(q: &Query, i: &Issue, ctx: &MatchCtx<'_>) -> bool {
    if q.terms.is_empty() {
        return true;
    }
    let text_lc = q.has_text_term().then(|| TextLc::new(i));
    q.terms
        .iter()
        .all(|t| eval_term(t, i, ctx, text_lc.as_ref()))
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

            // Backslash escape. Outside quotes: `\` consumes the next
            // char literally (works for `:`, `\`, ` `, `"`, `-`,
            // and anything else — the escape is "treat the next char
            // as plain text"). Inside quotes: only `\\` and `\"`
            // are recognized; any other `\X` preserves the
            // backslash literally so paths like `"C:\temp"` and
            // regex fragments like `"\d+"` survive intact. A
            // trailing `\` at end-of-input is a syntax error in
            // both contexts.
            if c == '\\' {
                let Some(&next) = chars.get(i + 1) else {
                    bail!("trailing backslash in query");
                };
                if in_quotes && next != '\\' && next != '"' {
                    // Literal backslash; let the next iteration
                    // process `next` normally.
                    push_char('\\', seen_colon, &mut before, &mut after);
                    started = true;
                    i += 1;
                    continue;
                }
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

            let field =
                parse_field_name(&key_lc).map_err(|_| anyhow!("unknown query field: {key:?}"))?;

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

            if starts_with_date_op(&value) {
                if !is_date_field(field) {
                    bail!(
                        "relative date comparison only valid on \
                         updated/created/closed: {key}:{value}"
                    );
                }
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
        "reviewer" => FieldName::Reviewer,
        "review_status" => FieldName::ReviewStatus,
        "blocked_by" => FieldName::BlockedBy,
        "blocks" => FieldName::Blocks,
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
    if days.abs() > MAX_RELATIVE_DATE_DAYS {
        bail!("relative date offset out of range: {days}d (max ±{MAX_RELATIVE_DATE_DAYS}d)");
    }
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

fn eval_term(t: &Term, i: &Issue, ctx: &MatchCtx<'_>, text_lc: Option<&TextLc>) -> bool {
    match t {
        Term::Text { needle_lc, negated } => {
            // Invariant (enforced by `matches_with`): `text_lc` is
            // `Some` whenever the query contains a text term, so a
            // `Text` term is only ever evaluated with the cache in
            // hand. Panicking here is the right behavior for an
            // internal contract violation.
            let lc = text_lc.expect("text_lc must be present when evaluating a Text term");
            lc.contains(needle_lc) ^ *negated
        }
        Term::Field { field, m, negated } => eval_field(*field, m, i, ctx) ^ *negated,
    }
}

fn eval_field(f: FieldName, m: &FieldMatch, i: &Issue, ctx: &MatchCtx<'_>) -> bool {
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
        FieldName::Updated => date_match(m, i.updated.as_deref(), ctx.today),
        FieldName::Created => date_match(m, i.created.as_deref(), ctx.today),
        FieldName::Closed => date_match(m, i.closed.as_deref(), ctx.today),
        FieldName::Reviewer => opt_string_match(m, extra_str(i, "reviewer")),
        FieldName::ReviewStatus => opt_string_match(m, extra_str(i, "review_status")),
        FieldName::BlockedBy => blocked_by_match(m, &i.blocked_by()),
        FieldName::Blocks => blocks_match(m, i, ctx),
    }
}

fn blocked_by_match(m: &FieldMatch, deps: &[String]) -> bool {
    match m {
        FieldMatch::Equals(v) => {
            let needle = v.trim().strip_prefix('@').unwrap_or(v.trim());
            deps.iter().any(|d| d.eq_ignore_ascii_case(needle))
        }
        FieldMatch::Present => !deps.is_empty(),
        FieldMatch::Absent => deps.is_empty(),
        FieldMatch::DateRel { .. } => false,
    }
}

/// `blocks:<slug>` matches issues that appear in the target slug's
/// `blocked_by` list. `blocks:any` / `blocks:none` ask: does this
/// issue block any other / no other issue in the repo. Both rely on
/// the precomputed `blocked_by_graph` in [`MatchCtx`]; with an empty
/// graph everything but `blocks:none` evaluates to false.
fn blocks_match(m: &FieldMatch, i: &Issue, ctx: &MatchCtx<'_>) -> bool {
    let graph = ctx.blocked_by_graph;
    match m {
        FieldMatch::Equals(v) => {
            let target = v.trim().strip_prefix('@').unwrap_or(v.trim()).to_string();
            graph
                .get(&target)
                .map(|deps| deps.iter().any(|d| d.eq_ignore_ascii_case(&i.slug)))
                .unwrap_or(false)
        }
        FieldMatch::Present => graph.values().any(|deps| deps.contains(&i.slug)),
        FieldMatch::Absent => graph.values().all(|deps| !deps.contains(&i.slug)),
        FieldMatch::DateRel { .. } => false,
    }
}

/// Project an `extra` frontmatter key down to a `Option<&str>` for
/// opt-string matching. Non-string extras (lists, mappings) match
/// nothing — same shape as `assignee:` matching an empty value.
fn extra_str<'a>(i: &'a Issue, key: &str) -> Option<&'a str> {
    i.extra
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
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

/// Parse a `YYYY-MM-DD` date from the head of `s`. Accepts a bare
/// date or a date followed by `T` / space (so ISO-8601 timestamps
/// like `2026-05-07T12:34:56Z` parse to their date component).
/// Anything else trailing — `2026-05-07garbage`, `2026-05-07x` —
/// fails so a typo on the query side doesn't silently match.
fn parse_date_prefix(s: &str) -> Option<NaiveDate> {
    let head = s.get(..10)?;
    let date = NaiveDate::parse_from_str(head, "%Y-%m-%d").ok()?;
    match s.as_bytes().get(10) {
        None | Some(b'T') | Some(b' ') => Some(date),
        _ => None,
    }
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
            closed_by: None,
            lane: None,
            collision: None,
            lane_seq: None,
            commits: None,
            title: "Login redirect loop".to_string(),
            body: "User flock contention on flock(2) deadlock.".to_string(),
            extra: std::collections::BTreeMap::new(),
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
    fn reviewer_field_reads_from_extra() {
        let mut i = mk("a-b");
        i.extra.insert(
            "reviewer".to_string(),
            serde_json::Value::String("dana".to_string()),
        );
        let q = parse("reviewer:dana").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("reviewer:bob").unwrap();
        assert!(!matches_at(&q, &i, today()));
        let q = parse("reviewer:any").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("reviewer:none").unwrap();
        assert!(!matches_at(&q, &i, today()));
    }

    #[test]
    fn review_status_field_reads_from_extra() {
        let mut i = mk("a-b");
        i.extra.insert(
            "review_status".to_string(),
            serde_json::Value::String("requested".to_string()),
        );
        let q = parse("review_status:requested").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("review_status:approved").unwrap();
        assert!(!matches_at(&q, &i, today()));
    }

    #[test]
    fn resolve_me_substitutes_current_user() {
        let mut q = parse("reviewer:me").unwrap();
        resolve_me(&mut q, Some("dana")).unwrap();
        let mut i = mk("a-b");
        i.extra.insert(
            "reviewer".to_string(),
            serde_json::Value::String("dana".to_string()),
        );
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn resolve_me_errors_without_current_user() {
        let mut q = parse("reviewer:me").unwrap();
        let err = resolve_me(&mut q, None).expect_err("expected error");
        assert!(err.to_string().contains(":me"), "err={err}");
    }

    #[test]
    fn resolve_me_leaves_non_user_fields_alone() {
        // `type:me` would be a nonsense literal; the resolver must not
        // touch fields outside the user-shaped set.
        let mut q = parse("type:me").unwrap();
        resolve_me(&mut q, Some("dana")).unwrap();
        match &q.terms[0] {
            Term::Field {
                m: FieldMatch::Equals(v),
                ..
            } => assert_eq!(v, "me"),
            _ => panic!("expected Field term"),
        }
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
    fn date_offset_out_of_range_errors() {
        // R2-C1 regression: huge offsets must error at parse time
        // instead of panicking inside `Duration::days` at eval.
        assert!(parse("updated:<-9999999999999d").is_err());
        assert!(parse(&format!("updated:<-{}d", MAX_RELATIVE_DATE_DAYS + 1)).is_err());
        // Boundary value still parses.
        assert!(parse(&format!("updated:<-{MAX_RELATIVE_DATE_DAYS}d")).is_ok());
    }

    #[test]
    fn quoted_unknown_escape_preserves_backslash() {
        // R2-C2 regression: inside `"..."`, only `\\` and `\"` are
        // escapes. Anything else keeps the backslash literal.
        let q = parse(r#"text:"C:\temp""#).unwrap();
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, r"c:\temp"),
            other => panic!("expected text, got {other:?}"),
        }

        let q = parse(r#"text:"\d+\s+""#).unwrap();
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, r"\d+\s+"),
            other => panic!("expected text, got {other:?}"),
        }

        // `\\` and `\"` still recognized inside quotes.
        let q = parse(r#"text:"a\\b\"c""#).unwrap();
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, r#"a\b"c"#),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn date_prefix_rejects_garbage_suffix() {
        // R2-C3 regression: query-side date with trailing garbage
        // must not silently match a clean stored date.
        let mut i = mk("a-b");
        i.updated = Some("2026-05-07".to_string());
        let q = parse("updated:2026-05-07garbage").unwrap();
        assert!(!matches_at(&q, &i, today()));

        // Real ISO-8601 timestamps still match cleanly.
        i.updated = Some("2026-05-07T12:34:56Z".to_string());
        let q = parse("updated:2026-05-07").unwrap();
        assert!(matches_at(&q, &i, today()));
    }

    #[test]
    fn trailing_backslash_errors() {
        // R2-M1 regression.
        assert!(parse(r"text:foo\").is_err());
        assert!(parse(r"\").is_err());
    }

    #[test]
    fn date_op_on_non_date_field_errors() {
        // R2-M4 regression: `priority:<14d` must not silently
        // become `Equals("<14d")` and match nothing.
        assert!(parse("priority:<14d").is_err());
        assert!(parse("status:>-7d").is_err());
        // Date fields still accept the same syntax.
        assert!(parse("updated:<-14d").is_ok());
    }

    #[test]
    fn colon_at_token_start_errors() {
        // R2-Min1: `:foo` should error rather than producing a
        // confusing `unknown query field: ""`.
        let err = parse(":foo").unwrap_err().to_string();
        assert!(
            err.contains("field") || err.contains("unknown"),
            "expected field-name error, got: {err}"
        );
    }

    #[test]
    fn quoted_and_unquoted_segments_concatenate() {
        // R2-Min2: lock the tokenizer's segment-concatenation
        // behavior so future rewrites can't silently regress it.
        let q = parse(r#"text:"foo"bar"#).unwrap();
        match &q.terms[0] {
            Term::Text { needle_lc, .. } => assert_eq!(needle_lc, "foobar"),
            other => panic!("expected text, got {other:?}"),
        }

        let q = parse(r#"label:foo" bar"baz"#).unwrap();
        match &q.terms[0] {
            Term::Field {
                field: FieldName::Label,
                m: FieldMatch::Equals(v),
                ..
            } => assert_eq!(v, "foo barbaz"),
            other => panic!("expected label field, got {other:?}"),
        }
    }

    fn mk_with_blocked_by(slug: &str, refs: &[&str]) -> Issue {
        let mut i = mk(slug);
        i.extra.insert(
            "blocked_by".into(),
            serde_json::Value::Array(
                refs.iter()
                    .map(|s| serde_json::Value::String((*s).into()))
                    .collect(),
            ),
        );
        i
    }

    #[test]
    fn blocked_by_any_none_match_on_issue_alone() {
        let q = parse("blocked_by:any").unwrap();
        let with = mk_with_blocked_by("a-b", &["@other-issue-here"]);
        let without = mk("c-d");
        assert!(matches_at(&q, &with, today()));
        assert!(!matches_at(&q, &without, today()));
        let q = parse("blocked_by:none").unwrap();
        assert!(!matches_at(&q, &with, today()));
        assert!(matches_at(&q, &without, today()));
    }

    #[test]
    fn blocked_by_equals_strips_at_sigil_and_is_case_insensitive() {
        let q = parse("blocked_by:other-issue-here").unwrap();
        let i = mk_with_blocked_by("a-b", &["@other-issue-here"]);
        assert!(matches_at(&q, &i, today()));
        let q = parse("blocked_by:@other-issue-here").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("blocked_by:OTHER-issue-here").unwrap();
        assert!(matches_at(&q, &i, today()));
        let q = parse("blocked_by:not-present").unwrap();
        assert!(!matches_at(&q, &i, today()));
    }

    #[test]
    fn blocks_requires_graph_in_context() {
        // `blocks:<slug>` needs the target's `blocked_by` list. The plain
        // `matches_at` entry has no graph and therefore evaluates the
        // term to false; `matches_with` + a populated graph honours it.
        let target = mk_with_blocked_by("target-issue-here", &["@source-one-here"]);
        let source = mk("source-one-here");
        let unrelated = mk("third-other-here");
        let q = parse("blocks:target-issue-here").unwrap();

        // No graph: every issue evaluates false.
        assert!(!matches_at(&q, &source, today()));

        let graph = build_blocked_by_graph(&[target.clone(), source.clone(), unrelated.clone()]);
        let ctx = MatchCtx::new(today(), &graph);
        assert!(matches_with(&q, &source, &ctx));
        assert!(!matches_with(&q, &unrelated, &ctx));
        assert!(!matches_with(&q, &target, &ctx));
    }

    #[test]
    fn blocks_any_lists_issues_that_block_anything() {
        let target = mk_with_blocked_by("blocked-target-here", &["@first-source-here"]);
        let first = mk("first-source-here");
        let idle = mk("idle-issue-here");
        let graph = build_blocked_by_graph(&[target, first.clone(), idle.clone()]);
        let ctx = MatchCtx::new(today(), &graph);
        let q = parse("blocks:any").unwrap();
        assert!(matches_with(&q, &first, &ctx));
        assert!(!matches_with(&q, &idle, &ctx));
        let q = parse("blocks:none").unwrap();
        assert!(!matches_with(&q, &first, &ctx));
        assert!(matches_with(&q, &idle, &ctx));
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
