//! Internal implementation crate for the `issuectl` binary.
//!
//! **This crate has no stable public API.** Items are `pub` only so
//! the `issuectl` binary (a sibling crate in the same workspace) can
//! call into them. Any module here may change shape — signatures,
//! type layouts, module locations — between any two releases without
//! notice. Depending on `issuectl-core` directly opts out of semver
//! guarantees.
//!
//! Every module is annotated `#[doc(hidden)]`, so `cargo doc` and
//! docs.rs render only this disclaimer. **`#[doc(hidden)]` is *not*
//! a compiler-enforced visibility barrier** — the modules remain
//! `pub` (the binary crate needs that), and IDE tooling such as
//! rust-analyzer will still surface them in completion. The
//! attribute is a social-contract signal, not enforcement; reaching
//! past it acknowledges the instability contract above.
//!
//! The user-facing CLI surface (and its semver contract) lives in the
//! `issuectl` binary crate.

// The release green gate promotes clippy warnings to errors. Keep the pre-existing
// implementation-style debt below explicit until those refactors are tackled as
// product work rather than as release-infra churn.
#![allow(
    clippy::derivable_impls,
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::into_iter_on_ref,
    clippy::io_other_error,
    clippy::large_enum_variant,
    clippy::manual_find,
    clippy::manual_ok_err,
    clippy::map_entry,
    clippy::question_mark,
    clippy::result_large_err,
    clippy::should_implement_trait,
    clippy::useless_format,
    clippy::while_let_loop
)]

#[doc(hidden)]
pub mod agents;
#[doc(hidden)]
pub mod body;
#[doc(hidden)]
pub mod body_sections;
#[doc(hidden)]
pub mod canonical;
#[doc(hidden)]
pub mod clock;
#[doc(hidden)]
pub mod config;
#[doc(hidden)]
pub mod context;
#[doc(hidden)]
pub mod cycle;
#[doc(hidden)]
pub mod dag;
#[doc(hidden)]
pub mod doctor;
#[doc(hidden)]
pub mod duplicates;
#[doc(hidden)]
pub mod envelope;
#[doc(hidden)]
pub mod epic_tree;
#[doc(hidden)]
pub mod estimate;
#[doc(hidden)]
pub mod fmt;
#[doc(hidden)]
pub mod git_trailers;
#[doc(hidden)]
pub mod help;
#[doc(hidden)]
pub mod hooks;
#[doc(hidden)]
pub mod init;
#[doc(hidden)]
pub mod issue_fields;
#[doc(hidden)]
pub mod item_text;
#[doc(hidden)]
pub mod merge_driver;
#[doc(hidden)]
pub mod migrate_layout;
#[doc(hidden)]
pub mod models;
#[doc(hidden)]
pub mod mutate;
#[doc(hidden)]
pub mod parser;
#[doc(hidden)]
pub mod patch_input;
#[doc(hidden)]
pub mod query;
#[doc(hidden)]
pub mod recurrence;
#[doc(hidden)]
pub mod refs;
#[doc(hidden)]
pub mod repo;
#[doc(hidden)]
pub mod report;
#[doc(hidden)]
pub mod schema;
#[doc(hidden)]
pub mod skill;
#[doc(hidden)]
pub mod slug;
#[doc(hidden)]
pub mod stale;
#[doc(hidden)]
pub mod sync_commits;
#[doc(hidden)]
pub mod transfer;
#[doc(hidden)]
pub mod transitions;
#[doc(hidden)]
pub mod write;
