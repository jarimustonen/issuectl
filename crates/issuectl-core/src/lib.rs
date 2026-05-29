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

#[doc(hidden)]
pub mod agents;
#[doc(hidden)]
pub mod boards;
#[doc(hidden)]
pub mod body_sections;
#[doc(hidden)]
pub mod canonical;
#[doc(hidden)]
pub mod context;
#[doc(hidden)]
pub mod docs;
#[doc(hidden)]
pub mod duplicates;
#[doc(hidden)]
pub mod doctor;
#[doc(hidden)]
pub mod fmt;
#[doc(hidden)]
pub mod git_trailers;
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
pub mod query;
#[doc(hidden)]
pub mod refs;
#[doc(hidden)]
pub mod repo;
#[doc(hidden)]
pub mod repo_config;
#[doc(hidden)]
pub mod schema;
#[doc(hidden)]
pub mod server;
#[doc(hidden)]
pub mod skill;
#[doc(hidden)]
pub mod slug;
#[doc(hidden)]
pub mod sync_commits;
#[doc(hidden)]
pub mod transfer;
#[doc(hidden)]
pub mod transitions;
#[doc(hidden)]
pub mod write;
