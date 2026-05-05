use std::cell::Cell;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod wordlists;

use wordlists::adjectives::ADJECTIVES;
use wordlists::intensifiers::INTENSIFIERS;
use wordlists::nouns::NOUNS;

const COLLISION_RETRY_CAP: usize = 8;

thread_local! {
    static RNG_STATE: Cell<u64> = Cell::new(seed());
}

fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15);
    let pid = std::process::id() as u64;
    let mut s = nanos
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(pid.wrapping_mul(0xBF58476D1CE4E5B9));
    if s == 0 {
        s = 0x9E3779B97F4A7C15;
    }
    s
}

fn next_u64() -> u64 {
    RNG_STATE.with(|cell| {
        let mut x = cell.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if x == 0 {
            x = 0x9E3779B97F4A7C15;
        }
        cell.set(x);
        x
    })
}

fn pick<'a>(words: &'a [&'a str]) -> &'a str {
    let n = words.len() as u64;
    let idx = (next_u64() % n) as usize;
    words[idx]
}

/// Generate a fresh `intensifier-adjective-noun` slug
/// (e.g. `extremely-quiet-otter`).
pub fn generate() -> String {
    format!(
        "{}-{}-{}",
        pick(INTENSIFIERS),
        pick(ADJECTIVES),
        pick(NOUNS)
    )
}

/// Generate a slug that does not collide with an existing directory under
/// `issues/{open,closed}/<slug>/`. Loops up to [`COLLISION_RETRY_CAP`] times.
pub fn generate_unique(repo_root: &Path) -> String {
    for _ in 0..COLLISION_RETRY_CAP {
        let s = generate();
        if !slug_exists(repo_root, &s) {
            return s;
        }
    }
    // Astronomically unlikely with ~1B combinations; fall back to a slug
    // suffixed with the current timestamp.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos:x}", generate())
}

fn slug_exists(repo_root: &Path, slug: &str) -> bool {
    for folder in &["open", "closed"] {
        if repo_root.join("issues").join(folder).join(slug).exists() {
            return true;
        }
    }
    false
}

/// Validate that `s` is a usable slug. The canonical slug shape:
/// - At least two `-`-separated segments (single-word slugs would collide
///   with random `intensifier-adjective-noun` slugs unintentionally).
/// - Each segment non-empty, lowercase ASCII letters and digits only.
/// - No leading, trailing, or consecutive hyphens.
///
/// Random slugs always emit three lowercase-letter segments (a strict
/// subset). User-supplied `--slug` overrides may include digits or extra
/// segments — that's intentional, and `is_valid` accepts them.
pub fn is_valid(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.starts_with('-') || s.ends_with('-') || s.contains("--") {
        return false;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() < 2 {
        return false;
    }
    for p in &parts {
        if p.is_empty() {
            return false;
        }
        if !p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_returns_three_segments() {
        let s = generate();
        let parts: Vec<&str> = s.split('-').collect();
        assert_eq!(parts.len(), 3, "got {s}");
        for p in parts {
            assert!(!p.is_empty());
            assert!(p.chars().all(|c| c.is_ascii_lowercase()));
        }
    }

    #[test]
    fn generate_is_valid() {
        for _ in 0..20 {
            let s = generate();
            assert!(is_valid(&s), "{s} should be valid");
        }
    }

    #[test]
    fn is_valid_accepts_minimum_two_segments() {
        assert!(is_valid("foo-bar"));
        assert!(is_valid("foo-bar-baz"));
        assert!(is_valid("foo-bar-baz-qux"));
        assert!(is_valid("a-b-c-d-e")); // Profile B: no upper bound on segments.
    }

    #[test]
    fn is_valid_rejects_single_segment() {
        assert!(!is_valid("foo"));
        assert!(!is_valid("a"));
    }

    #[test]
    fn is_valid_accepts_digits_in_segments() {
        assert!(is_valid("issue-7-redirect"));
        assert!(is_valid("api-v2"));
    }

    #[test]
    fn is_valid_accepts_purely_numeric_segments() {
        // Profile B: digits anywhere, including stand-alone numeric segments.
        // The legacy `#42` namespace is distinguished by the leading `#`,
        // not by segment shape.
        assert!(is_valid("42-fix"));
        assert!(is_valid("fix-42"));
        assert!(is_valid("2024-q4-plan"));
    }

    #[test]
    fn is_valid_rejects_empty_segments_and_edge_hyphens() {
        assert!(!is_valid(""));
        assert!(!is_valid("foo-"));
        assert!(!is_valid("-foo"));
        assert!(!is_valid("foo--bar"));
    }

    #[test]
    fn is_valid_rejects_uppercase_and_unicode() {
        assert!(!is_valid("Foo-bar"));
        assert!(!is_valid("käyttäjän-virhe"));
        assert!(!is_valid("foo-bar_baz"));
    }

    #[test]
    fn generate_produces_diverse_slugs() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        for _ in 0..50 {
            set.insert(generate());
        }
        assert!(set.len() > 40, "expected highly diverse slugs, got {} unique", set.len());
    }
}
