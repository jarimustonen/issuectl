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

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

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
/// are noise. Two-letter tokens are kept: software issues lean on
/// acronyms (`ui`, `ux`, `db`, `ci`, `qa`, `pr`, `os`, `s3`) that carry
/// real signal, so common two-letter function words are stop-listed
/// individually instead.
const MIN_TOKEN_LEN: usize = 2;

/// A small, deliberately conservative English stop-word set. Kept short
/// on purpose: aggressive stop-listing risks discarding the very domain
/// words ("the login the user the redirect") that distinguish issues.
/// Two-letter function words are listed here (rather than handled by a
/// length cutoff) so meaningful acronyms survive tokenization.
const STOPWORDS: &[&str] = &[
    "is", "to", "of", "or", "an", "in", "on", "at", "be", "by", "as", "we", "it", "do", "no", "so",
    "the", "and", "for", "with", "that", "this", "from", "have", "has", "not", "but", "are", "was",
    "were", "will", "should", "would", "could", "can", "when", "then", "than", "into", "out",
    "via", "use", "uses", "using", "make", "made", "does", "did", "all", "any", "some", "its",
    "our", "your", "their",
];

/// `STOPWORDS` as a hash set, built once. Tokenization checks membership
/// for every token, so a linear scan over the slice would cost ~50 string
/// comparisons per token; the set makes it O(1).
static STOPWORD_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| STOPWORDS.iter().copied().collect());

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

/// Maps token strings to dense integer IDs so token-set intersection
/// becomes a merge over sorted `u32` slices instead of repeated
/// `String` comparisons. One interner is shared across every issue in a
/// scan, so equal tokens always collapse to the same ID. Title, body,
/// and label tokens share the ID space — harmless, since dimensions are
/// only ever intersected against their own kind.
#[derive(Default)]
struct Interner {
    map: HashMap<String, u32>,
}

impl Interner {
    fn intern(&mut self, s: String) -> u32 {
        if let Some(&id) = self.map.get(s.as_str()) {
            return id;
        }
        let id = u32::try_from(self.map.len()).expect("token interner exceeded u32::MAX entries");
        self.map.insert(s, id);
        id
    }
}

/// Pre-tokenized view of an issue, so an all-pairs scan tokenizes each
/// issue once rather than once per comparison. Each field is a sorted,
/// deduplicated list of interned token IDs.
struct Tokens {
    title: Vec<u32>,
    body: Vec<u32>,
    labels: Vec<u32>,
}

