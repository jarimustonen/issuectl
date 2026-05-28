//! Local-only heuristic duplicate detection.
//!
//! Random slugs make it hard for a human to notice that the issue
//! they're filing already exists. This module scores issue pairs using
//! cheap, deterministic, offline signals — no embeddings, no remote
//! AI:
//!
//! - **title overlap** — Jaccard similarity of normalized title tokens,
//!   the dominant signal (people phrase the same bug similarly);
//! - **body overlap** — Jaccard of significant body tokens;
//! - **label overlap** — Jaccard of the (case-folded) label sets.
//!
//! The three are combined with fixed weights into a single score in
//! `[0.0, 1.0]`. A dimension with no content on either side simply
//! contributes 0 — it never invents similarity.
//!
//! Consumed by the CLI `duplicates` command and the opt-in `new
//! --check-duplicates` pre-check.

use std::collections::BTreeSet;

use crate::models::Issue;

/// Weight applied to title-token overlap.
const W_TITLE: f64 = 0.60;
/// Weight applied to body-token overlap.
const W_BODY: f64 = 0.25;
/// Weight applied to label-set overlap.
const W_LABEL: f64 = 0.15;

/// Default score threshold for the `duplicates` command — tuned to
/// surface plausible matches a human should glance at, accepting some
/// false positives (the cost of a false positive is one ignored line).
pub const DEFAULT_THRESHOLD: f64 = 0.30;

/// Threshold for the `new --check-duplicates` pre-check. Higher than
/// [`DEFAULT_THRESHOLD`] because here a hit blocks creation, so we only
/// want to stop the user for a genuinely strong match.
pub const STRONG_THRESHOLD: f64 = 0.50;

/// Tokens shorter than this are dropped before scoring — single letters
/// and most two-letter words are noise that inflates overlap.
const MIN_TOKEN_LEN: usize = 3;

/// A small, deliberately conservative English stop-word set. Kept short
/// on purpose: aggressive stop-listing risks discarding the very domain
/// words ("the login the user the redirect") that distinguish issues.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "have", "has", "not", "but", "are", "was",
    "were", "will", "should", "would", "could", "can", "when", "then", "than", "into", "out", "via",
    "use", "uses", "using", "make", "made", "does", "did", "all", "any", "some", "its", "our",
    "your", "their",
];

/// One scored match against a target issue.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateMatch {
    pub slug: String,
    pub title: String,
    pub score: f64,
    pub title_overlap: f64,
    pub body_overlap: f64,
    pub label_overlap: f64,
}

/// One scored pair from an all-pairs scan. `a`/`b` are ordered by slug
/// so a given unordered pair is reported exactly once.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicatePair {
    pub a_slug: String,
    pub a_title: String,
    pub b_slug: String,
    pub b_title: String,
    pub score: f64,
    pub title_overlap: f64,
    pub body_overlap: f64,
    pub label_overlap: f64,
}

/// Pre-tokenized view of an issue, so an all-pairs scan tokenizes each
/// issue once rather than once per comparison.
struct Tokens {
    title: BTreeSet<String>,
    body: BTreeSet<String>,
    labels: BTreeSet<String>,
}

impl Tokens {
    fn new(i: &Issue) -> Self {
        Tokens {
            title: tokenize(&i.title),
            body: tokenize(&i.body),
            labels: i
                .labels
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty())
                .collect(),
        }
    }
}

struct Components {
    score: f64,
    title: f64,
    body: f64,
    label: f64,
}

fn score_tokens(a: &Tokens, b: &Tokens) -> Components {
    let title = jaccard(&a.title, &b.title);
    let body = jaccard(&a.body, &b.body);
    let label = jaccard(&a.labels, &b.labels);
    Components {
        score: W_TITLE * title + W_BODY * body + W_LABEL * label,
        title,
        body,
        label,
    }
}

/// Score `target` against each candidate (excluding the target itself
/// by slug), returning matches at or above `threshold`, sorted by
/// descending score then ascending slug for stable output.
pub fn find_duplicates<'a, I>(target: &Issue, candidates: I, threshold: f64) -> Vec<DuplicateMatch>
where
    I: IntoIterator<Item = &'a Issue>,
{
    let target_tokens = Tokens::new(target);
    let mut matches: Vec<DuplicateMatch> = candidates
        .into_iter()
        .filter(|c| c.slug != target.slug)
        .filter_map(|c| {
            let comp = score_tokens(&target_tokens, &Tokens::new(c));
            (comp.score >= threshold).then(|| DuplicateMatch {
                slug: c.slug.clone(),
                title: c.title.clone(),
                score: comp.score,
                title_overlap: comp.title,
                body_overlap: comp.body,
                label_overlap: comp.label,
            })
        })
        .collect();
    matches.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.slug.cmp(&y.slug))
    });
    matches
}

