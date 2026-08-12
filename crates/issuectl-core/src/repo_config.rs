//! Read-side abstraction over "where do `schema` and `transitions`
//! come from for this call?".
//!
//! Every mutate (and read) entry point that consults config takes a
//! `&dyn ConfigSource` parameter so the schema/transitions load reaches
//! the load site through the type signature rather than ambient state.
//! The CLI passes [`UncachedConfig`], which re-parses the YAML on every
//! call — cheap for a short-lived process. (A caching implementation
//! backing the long-running web server used to live here; it was
//! removed with the web UI in 0.10.0.)

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::schema::{self, Schema};
use crate::transitions::{self, TransitionRules};

/// Read-side abstraction over "where do `schema` and `transitions`
/// come from for this call?". The sole implementation is
/// [`UncachedConfig`] (re-parse on every call — fine for short-lived
/// CLI commands). The trait is kept as the load-site seam: every
/// mutate entry point takes a `&dyn ConfigSource` parameter, so the
/// config load reaches the load site through the function signature.
pub trait ConfigSource: Send + Sync {
    /// Return a snapshot of the parsed schema for `root`.
    fn schema(&self, root: &Path) -> Result<Arc<Schema>>;
    /// Return a snapshot of the parsed transition rules for `root`.
    fn rules(&self, root: &Path) -> Result<Arc<TransitionRules>>;
}

/// Always-re-parse implementation of [`ConfigSource`]. Each method
/// call hits disk and parses the YAML; there is no memoization
/// within a single `UncachedConfig` instance.
///
/// Intended for short-lived CLI commands where total YAML parse cost
/// is negligible. CLI handlers do occasionally call `schema()` or
/// `rules()` more than once per command (`note_issue` and similar
/// route through `validate_against_schema` and `transition_warnings`
/// helpers that each take their own load); the cost is acceptable
/// because the schema YAML is small (~2 KB) and the CLI process
/// exits immediately after. If a code path ever wants memoization
/// within one command, build a dedicated `MemoizingConfig` rather
/// than mutating this one — the name is load-bearing.
///
/// Zero-sized; construct as `UncachedConfig` directly.
pub struct UncachedConfig;

impl ConfigSource for UncachedConfig {
    fn schema(&self, root: &Path) -> Result<Arc<Schema>> {
        Ok(Arc::new(schema::load_uncached(root)?))
    }
    fn rules(&self, root: &Path) -> Result<Arc<TransitionRules>> {
        Ok(Arc::new(transitions::load_uncached(root)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn uncached_config_parses_schema_and_rules_on_every_call() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("issues")).unwrap();
        let cfg = UncachedConfig;

        // Two consecutive calls return distinct `Arc`s — UncachedConfig
        // does not memoise. Same root, but a fresh parse each time.
        let a = cfg.schema(root).unwrap();
        let b = cfg.schema(root).unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        let r1 = cfg.rules(root).unwrap();
        let r2 = cfg.rules(root).unwrap();
        assert!(!Arc::ptr_eq(&r1, &r2));
    }
}