impl Tokens {
    fn new(i: &Issue, interner: &mut Interner) -> Self {
        let mut labels: Vec<u32> = i
            .labels
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .map(|l| interner.intern(l))
            .collect();
        labels.sort_unstable();
        labels.dedup();
        Tokens {
            title: tokenize(&i.title, interner),
            body: tokenize(&i.body, interner),
            labels,
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

    // Renormalize over the dimensions that actually have content on at
    // least one side. Without this, a body-less, label-less issue could
    // never exceed W_TITLE (0.60) even against an exact title twin — the
    // absent dimensions would silently cap the score. A dimension where
    // both sides are empty carries no evidence either way, so it drops
    // out of both numerator and denominator.
    let mut total = 0.0;
    let mut wsum = 0.0;
    if !a.title.is_empty() || !b.title.is_empty() {
        total += W_TITLE * title;
        wsum += W_TITLE;
    }
    if !a.body.is_empty() || !b.body.is_empty() {
        total += W_BODY * body;
        wsum += W_BODY;
    }
    if !a.labels.is_empty() || !b.labels.is_empty() {
        total += W_LABEL * label;
        wsum += W_LABEL;
    }
    Components {
        score: if wsum > 0.0 { total / wsum } else { 0.0 },
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
    let mut interner = Interner::default();
    let target_tokens = Tokens::new(target, &mut interner);
    let mut matches: Vec<DuplicateMatch> = candidates
        .into_iter()
        .filter(|c| c.slug != target.slug)
        .filter_map(|c| {
            let comp = score_tokens(&target_tokens, &Tokens::new(c, &mut interner));
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
            .total_cmp(&x.score)
            .then_with(|| x.slug.cmp(&y.slug))
    });
    matches
}

/// Scan every unordered pair in `issues`, returning those at or above
/// `threshold`, sorted by descending score then by slug pair.
pub fn find_all_pairs(issues: &[Issue], threshold: f64) -> Vec<DuplicatePair> {
    let mut interner = Interner::default();
    let toks: Vec<Tokens> = issues
        .iter()
        .map(|i| Tokens::new(i, &mut interner))
        .collect();
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
            .total_cmp(&x.score)
            .then_with(|| x.a_slug.cmp(&y.a_slug))
            .then_with(|| x.b_slug.cmp(&y.b_slug))
    });
    pairs
}

/// Split text into normalized tokens: lowercase, split on any
/// non-alphanumeric boundary, drop short tokens and stop-words, then
/// intern to a sorted, deduplicated list of integer IDs.
fn tokenize(text: &str, interner: &mut Interner) -> Vec<u32> {
    let mut ids: Vec<u32> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .filter(|t| t.chars().count() >= MIN_TOKEN_LEN && !STOPWORD_SET.contains(t.as_str()))
        .map(|t| interner.intern(t))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Jaccard similarity: |A ∩ B| / |A ∪ B|, computed by merging two
/// sorted, deduplicated ID slices. Two empty sets score 0 (no evidence
/// of similarity), not 1.
fn jaccard(a: &[u32], b: &[u32]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut inter) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
        }
    }
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
            closed_by: None,
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
        // Renormalization: with no body/labels on either side, a perfect
        // title twin scores ~1.0, not the W_TITLE cap (0.60).
        assert!((m[0].score - 1.0).abs() < 1e-9, "got {}", m[0].score);
    }

    #[test]
    fn two_letter_acronyms_survive_tokenization() {
        // `ui` / `db` must not be dropped as "too short" — they carry
        // real signal in software issues.
        let a = issue("a-a", "ui crash on resize", "", &[]);
        let b = issue("b-b", "ui crash after resize", "", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1, "shared 'ui'/'crash'/'resize' should match");
        assert!(m[0].title_overlap > 0.4);
    }

    #[test]
    fn unrelated_issues_score_zero() {
        let a = issue("a-a", "Login redirect loop", "auth session cookie", &[]);
        let b = issue(
            "b-b",
            "Export CSV pagination",
            "report download offset",
            &[],
        );
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
        let m = find_duplicates(&target, vec![&weak, &strong], 0.0001);
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
        assert!(
            m.is_empty(),
            "stop-words must not manufacture a match: {m:?}"
        );
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
    fn unicode_tokens_are_lowercased_consistently() {
        // Pins the `to_lowercase()` (Unicode, not ASCII) semantics so the
        // interner can't silently diverge on non-ASCII titles.
        let a = issue("a-a", "café résumé", "", &[]);
        let b = issue("b-b", "CAFÉ Résumé", "", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1);
        assert!((m[0].title_overlap - 1.0).abs() < 1e-9);
    }

    #[test]
    fn repeated_tokens_within_issue_do_not_inflate_overlap() {
        // Deduplication within a single issue must match set semantics:
        // "bug" repeated three times is one token, so identical titles
        // still score a perfect 1.0 (not >1, not diluted).
        let a = issue("a-a", "bug bug bug crash", "", &[]);
        let b = issue("b-b", "bug crash", "", &[]);
        let m = find_duplicates(&a, std::slice::from_ref(&b), DEFAULT_THRESHOLD);
        assert_eq!(m.len(), 1);
        assert!(
            (m[0].title_overlap - 1.0).abs() < 1e-9,
            "got {}",
            m[0].title_overlap
        );
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