/// Scan every unordered pair in `issues`, returning those at or above
/// `threshold`, sorted by descending score then by slug pair.
pub fn find_all_pairs(issues: &[Issue], threshold: f64) -> Vec<DuplicatePair> {
    let toks: Vec<Tokens> = issues.iter().map(Tokens::new).collect();
    let mut pairs = Vec::new();
    for i in 0..issues.len() {
        for j in (i + 1)..issues.len() {
            let comp = score_tokens(&toks[i], &toks[j]);
            if comp.score < threshold {
                continue;
            }
            // Order endpoints by slug so the pair is canonical.
            let (lo, hi) = if issues[i].slug <= issues[j].slug {
                (&issues[i], &issues[j])
            } else {
                (&issues[j], &issues[i])
            };
            pairs.push(DuplicatePair {
                a_slug: lo.slug.clone(),
                a_title: lo.title.clone(),
                b_slug: hi.slug.clone(),
                b_title: hi.title.clone(),
                score: comp.score,
                title_overlap: comp.title,
                body_overlap: comp.body,
                label_overlap: comp.label,
            });
        }
    }
    pairs.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.a_slug.cmp(&y.a_slug))
            .then_with(|| x.b_slug.cmp(&y.b_slug))
    });
    pairs
}

/// Split text into normalized tokens: lowercase, split on any
/// non-alphanumeric boundary, drop short tokens and stop-words.
fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .filter(|t| t.chars().count() >= MIN_TOKEN_LEN && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Jaccard similarity: |A ∩ B| / |A ∪ B|. Two empty sets score 0 (no
/// evidence of similarity), not 1.
fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn issue(slug: &str, title: &str, body: &str, labels: &[&str]) -> Issue {
        Issue {
            slug: slug.to_string(),
            folder: "open".to_string(),
            created: None,
            status: "open".to_string(),
            updated: None,
            priority: "normal".to_string(),
            issue_type: "bug".to_string(),
            reporter: None,
            assignee: None,
            owner: None,
            epic: None,
            related: None,
            labels: if labels.is_empty() {
                None
            } else {
                Some(labels.iter().map(|s| s.to_string()).collect())
            },
            closed: None,
            commits: None,
            title: title.to_string(),
            body: body.to_string(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn identical_titles_score_high() {
        let a = issue("a-a", "Login redirect loop", "", &[]);
        let b = issue("b-b", "Login redirect loop", "", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1);
        assert!((m[0].title_overlap - 1.0).abs() < 1e-9);
        assert!(m[0].score >= W_TITLE - 1e-9);
    }

    #[test]
    fn unrelated_issues_score_zero() {
        let a = issue("a-a", "Login redirect loop", "auth session cookie", &[]);
        let b = issue("b-b", "Export CSV pagination", "report download offset", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), 0.0001);
        assert!(m.is_empty(), "got {m:?}");
    }

    #[test]
    fn shared_labels_and_body_lift_score() {
        let a = issue(
            "a-a",
            "Cannot upload avatar",
            "avatar upload fails with timeout error",
            &["frontend", "upload"],
        );
        let b = issue(
            "b-b",
            "Avatar upload broken",
            "uploading avatar times out repeatedly",
            &["upload", "frontend"],
        );
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1);
        assert!(m[0].label_overlap > 0.9, "labels identical → ~1.0");
        assert!(m[0].body_overlap > 0.0);
        assert!(m[0].title_overlap > 0.0);
    }

    #[test]
    fn self_is_excluded() {
        let a = issue("a-a", "Same title", "same body", &[]);
        let dup = issue("a-a", "Same title", "same body", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&dup), 0.0);
        assert!(m.is_empty(), "target must not match itself by slug");
    }

    #[test]
    fn results_sorted_descending_by_score() {
        let target = issue("t-t", "alpha beta gamma delta", "", &[]);
        let strong = issue("s-s", "alpha beta gamma delta", "", &[]);
        let weak = issue("w-w", "alpha zulu yankee xray", "", &[]);
        let m = find_duplicates(
            &target,
            vec![&weak, &strong],
            0.0001,
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].slug, "s-s", "highest score first");
        assert!(m[0].score >= m[1].score);
    }

    #[test]
    fn all_pairs_reports_each_pair_once_canonical() {
        let issues = vec![
            issue("zzz-z", "shared duplicate title", "", &[]),
            issue("aaa-a", "shared duplicate title", "", &[]),
            issue("mmm-m", "totally different subject matter", "", &[]),
        ];
        let pairs = find_all_pairs(&issues, DEFAULT_THRESHOLD);
        assert_eq!(pairs.len(), 1, "only the two matching titles pair");
        // Endpoints ordered by slug.
        assert_eq!(pairs[0].a_slug, "aaa-a");
        assert_eq!(pairs[0].b_slug, "zzz-z");
    }

    #[test]
    fn short_tokens_and_stopwords_dropped() {
        // Only stop-words / short tokens in common → no real overlap.
        let a = issue("a-a", "the a an of", "to be or", &[]);
        let b = issue("b-b", "the a an of", "to be or", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), 0.0001);
        assert!(m.is_empty(), "stop-words must not manufacture a match: {m:?}");
    }

    #[test]
    fn missing_labels_contribute_zero_not_one() {
        let a = issue("a-a", "Some unique heading here", "", &[]);
        let b = issue("b-b", "Some unique heading here", "", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label_overlap, 0.0, "two empty label sets → 0");
    }

    #[test]
    fn punctuation_and_case_are_normalized() {
        let a = issue("a-a", "Login/Redirect LOOP!", "", &[]);
        let b = issue("b-b", "login redirect loop", "", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1);
        assert!((m[0].title_overlap - 1.0).abs() < 1e-9);
    }
}
