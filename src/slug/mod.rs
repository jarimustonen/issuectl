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

/// Loose validation: lowercase, kebab, 2–4 segments, all-alpha segments.
pub fn is_valid(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }
    for p in &parts {
        if p.is_empty() {
            return false;
        }
        if !p.chars().all(|c| c.is_ascii_lowercase()) {
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
    fn is_valid_accepts_2_to_4_segments() {
        assert!(is_valid("foo-bar"));
        assert!(is_valid("foo-bar-baz"));
        assert!(is_valid("foo-bar-baz-qux"));
    }

    #[test]
    fn is_valid_rejects_too_few_or_too_many_segments() {
        assert!(!is_valid("foo"));
        assert!(!is_valid("a-b-c-d-e"));
    }

    #[test]
    fn is_valid_rejects_empty_segments() {
        assert!(!is_valid(""));
        assert!(!is_valid("foo-"));
        assert!(!is_valid("-foo"));
        assert!(!is_valid("foo--bar"));
    }

    #[test]
    fn is_valid_rejects_uppercase_or_digits() {
        assert!(!is_valid("Foo-bar"));
        assert!(!is_valid("foo-bar1"));
        assert!(!is_valid("foo-bar-baz_qux"));
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
