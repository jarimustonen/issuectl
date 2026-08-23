use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fresh_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues")).unwrap();
        tmp
    }

    fn seed_issue(root: &Path, _folder: &str, slug: &str, status: &str) -> String {
        // Flat layout: `_folder` retained for test-call-site compatibility
        // but no longer affects on-disk placement.
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!(
                "---\ntype: bug\ncreated: 2026-05-06\nstatus: {status}\npriority: normal\n---\n\n# Title\n",
            ),
        )
        .unwrap();
        let parsed = crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), slug, "open");
        let mut issue = parsed.issue;
        let schema = crate::schema::default_schema();
        issue.folder = crate::repo::folder_for_status(&schema, &issue.status).to_string();
        canonical_hash(&issue)
    }

    #[test]
    fn depend_add_writes_blocked_by_and_normalizes_at_sigil() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "subject-issue-here", "open");
        seed_issue(tmp.path(), "open", "blocker-one-here", "open");
        let req = UpdateIssueRequest {
            add_blocked_by: vec!["@blocker-one-here".into()],
            ..Default::default()
        };
        update_issue(tmp.path(), "subject-issue-here", req).unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/subject-issue-here/item.md")).unwrap();
        // Normalization strips the sigil before writing.
        assert!(
            after.contains("blocked_by:") && after.contains("blocker-one-here"),
            "{after}"
        );
    }

    #[test]
    fn depend_remove_drops_blocker_and_removes_empty_key() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "subject-issue-here", "open");
        seed_issue(tmp.path(), "open", "blocker-one-here", "open");
        let add = UpdateIssueRequest {
            add_blocked_by: vec!["blocker-one-here".into()],
            ..Default::default()
        };
        update_issue(tmp.path(), "subject-issue-here", add).unwrap();
        let remove = UpdateIssueRequest {
            remove_blocked_by: vec!["blocker-one-here".into()],
            ..Default::default()
        };
        update_issue(tmp.path(), "subject-issue-here", remove).unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/subject-issue-here/item.md")).unwrap();
        assert!(
            !after.contains("blocked_by:"),
            "empty list must drop the key: {after}"
        );
    }

    #[test]
    fn depend_rejects_self_blocker() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "loop-target-here", "open");
        let req = UpdateIssueRequest {
            add_blocked_by: vec!["loop-target-here".into()],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "loop-target-here", req).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot block itself"), "got: {msg}");
    }

    #[test]
    fn depend_add_and_remove_overlap_is_conflicting_intent() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "subject-issue-here", "open");
        let req = UpdateIssueRequest {
            add_blocked_by: vec!["blocker-x-here".into()],
            remove_blocked_by: vec!["blocker-x-here".into()],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "subject-issue-here", req).unwrap_err();
        assert!(matches!(err, MutateError::ConflictingIntent(_)));
    }

    #[test]
    fn update_with_fresh_version_succeeds() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "test-slug-one", "open");
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "test-slug-one", req).unwrap();
        assert!(out.version.starts_with("sha256:"));
        let after = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(after.contains("priority: high"));
    }

    #[test]
    fn schema_override_of_built_in_done_to_active_drops_closed_stamp() {
        // A project that re-classifies the built-in `done` as active
        // via `status_classes:` must see closing→active edge clear
        // `closed:`. Pins down the override-permitted policy
        // documented in `schema::status_class`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nstatus_classes:\n  done: active\n",
        )
        .unwrap();
        // Seed an issue that's already at `done` with `closed:` stamped.
        let dir = tmp.path().join("issues/done-active-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: done\nclosed: 2026-05-06\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        // Move it to `done` again as if the project just re-classified
        // the status — actually we bump priority so the file rewrites.
        let req = UpdateIssueRequest {
            status: Patch::Set("done".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "done-active-target", req).unwrap();
        // Now `done` is active, so the lifecycle treats this as
        // active→active → no moved_to_closed, and `closed:` should be
        // dropped because the new status is classified Active.
        assert!(!out.moved_to_closed);
        let after =
            fs::read_to_string(tmp.path().join("issues/done-active-target/item.md")).unwrap();
        assert!(
            !after.contains("closed:"),
            "schema-overridden active `done` must drop closed:; got:\n{after}"
        );
        assert_eq!(out.issue.folder, "open");
    }

    #[test]
    fn update_with_no_enum_status_field_rejects_unknown_status() {
        // Whole-spec replacement of `fields.status` without an `enum:`
        // used to leave `status: pizza` to land. The under-lock
        // `status_universe` belt-and-braces gate now catches it
        // (falls back to built-in all_statuses() when no enum is
        // declared).
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n",
        )
        .unwrap();
        let _v0 = seed_issue(tmp.path(), "open", "no-enum-target", "open");
        let req = UpdateIssueRequest {
            status: Patch::Set("pizza".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "no-enum-target", req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(
                    msg.contains("pizza"),
                    "expected pizza in violation message: {msg}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn update_to_schema_declared_custom_closing_status_stamps_closed() {
        // A project that adds `archived` to its schema's status enum
        // and declares it as closing must get the full lifecycle
        // treatment: `closed:` stamped, folder = "closed",
        // `moved_to_closed` reported. Regression-anchors the
        // schema-derived classifier replacing the static
        // CLOSING_STATUSES list.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, in-progress, archived]\nstatus_classes:\n  archived: closing\n",
        )
        .unwrap();
        let _v0 = seed_issue(tmp.path(), "open", "archive-target", "open");
        let req = UpdateIssueRequest {
            status: Patch::Set("archived".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "archive-target", req).unwrap();
        assert!(
            out.moved_to_closed,
            "active→archived must report moved_to_closed"
        );
        assert_eq!(out.issue.folder, "closed");
        let after = fs::read_to_string(tmp.path().join("issues/archive-target/item.md")).unwrap();
        assert!(after.contains("status: archived"));
        assert!(
            after.contains("closed:"),
            "closed: must be stamped on schema-classified closing status; got:\n{after}"
        );
    }

    #[test]
    fn dod_delivery_policy_flows_through_update_warnings_and_strict_errors() {
        let tmp = fresh_repo();

        // Built-in non-delivery closes remain possible even under strict DoD.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\ndod:\n  strict: true\n",
        )
        .unwrap();
        seed_issue(tmp.path(), "open", "duplicate-target", "open");
        let duplicate = update_issue(
            tmp.path(),
            "duplicate-target",
            UpdateIssueRequest {
                status: Patch::Set("duplicate".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(duplicate.warnings.is_empty());
        assert!(duplicate.moved_to_closed);

        // Declaring only the pre-existing severity knob must inherit the new
        // built-in delivery defaults rather than silently disabling the gate.
        seed_issue(tmp.path(), "open", "fixed-strict-target", "open");
        let err = update_issue(
            tmp.path(),
            "fixed-strict-target",
            UpdateIssueRequest {
                status: Patch::Set("fixed".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, MutateError::TransitionViolation(ref msg) if msg.contains("dod:")),
            "expected strict DoD violation, got {err:?}"
        );

        let custom_schema = |strict| {
            format!(
                "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, shipped]\nstatus_classes:\n  shipped: closing\ndod:\n  strict: {strict}\n  delivery_statuses: [shipped]\n"
            )
        };

        // A project-defined delivery close gets the default warning behavior.
        fs::write(tmp.path().join("issues/.schema.yaml"), custom_schema(false)).unwrap();
        seed_issue(tmp.path(), "open", "shipped-warning-target", "open");
        let warned = update_issue(
            tmp.path(),
            "shipped-warning-target",
            UpdateIssueRequest {
                status: Patch::Set("shipped".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(warned.warnings.len(), 1);
        assert!(warned.warnings[0].contains("dod:"));

        // The same custom declaration blocks before writing in strict mode.
        fs::write(tmp.path().join("issues/.schema.yaml"), custom_schema(true)).unwrap();
        seed_issue(tmp.path(), "open", "shipped-strict-target", "open");
        let err = update_issue(
            tmp.path(),
            "shipped-strict-target",
            UpdateIssueRequest {
                status: Patch::Set("shipped".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::TransitionViolation(ref msg) if msg.contains("dod:")));
        let unchanged =
            fs::read_to_string(tmp.path().join("issues/shipped-strict-target/item.md")).unwrap();
        assert!(unchanged.contains("status: open"));
        assert!(!unchanged.contains("closed:"));
    }

    #[test]
    fn status_write_rejects_empty_closed_on_closing_status() {
        // An issue at a closing status whose `closed:` is empty (an
        // explicit unset). Re-asserting a closing status *touches* the
        // status/closed pair, so the resulting RequiredWhen is one this
        // write is responsible for — it must be rejected, not silently
        // accepted and left for `doctor` to heal.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/empty-closed-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: done\nclosed: \"\"\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("done".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "empty-closed-target", req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(
                    msg.contains("closed"),
                    "expected the closed RequiredWhen in the message: {msg}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_edit_keeps_requiredwhen_exempt_on_closing_status() {
        // The mirror case: the same inconsistent on-disk state, but the
        // mutation only bumps `priority` — it touches neither `status`
        // nor `closed`. Blocking an unrelated edit because of a
        // pre-existing inconsistency the user didn't introduce would be
        // surprising, so the RequiredWhen stays exempt and the write
        // succeeds (doctor owns healing the empty `closed:`).
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/unrelated-edit-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: done\nclosed: \"\"\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "unrelated-edit-target", req).unwrap();
        let after = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(after.contains("priority: high"));
    }

    #[test]
    fn update_with_stale_version_returns_409() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "test-slug-two", "open");
        let req = UpdateIssueRequest {
            expected_version: Some("sha256:deadbeef".into()),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "test-slug-two", req).unwrap_err();
        match err {
            MutateError::VersionMismatch { current, version } => {
                assert_eq!(current.slug, "test-slug-two");
                assert!(version.starts_with("sha256:"));
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn simple_unknown_scalar_fields_survive_read_write_round_trip() {
        // A user-added `triage:` key on a fresh issue with a simple
        // scalar value survives a no-op read→write cycle without
        // textual change, AND lands in `Issue::extra` so
        // canonical_hash sees it. Byte identity is *not* a general
        // contract — `serde_yaml` reformats comments, scalar styles,
        // anchors, list flow style — but for the simple
        // `key: scalar` case it's stable, and that's the case this
        // test pins down.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/keep-triage");
        fs::create_dir_all(&dir).unwrap();
        let original = "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
                        priority: normal\ntriage: alice\nreviewer: bob\n\
                        ---\n\n# Title\n";
        fs::write(dir.join("item.md"), original).unwrap();
        let item = crate::write::read_item(&dir.join("item.md")).unwrap();
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(original, after);

        // The parsed Issue must carry the unknowns into `extra` so
        // canonical_hash sees them.
        let parsed =
            crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), "keep-triage", "open");
        assert_eq!(
            parsed.issue.extra.get("triage"),
            Some(&serde_json::Value::String("alice".into()))
        );
        assert_eq!(
            parsed.issue.extra.get("reviewer"),
            Some(&serde_json::Value::String("bob".into()))
        );
    }

    #[test]
    fn unknown_field_edits_with_refreshed_version_do_not_block_later_updates() {
        // Two external writes land in sequence on different custom
        // keys (`triage:` then `reviewer:`); the third writer takes
        // the post-edit version and PATCHes a known field. No 409
        // because the third writer didn't carry a stale view. This
        // does NOT prove field-level merge — whole-document
        // optimistic concurrency means a writer that *was* stale on
        // either custom key would still 409 (covered separately by
        // `external_edit_to_unknown_field_makes_stale_version_409`).
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/concurrent-distinct");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# Title\n",
        )
        .unwrap();

        // Edit #1: external writer adds `triage: alice`. Then a
        // mutate.rs PATCH with the *post-external-edit* version
        // succeeds — no 409 because we picked up the new hash first.
        let v0 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-distinct",
                "open",
            )
            .issue,
        );
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        item.frontmatter.insert(
            serde_yaml::Value::String("triage".into()),
            serde_yaml::Value::String("alice".into()),
        );
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();
        let v1 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-distinct",
                "open",
            )
            .issue,
        );
        assert_ne!(v0, v1, "adding unknown key must change the hash");

        // Edit #2: another external writer adds `reviewer: bob` while
        // *holding the fresh* v1, then `issuectl update --priority high
        // --expected-version v2` lands cleanly.
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        item.frontmatter.insert(
            serde_yaml::Value::String("reviewer".into()),
            serde_yaml::Value::String("bob".into()),
        );
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();
        let v2 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-distinct",
                "open",
            )
            .issue,
        );
        assert_ne!(v1, v2);

        let req = UpdateIssueRequest {
            expected_version: Some(v2),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "concurrent-distinct", req).unwrap();
        assert_eq!(out.issue.priority, "high");
        // Both unknown keys must survive the mutation round-trip.
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("triage: alice"));
        assert!(on_disk.contains("reviewer: bob"));
    }

    #[test]
    fn external_edit_to_unknown_field_makes_stale_version_409() {
        // The contract: an unknown field changing under a writer's
        // feet must trip optimistic concurrency the same way a known
        // field would. Without unknown-key projection in
        // `canonical_hash`, this PATCH would silently succeed and
        // could clobber a custom field the writer never read.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/concurrent-same-key");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\ntriage: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let v0 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-same-key",
                "open",
            )
            .issue,
        );

        // External writer overwrites the same unknown key.
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        item.frontmatter.insert(
            serde_yaml::Value::String("triage".into()),
            serde_yaml::Value::String("bob".into()),
        );
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();

        // Original writer comes back with v0 — must 409.
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "concurrent-same-key", req).unwrap_err();
        match err {
            MutateError::VersionMismatch { current, .. } => {
                assert_eq!(current.slug, "concurrent-same-key");
                assert_eq!(
                    current.extra.get("triage"),
                    Some(&serde_json::Value::String("bob".into())),
                    "current state surfaced to the caller must reflect the new unknown value"
                );
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn external_delete_of_unknown_field_makes_stale_version_409() {
        // Symmetric to the same-key 409 test: a writer who saved
        // `triage: alice` and didn't notice an external `git pull`
        // wiped the key must still trip optimistic concurrency,
        // because removal changes the hash too.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/concurrent-delete-key");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\ntriage: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let v0 = crate::canonical::canonical_hash(
            &crate::parser::parse_item_md_with_warnings(
                &dir.join("item.md"),
                "concurrent-delete-key",
                "open",
            )
            .issue,
        );

        // External writer removes the unknown key entirely.
        let mut item = crate::write::read_item(&dir.join("item.md")).unwrap();
        crate::write::remove_key(&mut item.frontmatter, "triage");
        crate::write::write_item(&dir.join("item.md"), &item).unwrap();

        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "concurrent-delete-key", req).unwrap_err();
        assert!(
            matches!(err, MutateError::VersionMismatch { .. }),
            "expected VersionMismatch on stale view after unknown-key delete, got {err:?}"
        );
    }

    #[test]
    fn non_string_nested_key_in_unknown_value_warns_not_panics() {
        // YAML allows non-string mapping keys; JSON does not. The
        // parser must surface that as a `MutateError::Corrupt`
        // (carrying the warning) rather than letting the hash code
        // panic on a `serde_json::to_value` failure.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/bad-nested-key");
        fs::create_dir_all(&dir).unwrap();
        // `? [1, 2]` is YAML's explicit-key syntax for a sequence
        // key. Top-level keys are still strings (so the frontmatter
        // parses); the offending non-string key lives inside
        // `weird:`.
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nweird:\n  ? [1, 2]\n  : foo\n---\n\n# Title\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "bad-nested-key", req).unwrap_err();
        match err {
            MutateError::Corrupt { warnings } => {
                assert!(
                    warnings.iter().any(|w| w.contains("weird")
                        && (w.contains("string") || w.contains("mapping key"))),
                    "expected a warning naming the bad key, got: {warnings:?}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn status_only_patch_leaves_other_fields_untouched() {
        // A status-only update touches `status` alone. Other fields
        // (priority, assignee, epic, …) must round-trip unchanged via
        // `Patch::Unspecified` — without this a status change would
        // silently clobber the rest of the frontmatter.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/dnd-status-only");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: high\nassignee: alice\nepic: roadmap\n---\n\n# Title\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("in-progress".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dnd-status-only", req).unwrap();
        assert_eq!(out.issue.status, "in-progress");
        assert_eq!(out.issue.priority, "high");
        assert_eq!(out.issue.assignee.as_deref(), Some("alice"));
        assert_eq!(out.issue.epic.as_deref(), Some("roadmap"));
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: in-progress"));
        assert!(on_disk.contains("priority: high"));
        assert!(on_disk.contains("assignee: alice"));
        assert!(on_disk.contains("epic: roadmap"));
    }

    #[test]
    fn reopening_a_closed_issue_clears_closed_date() {
        // Reopening moves an issue from a closed status back to an
        // active status. The frontmatter `closed:` date must be removed
        // in the same write so the issue isn't left in a contradictory
        // "status: open, closed: 2026-01-01" state.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/reopen-me");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-01\nstatus: fixed\n\
             priority: normal\nclosed: 2026-05-05\n---\n\n# Title\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "reopen-me", req).unwrap();
        assert!(out.moved_to_open);
        assert_eq!(out.issue.status, "open");
        assert!(
            out.issue.closed.is_none(),
            "closed: must be cleared on reopen"
        );
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: open"));
        assert!(
            !on_disk.contains("closed:"),
            "frontmatter must not retain closed: after reopen, got:\n{on_disk}"
        );
    }

    /// Seed an issue directly into cold storage at
    /// `issues/archive/YYYY/MM/<slug>/item.md`, as the `archive` verb
    /// would leave it.
    fn seed_archived(root: &Path, year: &str, month: &str, slug: &str, body: &str) -> PathBuf {
        let dir = root
            .join("issues/archive")
            .join(year)
            .join(month)
            .join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
        dir
    }

    #[test]
    fn reopening_archived_issue_lifts_it_out_of_cold_storage() {
        // The arch-stale-archive feature moves closed issues to
        // issues/archive/YYYY/MM/<slug>/. Reopening one (closing→active)
        // must move the directory back to the active root, else the issue
        // reads as active in list/show while physically still archived.
        let tmp = fresh_repo();
        let archived_dir = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "old-archived-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# Title\n",
        );

        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "old-archived-fox", req).unwrap();

        assert!(out.moved_to_open);
        assert_eq!(out.issue.status, "open");
        // Physically relocated to the active root.
        let active_dir = tmp.path().join("issues/old-archived-fox");
        assert_eq!(out.issue_dir, active_dir, "issue_dir must be active root");
        assert!(
            active_dir.join("item.md").is_file(),
            "active copy must exist"
        );
        assert!(!archived_dir.exists(), "archive copy must be gone");
        let on_disk = fs::read_to_string(active_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: open"));
        assert!(!on_disk.contains("closed:"), "closed: cleared on reopen");
        // No leftover empty archive month/year/root tree is required, but
        // the slug dir itself must not linger.
        assert!(!tmp
            .path()
            .join("issues/archive/2020/01/old-archived-fox")
            .exists());
    }

    #[test]
    fn reopening_archived_issue_via_close_status_change_stays_in_archive() {
        // Changing one closing status to another (fixed→wontfix) is NOT a
        // reopen: the issue stays closed and must remain in cold storage.
        let tmp = fresh_repo();
        let archived_dir = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "still-closed-elk",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# Title\n",
        );

        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "still-closed-elk", req).unwrap();

        assert!(!out.moved_to_open);
        assert!(archived_dir.join("item.md").is_file(), "stays archived");
        assert!(
            !tmp.path().join("issues/still-closed-elk").exists(),
            "must not appear in active root"
        );
        // Historical close date preserved on closing→closing.
        let on_disk = fs::read_to_string(archived_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: wontfix"));
        assert!(on_disk.contains("closed: 2020-01-01"));
    }

    #[test]
    fn reopening_archived_issue_refuses_when_active_copy_exists() {
        // Defence in depth: a slug present both active and archived is
        // Ambiguous and fails the read-time locate, but if it somehow got
        // past, the unarchive move must refuse rather than clobber.
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "dup-slug-newt",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# Title\n",
        );
        // Also seed an active copy so the locate is Ambiguous.
        let active = tmp.path().join("issues/dup-slug-newt");
        fs::create_dir_all(&active).unwrap();
        fs::write(
            active.join("item.md"),
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "dup-slug-newt", req).unwrap_err();
        // Ambiguous locate fires before the write; either way both copies
        // survive untouched.
        assert!(matches!(err, MutateError::AmbiguousSlug { .. }));
        assert!(tmp
            .path()
            .join("issues/archive/2020/01/dup-slug-newt/item.md")
            .is_file());
        assert!(active.join("item.md").is_file());
    }

    #[test]
    fn unarchive_if_active_is_noop_when_post_status_closing() {
        // A mutation that leaves the issue closing (post_closing == true)
        // on an archived issue must leave the path untouched.
        let tmp = fresh_repo();
        let item = tmp
            .path()
            .join("issues/archive/2020/01/keep-archived-owl/item.md");
        let out = unarchive_if_active(tmp.path(), "keep-archived-owl", item.clone(), true).unwrap();
        assert_eq!(
            out, item,
            "still-closing leaves the archived path unchanged"
        );
    }

    #[test]
    fn unarchive_if_active_refuses_when_active_copy_exists() {
        // Defence-in-depth branch: exercised directly because the normal
        // entry points reject the active+archived collision as Ambiguous
        // before this helper runs.
        let tmp = fresh_repo();
        let archived = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "collide-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\npriority: normal\n---\n# T\n",
        );
        let active = tmp.path().join("issues/collide-fox");
        fs::create_dir_all(&active).unwrap();
        let err = unarchive_if_active(tmp.path(), "collide-fox", archived.join("item.md"), false)
            .unwrap_err();
        assert!(matches!(err, MutateError::Io(_)));
        assert!(archived.join("item.md").exists(), "archive copy untouched");
    }

    #[test]
    fn dry_run_reopen_of_archived_issue_predicts_active_dir() {
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "dry-reopen-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dry-reopen-fox", req).unwrap();
        assert_eq!(out.issue_dir, tmp.path().join("issues/dry-reopen-fox"));
        // Dry-run wrote nothing: still physically archived.
        assert!(tmp
            .path()
            .join("issues/archive/2020/01/dry-reopen-fox/item.md")
            .is_file());
        assert!(!tmp.path().join("issues/dry-reopen-fox").exists());
    }

    #[test]
    fn dry_run_non_reopen_of_archived_issue_predicts_archive_dir() {
        // A priority patch that keeps the issue closing must report the
        // archive dir, matching where a real write would land.
        let tmp = fresh_repo();
        let archived = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "dry-stay-elk",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dry-stay-elk", req).unwrap();
        assert_eq!(
            out.issue_dir, archived,
            "non-reopen dry-run must predict the archive dir"
        );
    }

    #[test]
    fn unarchive_prunes_emptied_month_and_year_buckets() {
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "lonely-newt",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "lonely-newt", req).unwrap();
        // The slug was the only occupant, so its month/year buckets prune.
        assert!(!tmp.path().join("issues/archive/2020/01").exists());
        assert!(!tmp.path().join("issues/archive/2020").exists());
    }

    #[test]
    fn unarchive_keeps_bucket_with_other_archived_siblings() {
        let tmp = fresh_repo();
        seed_archived(
            tmp.path(),
            "2020",
            "01",
            "reopen-this-fox",
            "---\ntype: bug\ncreated: 2020-01-01\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-01\n---\n\n# T\n",
        );
        let sibling = seed_archived(
            tmp.path(),
            "2020",
            "01",
            "stay-put-owl",
            "---\ntype: bug\ncreated: 2020-01-15\nstatus: fixed\n\
             priority: normal\nclosed: 2020-01-15\n---\n\n# T\n",
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "reopen-this-fox", req).unwrap();
        // Bucket still holds the sibling, so it must NOT be pruned.
        assert!(tmp.path().join("issues/archive/2020/01").exists());
        assert!(sibling.join("item.md").is_file());
    }

    #[test]
    fn update_status_to_closing_does_not_move_directory() {
        // M14: use inode comparison rather than `created()` (which is
        // Err on most Linux ext4 setups, silently making the assertion
        // a no-op).
        use std::os::unix::fs::MetadataExt;
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "close-me-now", "open");
        let flat_dir = tmp.path().join("issues/close-me-now");
        let before_inode = fs::metadata(&flat_dir).unwrap().ino();
        let req = UpdateIssueRequest {
            status: Patch::Set("fixed".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "close-me-now", req).unwrap();
        // Status transition flag still flips, but the dir does not move.
        assert!(out.moved_to_closed);
        assert_eq!(out.issue_dir, flat_dir);
        assert!(flat_dir.is_dir(), "flat dir must still exist");
        assert!(!tmp.path().join("issues/closed/close-me-now").exists());
        assert!(!tmp.path().join("issues/open/close-me-now").exists());
        let after_inode = fs::metadata(&flat_dir).unwrap().ino();
        assert_eq!(
            before_inode, after_inode,
            "directory must not have been recreated"
        );
        let on_disk = fs::read_to_string(flat_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("status: fixed"));
        assert!(on_disk.contains("closed:"));
    }

    #[test]
    fn empty_patch_is_noop_no_legacy_migration() {
        // M13: an empty PATCH against a legacy-path issue must NOT
        // migrate the directory or bump `updated:`. The version returned
        // matches what `show --json` would have read.
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/empty-patch-legacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\nupdated: 2026-01-01\n---\n\n# T\n",
        )
        .unwrap();
        let before = fs::read_to_string(legacy.join("item.md")).unwrap();

        let req = UpdateIssueRequest::default();
        let out = update_issue(tmp.path(), "empty-patch-legacy", req).unwrap();

        // Legacy directory is preserved (no migration on a no-op).
        assert!(legacy.is_dir(), "legacy dir must remain untouched");
        let after = fs::read_to_string(legacy.join("item.md")).unwrap();
        assert_eq!(before, after, "no-op must not touch item.md");
        assert!(out.version.starts_with("sha256:"));
    }

    #[test]
    fn closing_to_closing_preserves_closed_date() {
        // C2: fixed → wontfix must preserve the original `closed:` date.
        // Overwriting it silently destroys historical close provenance.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/preserve-closed-date");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: fixed\nclosed: 2026-01-15\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "preserve-closed-date", req).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("closed: 2026-01-15"), "got:\n{after}");
        assert!(after.contains("status: wontfix"));
    }

    #[test]
    fn closing_backfills_closed_date_when_missing() {
        // Closing→closing on an issue that pre-dates auto-stamping should
        // backfill rather than leave the field empty.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/backfill-closed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: fixed\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "backfill-closed", req).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(
            after.contains("closed:"),
            "expected backfilled closed date in:\n{after}"
        );
    }

    #[test]
    fn legacy_path_is_migrated_in_place_on_write() {
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/legacy-one-here");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let v0 = {
            let parsed = crate::parser::parse_item_md_with_warnings(
                &legacy.join("item.md"),
                "legacy-one-here",
                "open",
            );
            canonical_hash(&parsed.issue)
        };
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "legacy-one-here", req).unwrap();
        assert!(out
            .issue_dir
            .to_string_lossy()
            .ends_with("issues/legacy-one-here"));
        assert!(!legacy.exists(), "legacy dir must be gone after write");
    }

    #[test]
    fn ambiguous_layout_is_rejected() {
        let tmp = fresh_repo();
        // Both flat and legacy versions of the same slug exist.
        let flat = tmp.path().join("issues/dual-path-here");
        fs::create_dir_all(&flat).unwrap();
        fs::write(
            flat.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let legacy = tmp.path().join("issues/open/dual-path-here");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "dual-path-here", req).unwrap_err();
        assert!(matches!(err, MutateError::AmbiguousSlug { .. }));
    }

    #[test]
    fn patch_clear_removes_field() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/has-epic-here");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\nepic: foo-bar\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            epic: Patch::Clear,
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "has-epic-here", req).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(!after.contains("epic:"));
    }

    #[test]
    fn patch_unspecified_does_not_touch_field() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/keep-epic-as-is");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\nepic: stay-here\n---\n\n# T\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "keep-epic-as-is", req).unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("epic: stay-here"));
        assert!(after.contains("priority: high"));
    }

    #[test]
    fn add_and_remove_label_overlap_rejected() {
        let req = UpdateIssueRequest {
            add_labels: vec!["x".into()],
            remove_labels: vec!["x".into()],
            ..Default::default()
        };
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::ConflictingIntent(_)));
    }

    #[test]
    fn deserialize_unspecified_clear_set() {
        // Field absent → Unspecified, default-derived
        let r: UpdateIssueRequest = serde_json::from_str("{}").unwrap();
        assert!(matches!(r.epic, Patch::Unspecified));

        // null → Clear
        let r: UpdateIssueRequest = serde_json::from_str(r#"{"epic": null}"#).unwrap();
        assert!(matches!(r.epic, Patch::Clear));

        // string → Set
        let r: UpdateIssueRequest = serde_json::from_str(r#"{"epic": "foo"}"#).unwrap();
        assert!(matches!(r.epic, Patch::Set(ref s) if s == "foo"));
    }

    #[test]
    fn deserialize_rejects_unknown_field() {
        let result: Result<UpdateIssueRequest, _> = serde_json::from_str(r#"{"priorty": "high"}"#);
        assert!(result.is_err(), "typo'd field must be rejected");
    }

    #[test]
    fn empty_string_set_rejected() {
        let req = UpdateIssueRequest {
            epic: Patch::Set("".into()),
            ..Default::default()
        };
        assert!(matches!(req.validate(), Err(MutateError::Validation(_))));
    }

    #[test]
    fn update_sets_custom_field_via_patch() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "cf-set", "open");
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("P1".into()));
        let out = update_issue(tmp.path(), "cf-set", req).unwrap();
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("triage: P1"), "got: {on_disk}");
        assert_eq!(
            out.issue.extra.get("triage"),
            Some(&serde_json::Value::String("P1".into()))
        );
    }

    #[test]
    fn update_clears_custom_field_via_null_patch() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/cf-clear");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nowner_team: payments\n---\n\n# T\n",
        )
        .unwrap();
        let mut req = UpdateIssueRequest::default();
        req.custom_fields.insert("owner_team".into(), Patch::Clear);
        let out = update_issue(tmp.path(), "cf-clear", req).unwrap();
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(
            !on_disk.contains("owner_team:"),
            "owner_team should be removed; got: {on_disk}"
        );
    }

    #[test]
    fn update_custom_field_set_and_clear_atomic() {
        // JSON `{"custom_fields": {"triage": "P1", "owner_team": null}}`
        // sets one key and removes another in a single PATCH.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/cf-mixed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nowner_team: payments\n---\n\n# T\n",
        )
        .unwrap();
        let req: UpdateIssueRequest =
            serde_json::from_str(r#"{"custom_fields": {"triage": "P1", "owner_team": null}}"#)
                .unwrap();
        let out = update_issue(tmp.path(), "cf-mixed", req).unwrap();
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("triage: P1"));
        assert!(!on_disk.contains("owner_team:"));
    }

    #[test]
    fn update_custom_field_rejects_reserved_key() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("status".into(), Patch::Set("done".into()));
        let err = req.validate().unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(msg.contains("built-in"), "got: {msg}"),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_custom_field_rejects_invalid_key_shape() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("bad key!".into(), Patch::Set("x".into()));
        assert!(matches!(req.validate(), Err(MutateError::Validation(_))));
    }

    #[test]
    fn update_custom_field_rejects_empty_string_set() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("".into()));
        let err = req.validate().unwrap_err();
        assert!(
            matches!(err, MutateError::Validation(ref m) if m.contains("empty-string")),
            "got: {err:?}"
        );
    }

    #[test]
    fn update_custom_field_violating_schema_is_rejected() {
        // Schema declares `triage` required + enum; the PATCH supplies a
        // value outside the enum. Post-mutation schema validation must
        // 422 it (no on-disk change).
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  triage:\n    enum: [P0, P1, P2]\n",
        )
        .unwrap();
        let _v0 = seed_issue(tmp.path(), "open", "cf-schema", "open");
        let dir = tmp.path().join("issues/cf-schema");
        let before = fs::read_to_string(dir.join("item.md")).unwrap();
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("P9".into()));
        let err = update_issue(tmp.path(), "cf-schema", req).unwrap_err();
        assert!(
            matches!(err, MutateError::SchemaViolation(_)),
            "got: {err:?}"
        );
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(before, after, "schema-rejected PATCH must not write");
    }

    #[test]
    fn fixed_clock_stamps_closed_and_updated_dates() {
        use chrono::TimeZone;

        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "clocked-close", "open");
        let clock = crate::clock::FixedClock::new(
            chrono::Utc.with_ymd_and_hms(2026, 1, 31, 12, 0, 0).unwrap(),
        );
        let req = UpdateIssueRequest {
            status: Patch::Set("fixed".into()),
            ..Default::default()
        };
        update_issue_via(tmp.path(), "clocked-close", req, &clock).unwrap();
        let text = fs::read_to_string(tmp.path().join("issues/clocked-close/item.md")).unwrap();
        assert!(text.contains("closed: 2026-01-31"));
        assert!(text.contains("updated: 2026-01-31"));
    }

    #[test]
    fn update_custom_field_bumps_canonical_version() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "cf-bump", "open");
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("P1".into()));
        let out = update_issue(tmp.path(), "cf-bump", req).unwrap();
        assert_ne!(v0, out.version, "custom-field PATCH must change the hash");
    }

    #[test]
    fn update_custom_field_with_stale_version_returns_409() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "cf-stale", "open");
        let mut req = UpdateIssueRequest {
            expected_version: Some("sha256:deadbeef".into()),
            ..Default::default()
        };
        req.custom_fields
            .insert("triage".into(), Patch::Set("P1".into()));
        let err = update_issue(tmp.path(), "cf-stale", req).unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }));
    }

    #[test]
    fn update_custom_field_repairs_missing_required_schema_field() {
        // The motivating bug: a schema introduces a required custom
        // field, an existing issue lacks it, and every PATCH 422s on
        // SchemaViolation. The fix is exactly that the same PATCH can
        // SUPPLY the missing field via `custom_fields` and succeed.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let _ = seed_issue(tmp.path(), "open", "cf-required-repair", "open");

        // Sanity: a no-custom-field PATCH is rejected.
        let err = update_issue(
            tmp.path(),
            "cf-required-repair",
            UpdateIssueRequest {
                priority: Patch::Set("high".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, MutateError::SchemaViolation(_)),
            "expected SchemaViolation without team set, got {err:?}"
        );

        // The repair PATCH supplies the missing key.
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("team".into(), Patch::Set("payments".into()));
        let out = update_issue(tmp.path(), "cf-required-repair", req).unwrap();
        assert_eq!(
            out.issue.extra.get("team"),
            Some(&serde_json::Value::String("payments".into()))
        );
    }

    #[test]
    fn update_custom_field_rejects_whitespace_only_set() {
        // `--field key=" "` is rejected by the CLI parser; the API
        // path must reject it too so a JSON client cannot smuggle a
        // blank value past `validate()`.
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set("   ".into()));
        let err = req.validate().unwrap_err();
        assert!(
            matches!(err, MutateError::Validation(ref m) if m.contains("empty-string")),
            "got: {err:?}"
        );
    }

    #[test]
    fn update_custom_field_rejects_padded_set() {
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("triage".into(), Patch::Set(" P1".into()));
        assert!(matches!(req.validate(), Err(MutateError::Validation(_))));
    }

    #[test]
    fn deserialize_custom_fields_supports_set_clear() {
        let r: UpdateIssueRequest =
            serde_json::from_str(r#"{"custom_fields": {"a": "x", "b": null}}"#).unwrap();
        assert!(matches!(r.custom_fields.get("a"), Some(Patch::Set(s)) if s == "x"));
        assert!(matches!(r.custom_fields.get("b"), Some(Patch::Clear)));
    }

    #[test]
    fn update_request_rejects_duplicate_custom_field_keys_at_deserialization() {
        // Sister of the create-path duplicate-key rejection. Without a
        // custom visitor, BTreeMap silently keeps whichever value
        // serde_json saw last — this test pins the wire-level rejection.
        let payload = r#"{"custom_fields":{"team":"a","team":null}}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(payload).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-key rejection, got {msg:?}"
        );
    }

    #[test]
    fn validate_custom_field_key_rejects_invalid_shape_and_reserved() {
        // Single source of truth for the create/update paths.
        // Each call site (new/update) wraps this in its own
        // typed error variant, so the message must stay stable.
        assert!(validate_custom_field_key("team").is_ok());
        let err = validate_custom_field_key("bad key").unwrap_err();
        assert!(err.contains("alphanumeric"), "shape rejection: {err:?}");
        let err = validate_custom_field_key("status").unwrap_err();
        assert!(err.contains("built-in"), "reserved rejection: {err:?}");
    }

    #[test]
    fn update_request_validate_rejects_reserved_custom_field_key() {
        // CLI-update + API-update share `UpdateIssueRequest::validate`.
        // Routing through `validate_custom_field_key` keeps the error
        // text identical to the new-path rejection.
        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("status".into(), Patch::Set("ignored".into()));
        let err = req.validate().unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("status") && msg.contains("built-in"),
                "expected built-in rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_api_rejects_reserved_custom_field_key() {
        // API new path: previously accepted `{"custom_fields":
        // {"status":"…"}}` and let frontmatter-render ordering mask the
        // damage. Now `do_new_locked` runs the shared validator before
        // building the in-memory frontmatter, so the API surfaces the
        // same MutateError::Validation as the update path.
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Sneaky".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("status".into(), "fake".into())];
        let err = new_issue(tmp.path(), req).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("status") && msg.contains("built-in"),
                "expected built-in rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn validate_custom_field_value_rejects_blank_and_padded() {
        assert!(validate_custom_field_value("team", "payments").is_ok());
        let err = validate_custom_field_value("team", "   ").unwrap_err();
        assert!(err.contains("empty-string Set"), "blank: {err:?}");
        let err = validate_custom_field_value("team", " payments").unwrap_err();
        assert!(err.contains("whitespace"), "leading ws: {err:?}");
        let err = validate_custom_field_value("team", "payments ").unwrap_err();
        assert!(err.contains("whitespace"), "trailing ws: {err:?}");
    }

    #[test]
    fn new_issue_api_rejects_whitespace_only_custom_field_value() {
        // Closes the value-validation asymmetry: API update already
        // rejected `{"team":"   "}` via UpdateIssueRequest::validate,
        // and CLI new rejected it via parse_custom_field. API new used
        // to slip blank values through to frontmatter — now both key
        // and value go through the shared validators inside
        // `do_new_locked`.
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Blank value".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("team".into(), "   ".into())];
        let err = new_issue(tmp.path(), req).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("team") && msg.contains("empty-string Set"),
                "expected blank-value rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_api_rejects_padded_custom_field_value() {
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Padded value".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("team".into(), " payments".into())];
        let err = new_issue(tmp.path(), req).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("team") && msg.contains("whitespace"),
                "expected whitespace rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_api_rejects_invalid_custom_field_key_shape() {
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Bad shape".into();
        req.priority = "normal".into();
        req.custom_fields = vec![("bad key".into(), "x".into())];
        let err = new_issue(tmp.path(), req).unwrap_err();
        match err {
            MutateError::Validation(msg) => assert!(
                msg.contains("bad key") && msg.contains("alphanumeric"),
                "expected shape rejection, got {msg:?}"
            ),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_body_roundtrip_advances_version() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "body-roundtrip-x", "open");
        let out = update_body(
            tmp.path(),
            "body-roundtrip-x",
            Some(v0.clone()),
            "# rewrite\n\nnew body".into(),
            false,
        )
        .unwrap();
        assert!(out.version.starts_with("sha256:"));
        assert_ne!(out.version, v0);
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("new body"));
    }

    #[test]
    fn update_issue_title_rewrites_body_h1_and_advances_version() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "retitle-happy-x", "open");
        let req = UpdateIssueRequest {
            title: Patch::Set("A clearer title".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "retitle-happy-x", req).unwrap();
        assert_ne!(out.version, v0, "retitle must advance the version");
        assert_eq!(out.issue.title, "A clearer title");
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("\n# A clearer title\n"), "{on_disk}");
        assert!(!on_disk.contains("\n# Title\n"), "{on_disk}");
    }

    #[test]
    fn update_body_headingless_preserves_existing_title_and_warns() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "body-preserve-title-x", "open");
        let out = update_body(
            tmp.path(),
            "body-preserve-title-x",
            None,
            "Replacement without a heading".into(),
            false,
        )
        .unwrap();
        assert_eq!(out.issue.title, "Title");
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("no non-empty title H1") && w.contains("preserved")),
            "warnings: {:?}",
            out.warnings
        );
        assert!(out.issue.body.contains("Replacement without a heading"));
    }

    #[test]
    fn update_body_same_h1_has_no_title_warning() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "body-same-title-x", "open");
        let out = update_body(
            tmp.path(),
            "body-same-title-x",
            None,
            "# Title\n\nReplacement body".into(),
            false,
        )
        .unwrap();
        assert!(out.warnings.is_empty(), "warnings: {:?}", out.warnings);
    }

    #[test]
    fn update_body_different_h1_changes_title_and_warns() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "body-change-title-x", "open");
        let out = update_body(
            tmp.path(),
            "body-change-title-x",
            None,
            "# Replacement title\n\nReplacement body".into(),
            false,
        )
        .unwrap();
        assert_eq!(out.issue.title, "Replacement title");
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("changed the title") && w.contains("Replacement title")),
            "warnings: {:?}",
            out.warnings
        );
    }

    #[test]
    fn update_issue_set_body_replaces_body_and_advances_version() {
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "set-body-replace-x", "open");
        let req = UpdateIssueRequest {
            set_body: Some("# Title\n\nBRAND-NEW-BODY".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "set-body-replace-x", req).unwrap();
        assert_ne!(out.version, v0, "body replace must advance the version");
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("BRAND-NEW-BODY"), "{on_disk}");
        // The seeded body was just `# Title`; the replacement keeps the
        // heading but the whole body is swapped, and frontmatter is intact.
        assert!(
            on_disk.contains("status: open"),
            "frontmatter lost: {on_disk}"
        );
    }

    #[test]
    fn update_issue_set_body_headingless_preserves_title_and_warns() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "set-body-preserve-title-x", "open");
        let req = UpdateIssueRequest {
            set_body: Some("Replacement body".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "set-body-preserve-title-x", req).unwrap();
        assert_eq!(out.issue.title, "Title");
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("preserved existing title")),
            "warnings: {:?}",
            out.warnings
        );
    }

    #[test]
    fn update_issue_explicit_title_wins_over_set_body_without_title_warning() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "set-body-explicit-title-x", "open");
        let req = UpdateIssueRequest {
            set_body: Some("# Intermediate title\n\nReplacement body".into()),
            title: Patch::Set("Explicit title".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "set-body-explicit-title-x", req).unwrap();
        assert_eq!(out.issue.title, "Explicit title");
        assert!(
            !out.warnings.iter().any(|w| w.contains("title")),
            "explicit retitle should not warn about title intent: {:?}",
            out.warnings
        );
        assert!(out.issue.body.contains("Replacement body"));
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(
            on_disk.contains("\n# Explicit title\n\nReplacement body\n"),
            "title/body spacing or framing is non-canonical: {on_disk}"
        );
    }

    #[test]
    fn update_issue_set_body_composes_with_frontmatter_patch_atomically() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "set-body-combo-x", "open");
        let req = UpdateIssueRequest {
            set_body: Some("# Title\n\nCOMBINED-BODY".into()),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "set-body-combo-x", req).unwrap();
        let on_disk =
            fs::read_to_string(tmp.path().join("issues/set-body-combo-x/item.md")).unwrap();
        assert!(
            on_disk.contains("COMBINED-BODY") && on_disk.contains("priority: high"),
            "body and frontmatter must both land in one write: {on_disk}"
        );
    }

    #[test]
    fn update_issue_set_body_reserved_section_warns_without_blocking() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "set-body-warn-x", "open");
        let req = UpdateIssueRequest {
            set_body: Some("# Title\n\n## Notes\nlegacy heading".into()),
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "set-body-warn-x", req).unwrap();
        assert!(
            out.warnings.iter().any(|w| w.contains("## Notes")),
            "expected reserved-section warning, got {:?}",
            out.warnings
        );
        // Non-fatal: the write still lands.
        let on_disk = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("legacy heading"), "{on_disk}");
    }

    #[test]
    fn update_issue_title_rejects_multiline_input() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "title-multiline-x", "open");
        let req = UpdateIssueRequest {
            title: Patch::Set("First line\nSecond line".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "title-multiline-x", req).unwrap_err();
        assert!(
            matches!(err, MutateError::Validation(ref msg) if msg.contains("single line")),
            "got: {err:?}"
        );
    }

    #[test]
    fn update_issue_set_body_rejects_empty() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "set-body-empty-x", "open");
        let req = UpdateIssueRequest {
            set_body: Some("   \n".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "set-body-empty-x", req).unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(msg.contains("set_body cannot be empty"), "got: {msg}")
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_body_stale_version_returns_409() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "body-stale-here", "open");
        let err = update_body(
            tmp.path(),
            "body-stale-here",
            Some("sha256:deadbeef".into()),
            "x".into(),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }));
    }

    #[test]
    fn reopen_appends_reopen_notes_section() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/reopen-section");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-01\nstatus: fixed\n\
             priority: normal\nclosed: 2026-05-05\n---\n\n# T\n\n## Description\n\nx\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "reopen-section", req).unwrap();
        let on_disk = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(
            on_disk.contains("## Reopen Notes —"),
            "expected Reopen Notes section, got:\n{on_disk}"
        );
    }

    #[test]
    fn note_appends_to_comments_section() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/notable-issue-x");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n## Description\n\nx\n",
        )
        .unwrap();
        // First note creates the section.
        let _ = note_issue(
            tmp.path(),
            "notable-issue-x",
            "alice",
            "first thought",
            crate::body_sections::COMMENTS,
            None,
            false,
        )
        .unwrap();
        let after1 = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after1.contains("## Comments"));
        assert!(after1.contains("first thought"));
        // Second note appends without duplicating the section.
        let _ = note_issue(
            tmp.path(),
            "notable-issue-x",
            "bob",
            "second thought",
            crate::body_sections::COMMENTS,
            None,
            false,
        )
        .unwrap();
        let after2 = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(after2.matches("## Comments").count(), 1);
        assert!(after2.contains("first thought"));
        assert!(after2.contains("second thought"));
        let i_first = after2.find("first thought").unwrap();
        let i_second = after2.find("second thought").unwrap();
        assert!(i_first < i_second);
        // Description preserved.
        assert!(after2.contains("## Description"));
    }

    #[test]
    fn note_rejects_stale_version() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "stale-note-here", "open");
        let err = note_issue(
            tmp.path(),
            "stale-note-here",
            "alice",
            "hi",
            crate::body_sections::COMMENTS,
            Some("sha256:deadbeef".into()),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }));
    }

    #[test]
    fn write_lock_file_has_strict_permissions() {
        // The flock itself is exercised by every other test in this
        // module (each `update_issue` / `new_issue` call acquires it).
        // Cross-process exclusion would need `std::process::Command`
        // and is out of scope here; this test just asserts the
        // on-disk lock file gets `0o600` on Unix even when it
        // pre-exists with looser permissions.
        let tmp = fresh_repo();
        // Pre-create with permissive mode to verify the unconditional
        // chmod path (M1 reviewer flag C3).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tmp.path().join(".issuectl");
            fs::create_dir_all(&dir).unwrap();
            let lock = dir.join("write.lock");
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let _l1 = WriteLock::acquire(tmp.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(tmp.path().join(".issuectl/write.lock")).unwrap();
            assert_eq!(
                m.permissions().mode() & 0o777,
                0o600,
                "lock file should be 0o600 even if it pre-existed at 0o644"
            );
        }
    }

    #[test]
    fn update_writes_default_schema_on_first_use() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "bootstrap-target", "open");
        assert!(!tmp.path().join("issues/.schema.yaml").exists());
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "bootstrap-target", req).unwrap();
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file should be auto-written on first mutation"
        );
    }

    #[test]
    fn update_rejects_label_outside_schema_enum() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "label-enum-target", "open");
        // Constrain labels to a fixed set.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n  labels:\n    list: true\n    enum: [infra, frontend]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            add_labels: vec!["bogus".into()],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "label-enum-target", req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(
                msg.contains("labels") && msg.contains("bogus"),
                "expected labels/bogus in error, got {msg:?}"
            ),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn body_set_validates_against_schema() {
        // A schema tightened after the issue was created should block
        // body-set, matching the contract update_issue follows.
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "body-schema-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = update_body(
            tmp.path(),
            "body-schema-target",
            Some(v0),
            "# new body\n".into(),
            false,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(msg.contains("team"), "got {msg:?}"),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_passes_custom_fields_through() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n    enum: [payments, infra]\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "API new with custom field".into();
        req.priority = "normal".into();
        req.custom_fields.push(("team".into(), "payments".into()));
        let outcome = new_issue(tmp.path(), req).unwrap();
        let on_disk = fs::read_to_string(outcome.issue_dir.join("item.md")).unwrap();
        assert!(on_disk.contains("team: payments"), "got {on_disk}");
    }

    #[test]
    fn new_issue_request_defaults_missing_custom_fields_to_empty() {
        let req: NewIssueRequest = serde_json::from_str(r#"{"type":"bug","title":"x"}"#).unwrap();
        assert!(req.custom_fields.is_empty());
    }

    #[test]
    fn new_issue_request_accepts_empty_custom_fields_object() {
        let req: NewIssueRequest =
            serde_json::from_str(r#"{"type":"bug","title":"x","custom_fields":{}}"#).unwrap();
        assert!(req.custom_fields.is_empty());
    }

    #[test]
    fn new_issue_request_rejects_non_object_custom_fields() {
        // `custom_fields: []` (or any non-object shape) must be rejected
        // with the visitor's `expecting` text so calling agents get a
        // shape-error message rather than silent acceptance.
        let err = serde_json::from_str::<NewIssueRequest>(
            r#"{"type":"bug","title":"x","custom_fields":[]}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("expected"),
            "expected shape error, got {err}"
        );
    }

    #[test]
    fn new_issue_request_rejects_non_string_custom_field_value() {
        let err = serde_json::from_str::<NewIssueRequest>(
            r#"{"type":"bug","title":"x","custom_fields":{"team":1}}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("expected a string"),
            "expected string-type error, got {err}"
        );
    }

    #[test]
    fn new_issue_request_duplicate_key_error_precedes_bad_duplicate_value() {
        // Pinning the next_key/next_value ordering: a duplicate key
        // with a type-invalid second value must report duplicate, not
        // the type error. Otherwise the duplicate-rejection invariant
        // would be silently bypassed by anyone whose duplicate
        // happens to also be malformed.
        let err = serde_json::from_str::<NewIssueRequest>(
            r#"{"type":"bug","title":"x","custom_fields":{"team":"a","team":1}}"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-key error, got {msg:?}"
        );
    }

    #[test]
    fn new_issue_request_rejects_duplicate_custom_field_keys_at_deserialization() {
        // Calling agents that build their JSON dynamically can produce a
        // payload with two `team:` entries; rather than silent
        // last-write-wins (BTreeMap behavior), the wire deserializer
        // rejects duplicates to mirror CLI `--field foo=a --field foo=b`
        // rejection. This is the API-side enforcement of the
        // `do_new_locked` invariant.
        let payload = r#"{"type":"bug","title":"x","custom_fields":{"team":"a","team":"b"}}"#;
        let err = serde_json::from_str::<NewIssueRequest>(payload).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("team") && msg.contains("more than once"),
            "expected duplicate-key rejection, got {msg:?}"
        );
    }

    #[test]
    fn new_issue_schema_violation_returns_typed_error() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Missing team".into();
        req.priority = "normal".into();
        let err = new_issue(tmp.path(), req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(
                msg.contains("team"),
                "expected `team` in error, got {msg:?}"
            ),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn new_issue_slug_conflict_returns_typed_error() {
        // do_new_locked rejects an explicit slug whose flat directory
        // already exists. Pre-typed-error refactor this surfaced via a
        // string match on `"already" / "exists"`; now it must come
        // through DoNewError::Conflict → MutateError::ConflictingIntent.
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join("issues/taken-slug")).unwrap();
        fs::write(
            tmp.path().join("issues/taken-slug/item.md"),
            "---\nstatus: open\n---\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Conflict".into();
        req.priority = "normal".into();
        req.slug = Some("taken-slug".into());
        let err = new_issue(tmp.path(), req).unwrap_err();
        assert!(
            matches!(err, MutateError::ConflictingIntent(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn new_issue_legacy_slug_conflict_returns_typed_error() {
        // Companion to `new_issue_slug_conflict_returns_typed_error`:
        // covers the OTHER `DoNewError::Conflict` site where the slug
        // exists at a legacy `issues/open/<slug>/` path. Pre-flat-layout
        // installs hit this branch; the typed mapping must classify it
        // as ConflictingIntent (not Io / not SchemaViolation).
        let tmp = fresh_repo();
        let legacy_open = tmp.path().join("issues/open/legacy-slug");
        fs::create_dir_all(&legacy_open).unwrap();
        fs::write(legacy_open.join("item.md"), "---\nstatus: open\n---\n").unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Legacy conflict".into();
        req.priority = "normal".into();
        req.slug = Some("legacy-slug".into());
        let err = new_issue(tmp.path(), req).unwrap_err();
        assert!(
            matches!(err, MutateError::ConflictingIntent(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn new_issue_validation_returns_typed_error() {
        // Validation paths in do_new_locked (here: `--owner` on a
        // non-epic) used to be the catch-all string-match fallback, so
        // their classification was correct only by accident. Lock it.
        let tmp = fresh_repo();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Owner on non-epic".into();
        req.priority = "normal".into();
        req.owner = Some("alice".into());
        let err = new_issue(tmp.path(), req).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "got {err:?}");
    }

    #[test]
    fn new_issue_schema_config_returns_typed_error() {
        // Malformed `.schema.yaml` is the bug that motivated the typed-
        // error refactor: pre-refactor the catch-all string match
        // misclassified it as MutateError::Validation (HTTP 400). It
        // must now route through DoNewError::SchemaConfig →
        // MutateError::SchemaConfig (HTTP 500).
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields: : :\n",
        )
        .unwrap();
        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Bad schema".into();
        req.priority = "normal".into();
        let err = new_issue(tmp.path(), req).unwrap_err();
        assert!(matches!(err, MutateError::SchemaConfig(_)), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn new_issue_io_failure_returns_typed_error() {
        // Force `fs::create_dir(<root>/issues/<slug>)` to fail with
        // EACCES by chmod'ing the issues parent to read-only. That
        // path used to be funnelled into the `Validation` fallback by
        // the string matcher; the typed enum routes it correctly to
        // MutateError::Io.
        //
        // RAII guard restores permissions on every exit (including
        // panic) so `tempdir`'s `Drop` cleanup never inherits a
        // 0o500 directory.
        use std::os::unix::fs::PermissionsExt;
        struct PermGuard {
            path: PathBuf,
            original: std::fs::Permissions,
        }
        impl Drop for PermGuard {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.path, self.original.clone());
            }
        }

        let tmp = fresh_repo();
        // Use the production helper rather than a hardcoded YAML literal
        // so the test does not break if the schema format evolves.
        crate::schema::ensure_default_written(tmp.path()).unwrap();
        let issues_dir = tmp.path().join("issues");
        let original = fs::metadata(&issues_dir).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_mode(0o500);
        fs::set_permissions(&issues_dir, readonly).unwrap();
        let _guard = PermGuard {
            path: issues_dir.clone(),
            original: original.clone(),
        };

        // chmod 0o500 has no effect for uid 0; skip the assertion when
        // a probe write still succeeds (CI containers occasionally run
        // as root).
        let probe = issues_dir.join(".io-probe");
        let chmod_enforced = fs::write(&probe, b"x").is_err();
        let _ = fs::remove_file(&probe);
        if !chmod_enforced {
            return;
        }

        let mut req = NewIssueRequest::default();
        req.issue_type = "bug".into();
        req.title = "Cannot write".into();
        req.priority = "normal".into();
        req.slug = Some("io-fail-slug".into());
        let err = new_issue(tmp.path(), req).unwrap_err();

        assert!(matches!(err, MutateError::Io(_)), "got {err:?}");
    }

    #[test]
    fn close_with_as_records_closer_in_frontmatter() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-attributed", "open");

        close_issue(
            tmp.path(),
            "close-attributed",
            Some("wontfix".into()),
            Some("alice".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-attributed/item.md")).unwrap();
        assert!(after.contains("status: wontfix"), "{after}");
        assert!(after.contains("closed_by: alice"), "{after}");
        // The closer surfaces via the typed `Issue::closed_by` field —
        // not `extra`, which no longer carries it once the parser lifts
        // the key into the first-class slot.
        let parsed = crate::parser::parse_item_md_with_warnings(
            &tmp.path().join("issues/close-attributed/item.md"),
            "close-attributed",
            "open",
        );
        assert_eq!(parsed.issue.closed_by.as_deref(), Some("alice"));
        assert!(
            !parsed.issue.extra.contains_key("closed_by"),
            "closed_by must not remain in extra"
        );
    }

    #[test]
    fn close_without_as_writes_no_closed_by() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-anon", "open");

        close_issue(
            tmp.path(),
            "close-anon",
            Some("fixed".into()),
            None,
            None,
            Vec::new(),
            None,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-anon/item.md")).unwrap();
        assert!(after.contains("status: fixed"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn close_rejects_malformed_as_author() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-bad-author", "open");

        let err = close_issue(
            tmp.path(),
            "close-bad-author",
            Some("wontfix".into()),
            Some("has space".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        // Validation fires before any write — the issue stays open.
        let after = fs::read_to_string(tmp.path().join("issues/close-bad-author/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
    }

    #[test]
    fn reopen_clears_closer_attribution() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "reopen-clears-closer", "open");

        close_issue(
            tmp.path(),
            "reopen-clears-closer",
            Some("wontfix".into()),
            Some("alice".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap();

        // Reopen through the general update path; `closed_by` must drop
        // in lockstep with `closed:` so a reopened issue carries neither.
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "reopen-clears-closer", req).unwrap();

        let after =
            fs::read_to_string(tmp.path().join("issues/reopen-clears-closer/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
        assert!(!after.contains("closed:"), "{after}");
    }

    #[test]
    fn close_rejects_non_closing_status_override() {
        // `close --status open` must be refused: it is not a closing
        // status, so honoring it would leave the issue active — and with
        // `--as`, would strand a `closed_by` on an open issue.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-to-open", "open");

        let err = close_issue(
            tmp.path(),
            "close-to-open",
            Some("open".into()),
            Some("alice".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        let after = fs::read_to_string(tmp.path().join("issues/close-to-open/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn reopen_with_custom_field_closed_by_is_rejected() {
        // The reopen-clears invariant must be un-defeatable: a request
        // that reopens *and* smuggles `closed_by` through `custom_fields`
        // in the same call is rejected at validation, because `closed_by`
        // is a reserved key. Previously this ordering let the custom-field
        // loop re-add the closer the status branch had just cleared.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "reopen-smuggle", "open");
        close_issue(
            tmp.path(),
            "reopen-smuggle",
            Some("wontfix".into()),
            Some("alice".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap();

        let mut req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            ..Default::default()
        };
        req.custom_fields
            .insert("closed_by".into(), Patch::Set("mallory".into()));
        let err = update_issue(tmp.path(), "reopen-smuggle", req).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
    }

    #[test]
    fn set_closed_by_via_custom_field_is_rejected() {
        // `closed_by` is reserved, so it cannot be planted on an open
        // issue through the generic custom-field surface (`set` / `update
        // --field`). That keeps the field trustworthy: the only writer is
        // the validated lifecycle slot.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "plant-closer", "open");

        let mut req = UpdateIssueRequest::default();
        req.custom_fields
            .insert("closed_by".into(), Patch::Set("mallory".into()));
        let err = update_issue(tmp.path(), "plant-closer", req).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        let after = fs::read_to_string(tmp.path().join("issues/plant-closer/item.md")).unwrap();
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn restatus_between_closing_values_preserves_closer() {
        // fixed → wontfix must keep the recorded closer (and close date)
        // — a re-disposition is not a new close, so provenance survives.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "restatus-closer", "open");
        close_issue(
            tmp.path(),
            "restatus-closer",
            Some("fixed".into()),
            Some("alice".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Set("wontfix".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "restatus-closer", req).unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/restatus-closer/item.md")).unwrap();
        assert!(after.contains("status: wontfix"), "{after}");
        assert!(after.contains("closed_by: alice"), "{after}");
    }

    #[test]
    fn anonymous_close_scrubs_preexisting_closer() {
        // If a stray `closed_by` exists on an active issue (e.g. a manual
        // hand-edit of the frontmatter), an anonymous close must not
        // inherit it as false attribution — the active→closing edge
        // scrubs any stale value when no `--as` is given.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/anon-scrub");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\nclosed_by: ghost\n---\n\n# Title\n",
        )
        .unwrap();

        close_issue(
            tmp.path(),
            "anon-scrub",
            Some("fixed".into()),
            None,
            None,
            Vec::new(),
            None,
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("status: fixed"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
    }

    #[test]
    fn close_with_comment_appends_resolution_attributed_to_closer() {
        // `close --as alice --comment "..."` records the closing status,
        // the `closed_by:` attribution, AND a timestamped block under a
        // `## Resolution` section attributed to the closer — all in one
        // atomic write.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-with-note", "open");

        close_issue(
            tmp.path(),
            "close-with-note",
            Some("fixed".into()),
            Some("alice".into()),
            Some("Shipped in v1.2; superseded the manual workaround.".into()),
            Vec::new(),
            None,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-with-note/item.md")).unwrap();
        assert!(after.contains("status: fixed"), "{after}");
        assert!(after.contains("closed_by: alice"), "{after}");
        assert!(after.contains("## Resolution"), "{after}");
        assert!(
            after.contains("· @alice"),
            "resolution block attributed to closer: {after}"
        );
        assert!(
            after.contains("Shipped in v1.2; superseded the manual workaround."),
            "{after}"
        );
        // The block re-parses cleanly as a well-formed Resolution block.
        let parsed = crate::parser::parse_item_md_with_warnings(
            &tmp.path().join("issues/close-with-note/item.md"),
            "close-with-note",
            "closed",
        );
        let section = crate::body_sections::parse_section(
            &parsed.issue.body,
            crate::body_sections::RESOLUTION,
        );
        assert_eq!(section.blocks.len(), 1, "warnings={:?}", section.warnings);
        assert_eq!(section.blocks[0].author, "alice");
    }

    #[test]
    fn close_with_at_prefixed_closer_strips_sigil_across_both_write_sites() {
        // `close --as "@alice" --comment "..."` normalizes the single
        // leading `@` at the shared author seam, so the SAME stored token
        // `alice` lands in both write sites the closer feeds: the
        // `closed_by:` frontmatter slot and the `## Resolution` block
        // author. Guards against a refactor normalizing one but not the
        // other (issue as-flag-strip-at-sign).
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-at-note", "open");

        close_issue(
            tmp.path(),
            "close-at-note",
            Some("fixed".into()),
            Some("@alice".into()),
            Some("Stripped the sigil at the seam.".into()),
            Vec::new(),
            None,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-at-note/item.md")).unwrap();
        // Stored bare (`alice`), never the raw `@alice`, in frontmatter.
        assert!(after.contains("closed_by: alice"), "{after}");
        assert!(!after.contains("closed_by: '@alice'"), "{after}");
        assert!(!after.contains("closed_by: \"@alice\""), "{after}");
        // The resolution block heading renders the sigil exactly once
        // (`· @alice`), not doubled (`· @@alice`) from a re-prefixed store.
        assert!(after.contains("· @alice"), "{after}");
        assert!(!after.contains("· @@alice"), "{after}");
        let parsed = crate::parser::parse_item_md_with_warnings(
            &tmp.path().join("issues/close-at-note/item.md"),
            "close-at-note",
            "closed",
        );
        let section = crate::body_sections::parse_section(
            &parsed.issue.body,
            crate::body_sections::RESOLUTION,
        );
        assert_eq!(section.blocks.len(), 1, "warnings={:?}", section.warnings);
        assert_eq!(section.blocks[0].author, "alice");
        // And an interior `@` (email-shaped) is still rejected, leaving
        // no partial write.
        let tmp2 = fresh_repo();
        seed_issue(tmp2.path(), "open", "close-email", "open");
        let err = close_issue(
            tmp2.path(),
            "close-email",
            Some("fixed".into()),
            Some("alice@example.com".into()),
            None,
            Vec::new(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
    }

    #[test]
    fn close_with_comment_anonymous_uses_sentinel_author() {
        // `close --comment` without `--as` still records the rationale,
        // attributed to the `issuectl` sentinel so the managed block
        // shape stays well-formed; no `closed_by:` is written.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-anon-note", "open");

        close_issue(
            tmp.path(),
            "close-anon-note",
            Some("fixed".into()),
            None,
            Some("Root cause was a stale cache; cleared and verified.".into()),
            Vec::new(),
            None,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-anon-note/item.md")).unwrap();
        assert!(after.contains("status: fixed"), "{after}");
        assert!(!after.contains("closed_by"), "{after}");
        assert!(after.contains("## Resolution"), "{after}");
        assert!(after.contains("· @issuectl"), "{after}");
        assert!(
            after.contains("Root cause was a stale cache; cleared and verified."),
            "{after}"
        );
    }

    #[test]
    fn close_without_comment_appends_no_resolution() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-no-note", "open");

        close_issue(
            tmp.path(),
            "close-no-note",
            Some("fixed".into()),
            None,
            None,
            Vec::new(),
            None,
        )
        .unwrap();

        let after = fs::read_to_string(tmp.path().join("issues/close-no-note/item.md")).unwrap();
        assert!(!after.contains("## Resolution"), "{after}");
    }

    #[test]
    fn close_with_empty_comment_is_rejected() {
        // A whitespace-only comment is rejected by `validate_message`
        // before any write — the issue stays open.
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "close-empty-note", "open");

        let err = close_issue(
            tmp.path(),
            "close-empty-note",
            Some("done".into()),
            None,
            Some("   ".into()),
            Vec::new(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)), "{err:?}");
        let after = fs::read_to_string(tmp.path().join("issues/close-empty-note/item.md")).unwrap();
        assert!(after.contains("status: open"), "{after}");
    }

    #[test]
    fn malformed_schema_surfaces_as_schema_error() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "broken-schema-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields: : :\n",
        )
        .unwrap();
        let err = update_issue(
            tmp.path(),
            "broken-schema-target",
            UpdateIssueRequest {
                priority: Patch::Set("high".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::SchemaConfig(_)), "got {err:?}");
    }

    #[test]
    fn update_type_rejects_when_required_sections_missing() {
        // task→feature with `feature` requiring `Plan, Risks` and the
        // seeded body containing only `# Title` must surface a typed
        // `SchemaViolation` naming the missing headings (option 2).
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "type-reject-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan, Risks]\n",
        )
        .unwrap();
        let before =
            fs::read_to_string(tmp.path().join("issues/type-reject-target/item.md")).unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "type-reject-target", req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(
                    msg.contains("## Plan"),
                    "expected `## Plan` in error, got {msg}"
                );
                assert!(msg.contains("## Risks"), "expected `## Risks`, got {msg}");
                assert!(
                    msg.contains("feature"),
                    "expected new type in error, got {msg}"
                );
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
        let after =
            fs::read_to_string(tmp.path().join("issues/type-reject-target/item.md")).unwrap();
        assert_eq!(before, after, "rejected mutation must not touch disk");
    }

    #[test]
    fn update_type_succeeds_when_all_required_sections_present() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-ok-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n\
             # Title\n\n## Plan\n\nplan written\n\n## Risks\n\ntracked\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan, Risks]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-ok-target", req).unwrap();
        let content = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(content.contains("type: feature"));
        // Body untouched: counts unchanged, content preserved.
        assert_eq!(content.matches("## Plan").count(), 1, "got {content}");
        assert_eq!(content.matches("## Risks").count(), 1, "got {content}");
        assert!(content.contains("plan written"));
    }

    #[test]
    fn update_type_change_to_type_with_partial_overlap_is_rejected() {
        // feature→bug where bug requires `Repro Steps` and the body has
        // `## Plan` (from the old type) but no `## Repro Steps`. The
        // call must be rejected even though some sections are present —
        // schema requirements are evaluated against the new type only.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-overlap-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: feature\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n\
             # Title\n\n## Plan\n\nplan content\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n  bug: [\"Repro Steps\"]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("bug".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "type-overlap-target", req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => {
                assert!(msg.contains("Repro Steps"), "got {msg}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_to_looser_target_succeeds_and_leaves_old_stubs() {
        // feature→task where `task` has no required sections is
        // allowed; old stubs from `feature` are deliberately not pruned
        // (documented in AGENTS.md as "type change does not prune").
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-loose-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: feature\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n\
             # Title\n\n## Plan\n\nplan content\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("task".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-loose-target", req).unwrap();
        let content = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(content.contains("type: task"));
        // Old stubs from `feature` remain — by design.
        assert!(content.contains("## Plan"));
        assert!(content.contains("plan content"));
    }

    #[test]
    fn update_type_same_value_skips_invariant_and_section_checks() {
        // Idempotent JSON clients sending the current type must not
        // trip the new checks. Body is intentionally missing `## Plan`,
        // and there's an `assignee` (which would block a `feature→epic`
        // change). With Patch::Set("feature") on an already-feature
        // issue, none of those should fire.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-same-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: feature\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nassignee: bob\n---\n\n# Title\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-same-target", req).unwrap();
    }

    #[test]
    fn update_type_combined_with_reopen_is_rejected() {
        // Closed issue + status:open + type change in the same call
        // returns `Validation` and does not write. C4: reopen and
        // type-change must be split into separate calls.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-reopen-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: fixed\n\
             priority: normal\nclosed: 2026-05-06\n---\n\n# Title\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n\
             body_sections:\n  feature: [Plan]\n",
        )
        .unwrap();
        let before = fs::read_to_string(dir.join("item.md")).unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("open".into()),
            issue_type: Patch::Set("feature".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "type-reopen-target", req).unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(msg.contains("reopen"), "got {msg}");
                assert!(msg.contains("--type"), "got {msg}");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(before, after, "rejected combined call must not touch disk");
    }

    #[test]
    fn update_type_to_epic_migrates_lone_reporter_to_owner_with_warning() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-reporter-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\npriority: normal\nreporter: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let before = crate::parser::parse_item_md_with_warnings(
            &dir.join("item.md"),
            "type-reporter-target",
            "open",
        )
        .issue;
        let before_version = canonical_hash(&before);
        let out = update_issue(
            tmp.path(),
            "type-reporter-target",
            UpdateIssueRequest {
                issue_type: Patch::Set("epic".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let content = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(!content.contains("reporter:"), "got: {content}");
        assert!(content.contains("owner: alice"), "got: {content}");
        assert!(out.warnings.iter().any(|w| w.contains("migrated reporter")));
        // `type`, `reporter`, and `owner` are all existing canonical inputs.
        // This semantic migration must therefore advance the version rather
        // than changing the hash projection to conceal the conversion.
        assert_ne!(out.version, before_version);
        assert_eq!(out.version, canonical_hash(&out.issue));
    }

    #[test]
    fn update_type_to_epic_with_assignee_is_rejected() {
        // D1: `--type epic` on an issue with an assignee must be
        // rejected; mirrors `cmd_new`'s epic invariant.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-d1-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nassignee: alice\n---\n\n# Title\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("epic".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "type-d1-target", req).unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(
                    msg.contains("issuectl update type-d1-target --no-assignee --type epic"),
                    "got {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_to_epic_with_conflicting_owner_and_reporter_is_rejected() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-owner-conflict-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: open\npriority: normal\nreporter: alice\nowner: bob\n---\n\n# Title\n",
        )
        .unwrap();
        let err = update_issue(
            tmp.path(),
            "type-owner-conflict-target",
            UpdateIssueRequest {
                issue_type: Patch::Set("epic".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(
                    msg.contains(
                        "issuectl update type-owner-conflict-target --no-reporter --type epic"
                    ),
                    "got {msg}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_away_from_epic_with_owner_is_rejected() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/type-d1-epic-target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: epic\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\nowner: cara\n---\n\n# Title\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("task".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "type-d1-epic-target", req).unwrap_err();
        match err {
            MutateError::Validation(msg) => {
                assert!(msg.contains("owner"), "got {msg}");
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn update_type_accepts_schema_extended_type() {
        // C1: a custom schema declaring `type.enum: [bug, task, spike]`
        // must allow `--type spike` end-to-end. Pre-fix this hit the
        // hardcoded `ISSUE_TYPES` check in `validate()` and returned
        // `Validation("not one of the known types")`.
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "type-custom-target", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n    \
             enum: [bug, task, spike]\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            issue_type: Patch::Set("spike".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "type-custom-target", req).unwrap();
        let content =
            fs::read_to_string(tmp.path().join("issues/type-custom-target/item.md")).unwrap();
        assert!(content.contains("type: spike"), "got {content}");
    }

    #[test]
    fn update_type_clear_is_rejected() {
        let req = UpdateIssueRequest {
            issue_type: Patch::Clear,
            ..Default::default()
        };
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("type cannot be cleared")));
    }

    #[test]
    fn update_request_rejects_explicit_null_type() {
        // M5: JSON `"type": null` must deserialize to Patch::Clear and
        // be rejected by validate(). The CLI surface is independent —
        // this nails the API behaviour.
        let req: UpdateIssueRequest = serde_json::from_str(r#"{"type": null}"#).unwrap();
        assert!(matches!(req.issue_type, Patch::Clear));
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("type cannot be cleared")));
    }

    #[test]
    fn update_request_accepts_type_set_via_json() {
        let req: UpdateIssueRequest = serde_json::from_str(r#"{"type": "feature"}"#).unwrap();
        assert!(matches!(req.issue_type, Patch::Set(ref t) if t == "feature"));
    }

    #[test]
    fn update_rejects_when_custom_required_field_missing() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "custom-required-target", "open");
        // Add a custom required field after the issue exists. Any
        // mutation should now fail until the user adds the field.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  type:\n    required: true\n  team:\n    required: true\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "custom-required-target", req).unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(
                msg.contains("team"),
                "expected `team` in error, got {msg:?}"
            ),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    // ── Mutation-CLI verbs (set/check/label/apply via mutate.rs) ──────

    fn seed_with_body(root: &Path, slug: &str, body: &str) -> String {
        let dir = root.join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            format!(
                "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n{body}",
            ),
        )
        .unwrap();
        let parsed = crate::parser::parse_item_md_with_warnings(&dir.join("item.md"), slug, "open");
        let mut issue = parsed.issue;
        let schema = crate::schema::default_schema();
        issue.folder = folder_for_status(&schema, &issue.status).to_string();
        canonical_hash(&issue)
    }

    #[test]
    fn dry_run_does_not_write_and_returns_pending_serialized() {
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "dryrun-target-x", "open");
        let before = fs::read_to_string(tmp.path().join("issues/dryrun-target-x/item.md")).unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dryrun-target-x", req).unwrap();
        assert!(out.pending_serialized.is_some());
        let pending = out.pending_serialized.unwrap();
        assert!(pending.contains("priority: high"));
        let after = fs::read_to_string(tmp.path().join("issues/dryrun-target-x/item.md")).unwrap();
        assert_eq!(before, after, "dry-run must not touch disk");
    }

    #[test]
    fn toggle_checkbox_flips_unique_match() {
        let tmp = fresh_repo();
        let body = "# T\n\n## Tasks\n\n- [ ] write the parser\n- [ ] deploy script wiring\n- [x] tests passing\n";
        let _v0 = seed_with_body(tmp.path(), "checkbox-target-y", body);
        let out = toggle_checkbox(tmp.path(), "checkbox-target-y", "deploy", None, false).unwrap();
        assert!(out.pending_serialized.is_none());
        let after =
            fs::read_to_string(tmp.path().join("issues/checkbox-target-y/item.md")).unwrap();
        assert!(after.contains("- [x] deploy script wiring"));
        assert!(after.contains("- [ ] write the parser"));
    }

    #[test]
    fn toggle_checkbox_errors_on_zero_or_multiple_matches() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] alpha task\n- [ ] beta task\n";
        let _ = seed_with_body(tmp.path(), "checkbox-amb-z", body);
        let zero =
            toggle_checkbox(tmp.path(), "checkbox-amb-z", "missing", None, false).unwrap_err();
        assert!(matches!(zero, MutateError::Validation(s) if s.contains("no checkbox")));
        let many = toggle_checkbox(tmp.path(), "checkbox-amb-z", "task", None, false).unwrap_err();
        assert!(matches!(many, MutateError::Validation(s) if s.contains("matched")));
    }

    #[test]
    fn toggle_checkbox_dry_run_does_not_write() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "checkbox-dry-q", body);
        let before = fs::read_to_string(tmp.path().join("issues/checkbox-dry-q/item.md")).unwrap();
        let out = toggle_checkbox(tmp.path(), "checkbox-dry-q", "only one", None, true).unwrap();
        assert!(out.pending_serialized.is_some());
        assert!(out.pending_serialized.unwrap().contains("- [x] only one"));
        let after = fs::read_to_string(tmp.path().join("issues/checkbox-dry-q/item.md")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn label_add_is_idempotent_under_update_issue() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "label-idem-w", "open");
        for _ in 0..2 {
            let req = UpdateIssueRequest {
                add_labels: vec!["backend".into()],
                ..Default::default()
            };
            update_issue(tmp.path(), "label-idem-w", req).unwrap();
        }
        let after = fs::read_to_string(tmp.path().join("issues/label-idem-w/item.md")).unwrap();
        assert_eq!(after.matches("backend").count(), 1);
    }

    #[test]
    fn apply_rolls_back_on_schema_violation() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "apply-rollback-q", "open");
        let before =
            fs::read_to_string(tmp.path().join("issues/apply-rollback-q/item.md")).unwrap();
        // Schema requires `team:` — applying a patch without it must
        // be rejected and leave disk untouched.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            add_labels: vec!["backend".into()],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "apply-rollback-q", req).unwrap_err();
        assert!(matches!(err, MutateError::SchemaViolation(_)));
        let after = fs::read_to_string(tmp.path().join("issues/apply-rollback-q/item.md")).unwrap();
        assert_eq!(
            before, after,
            "schema violation must leave the file unchanged"
        );
    }

    #[test]
    fn body_ops_apply_atomically_with_frontmatter() {
        // Multi-op patch: status change + label add + checkbox toggle +
        // note append must produce a single canonical-hash bump for the
        // entire transaction (one write under one flock).
        let tmp = fresh_repo();
        let body = "# T\n\n## Tasks\n\n- [ ] tests passing\n\n## Description\n\nbody.\n";
        let v0 = seed_with_body(tmp.path(), "body-ops-mix", body);
        let req = UpdateIssueRequest {
            expected_version: Some(v0),
            status: Patch::Set("testing".into()),
            add_labels: vec!["agent-friendly".into()],
            body_ops: vec![
                BodyOp::SetCheckbox(SetCheckboxOp {
                    match_substring: "tests passing".into(),
                    checked: true,
                }),
                BodyOp::AppendNote(AppendNoteOp {
                    author: "ci-bot".into(),
                    message: "all checks green".into(),
                    section: NoteSection::AgentRuns,
                }),
            ],
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "body-ops-mix", req).unwrap();
        assert!(out.version.starts_with("sha256:"));
        let after = fs::read_to_string(out.issue_dir.join("item.md")).unwrap();
        assert!(after.contains("status: testing"));
        assert!(after.contains("agent-friendly"));
        assert!(after.contains("- [x] tests passing"));
        assert!(after.contains("## Agent Runs"));
        assert!(after.contains("@ci-bot"));
        assert!(after.contains("all checks green"));
    }

    #[test]
    fn body_ops_dry_run_emits_diff_without_writing() {
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "body-ops-dry", body);
        let before = fs::read_to_string(tmp.path().join("issues/body-ops-dry/item.md")).unwrap();
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "only one".into(),
                checked: true,
            })],
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "body-ops-dry", req).unwrap();
        let pending = out.pending_serialized.expect("dry-run carries pending");
        assert!(pending.contains("- [x] only one"));
        assert!(out.before_serialized.is_some());
        let after = fs::read_to_string(tmp.path().join("issues/body-ops-dry/item.md")).unwrap();
        assert_eq!(before, after, "dry-run must not touch disk");
    }

    #[test]
    fn body_ops_rollback_on_failed_op() {
        // A failing checkbox match must surface as Validation and leave
        // disk untouched — even when the patch also changes the
        // frontmatter (status, labels). The whole transaction rolls
        // back; nothing partial leaks.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] alpha\n- [ ] beta\n";
        let _ = seed_with_body(tmp.path(), "body-ops-rollback", body);
        let before =
            fs::read_to_string(tmp.path().join("issues/body-ops-rollback/item.md")).unwrap();
        let req = UpdateIssueRequest {
            status: Patch::Set("testing".into()),
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "nope".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "body-ops-rollback", req).unwrap_err();
        assert!(
            matches!(&err, MutateError::Validation(s) if s.contains("body_ops[0]") && s.contains("no checkbox")),
            "got {err:?}"
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/body-ops-rollback/item.md")).unwrap();
        assert_eq!(
            before, after,
            "failed body op must roll back frontmatter changes too"
        );
    }

    #[test]
    fn body_ops_deserialize_external_tag_yaml_shape() {
        // The patch.yaml shape is externally tagged: each list entry is
        // a single-key mapping. Pin the wire format so a future serde
        // refactor can't silently change the agent contract.
        let yaml = r#"
body_ops:
  - set_checkbox:
      match: "tests passing"
      checked: true
  - append_note:
      section: agent_runs
      author: ci-bot
      message: "all green"
"#;
        let req: UpdateIssueRequest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(req.body_ops.len(), 2);
        match &req.body_ops[0] {
            BodyOp::SetCheckbox(s) => {
                assert_eq!(s.match_substring, "tests passing");
                assert!(s.checked);
            }
            other => panic!("expected SetCheckbox, got {other:?}"),
        }
        match &req.body_ops[1] {
            BodyOp::AppendNote(n) => {
                assert_eq!(n.author, "ci-bot");
                assert_eq!(n.section, NoteSection::AgentRuns);
            }
            other => panic!("expected AppendNote, got {other:?}"),
        }
    }

    #[test]
    fn body_ops_deserialize_external_tag_json_shape() {
        // The JSON `apply` input must accept the same external-tag
        // shape. Pin both arms so the JSON contract round-trips with
        // the YAML one above.
        let json = r#"{
            "body_ops": [
                {"set_checkbox": {"match": "ship it", "checked": false}},
                {"append_note": {"author": "alice", "message": "done"}}
            ]
        }"#;
        let req: UpdateIssueRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.body_ops.len(), 2);
        match &req.body_ops[0] {
            BodyOp::SetCheckbox(s) => {
                assert_eq!(s.match_substring, "ship it");
                assert!(!s.checked);
            }
            other => panic!("expected SetCheckbox, got {other:?}"),
        }
        match &req.body_ops[1] {
            BodyOp::AppendNote(n) => {
                assert_eq!(n.author, "alice");
                assert_eq!(n.section, NoteSection::Comments);
            }
            other => panic!("expected AppendNote, got {other:?}"),
        }
    }

    #[test]
    fn body_ops_deserialize_rejects_unknown_top_key() {
        // Unknown variant key — visitor must reject with the canonical
        // unknown-field shape.
        let json = r#"{"body_ops": [{"toggl_checkbox": "x"}]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("toggl_checkbox") || msg.contains("unknown"),
            "got {msg}"
        );
    }

    #[test]
    fn body_ops_deserialize_rejects_extra_sibling_key() {
        // Unknown sibling key beside a valid op.
        let json = r#"{"body_ops": [
            {"set_checkbox": {"match": "x", "checked": true}, "junk": 1}
        ]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(err.to_string().contains("single-key mapping"), "got {err}");
    }

    #[test]
    fn body_ops_deserialize_rejects_null_sibling_key_bypass() {
        // Previous Option<T> helper struct accepted this — the null
        // collapsed to None and "exactly one variant" passed.
        let json = r#"{"body_ops": [
            {"set_checkbox": {"match": "x", "checked": true}, "append_note": null}
        ]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(err.to_string().contains("single-key mapping"), "got {err}");
    }

    #[test]
    fn append_note_op_rejects_unknown_field() {
        // Pin the pre-existing `deny_unknown_fields` on AppendNoteOp so a
        // future refactor doesn't silently drop the directive.
        let json = r#"{"body_ops": [
            {"append_note": {"author": "a", "message": "m", "junk": 1}}
        ]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(
            err.to_string().contains("junk") || err.to_string().contains("unknown"),
            "got {err}"
        );
    }

    #[test]
    fn body_ops_length_cap_rejected_by_validate() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "body-ops-too-many", "open");
        let huge: Vec<BodyOp> = (0..(MAX_BODY_OPS + 1))
            .map(|_| {
                BodyOp::SetCheckbox(SetCheckboxOp {
                    match_substring: "x".into(),
                    checked: true,
                })
            })
            .collect();
        let req = UpdateIssueRequest {
            body_ops: huge,
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "body-ops-too-many", req).unwrap_err();
        assert!(
            matches!(&err, MutateError::Validation(s) if s.contains("body_ops length") && s.contains("exceeds")),
            "got {err:?}"
        );
    }

    #[test]
    fn failed_body_op_does_not_create_default_schema() {
        // Regression: until the locate-then-validate-then-side-effects
        // refactor, `ensure_default_written` ran before body ops, so a
        // failing op left `.schema.yaml` newly created on a fresh repo.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] alpha\n";
        let _ = seed_with_body(tmp.path(), "body-ops-no-schema-bootstrap", body);
        let schema_path = tmp.path().join("issues/.schema.yaml");
        assert!(!schema_path.exists(), "precondition: no schema yet");
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "no-such-needle".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "body-ops-no-schema-bootstrap", req).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));
        assert!(
            !schema_path.exists(),
            "failed body op must not bootstrap .schema.yaml"
        );
    }

    #[test]
    fn failed_body_op_does_not_migrate_legacy_layout() {
        // Same regression class for the legacy → flat directory move.
        let tmp = fresh_repo();
        let legacy_dir = tmp.path().join("issues/open/body-ops-no-migrate");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n\n- [ ] only\n",
        )
        .unwrap();
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "no-such-needle".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "body-ops-no-migrate", req).unwrap_err();
        assert!(
            legacy_dir.join("item.md").exists(),
            "failed body op must leave legacy directory in place"
        );
        assert!(
            !tmp.path()
                .join("issues/body-ops-no-migrate/item.md")
                .exists(),
            "failed body op must NOT migrate to flat layout"
        );
    }

    #[test]
    fn standalone_note_does_not_create_default_schema_on_validation_failure() {
        // Side-effects deferral on body-only verbs: a `note_issue`
        // call that fails validation must not bootstrap `.schema.yaml`
        // (parity with the `update_issue`/`apply` path).
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "note-defer", "open");
        // Tighten the schema so the post-mutation frontmatter is
        // rejected (required field that doesn't exist on the issue).
        // The schema file *exists* before the call, so the failure
        // path we exercise is "schema validation rejects the write,"
        // not "schema file missing."
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        // Now stage a legacy directory so the migration side-effect
        // would also leak if not deferred.
        let legacy = tmp.path().join("issues/open/note-legacy-defer");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let err = note_issue(
            tmp.path(),
            "note-legacy-defer",
            "alice",
            "hi",
            crate::body_sections::COMMENTS,
            None,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::SchemaViolation(_)));
        assert!(
            legacy.join("item.md").exists(),
            "legacy dir must remain on schema-violation rollback"
        );
        assert!(
            !tmp.path().join("issues/note-legacy-defer/item.md").exists(),
            "no migration must have happened"
        );
    }

    #[test]
    fn standalone_toggle_checkbox_does_not_migrate_legacy_on_match_failure() {
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/cbx-legacy-defer");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n\n- [ ] only\n",
        )
        .unwrap();
        let err = toggle_checkbox(
            tmp.path(),
            "cbx-legacy-defer",
            "no-such-substring",
            None,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));
        assert!(
            legacy.join("item.md").exists(),
            "legacy dir must remain on no-match rollback"
        );
        assert!(
            !tmp.path().join("issues/cbx-legacy-defer/item.md").exists(),
            "no migration must have happened"
        );
    }

    #[test]
    fn idempotent_set_checkbox_keeps_canonical_version_stable() {
        // Pin the central retry-safety contract: replaying an
        // already-target set_checkbox produces the same canonical
        // version (false-409s would defeat optimistic concurrency).
        // `updated:` IS bumped on disk and an SSE event fires, but
        // both are excluded from / orthogonal to the canonical hash.
        let tmp = fresh_repo();
        let body = "# T\n\n- [x] already on\n";
        let _ = seed_with_body(tmp.path(), "set-cbx-version-stable", body);
        let v0 = {
            let req = UpdateIssueRequest::default();
            update_issue(tmp.path(), "set-cbx-version-stable", req)
                .unwrap()
                .version
        };
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "already on".into(),
                checked: true,
            })],
            ..Default::default()
        };
        let v1 = update_issue(tmp.path(), "set-cbx-version-stable", req)
            .unwrap()
            .version;
        assert_eq!(
            v0, v1,
            "no-op set_checkbox must not bump the canonical version (retry-safety contract)"
        );
    }

    #[test]
    fn idempotent_set_checkbox_uncheck_already_unchecked() {
        // Mirror of `set_checkbox_is_idempotent_on_target_state` for
        // the `checked: false` arm — pin both directions.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] already off\n";
        let _ = seed_with_body(tmp.path(), "set-cbx-uncheck-idem", body);
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "already off".into(),
                checked: false,
            })],
            ..Default::default()
        };
        update_issue(tmp.path(), "set-cbx-uncheck-idem", req).unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/set-cbx-uncheck-idem/item.md")).unwrap();
        assert!(after.contains("- [ ] already off"));
    }

    #[test]
    fn body_ops_visitor_rejects_empty_map() {
        // `{}` body-op entry must error rather than be accepted as a
        // mystery default — pin the visitor branch.
        let json = r#"{"body_ops": [{}]}"#;
        let err = serde_json::from_str::<UpdateIssueRequest>(json).unwrap_err();
        assert!(
            err.to_string().contains("declare exactly one operation"),
            "got {err}"
        );
    }

    #[test]
    fn transition_warnings_surface_malformed_config_instead_of_silence() {
        // Regression: previously `transition_warnings` swallowed
        // `transitions.yaml` load failures, so a body verb on a repo
        // with a broken rules engine got NO warning while the unified
        // PATCH path 5xx'd. Now the body verbs surface the load
        // failure as a warning string so agents and operators see the
        // outage either way.
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "broken-rules-target", "open");
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(".issuectl/transitions.yaml"),
            "this is not: [valid yaml: at all\n",
        )
        .unwrap();
        let out = note_issue(
            tmp.path(),
            "broken-rules-target",
            "alice",
            "hi",
            crate::body_sections::COMMENTS,
            None,
            false,
        )
        .unwrap();
        assert!(
            out.warnings.iter().any(|w| w.contains("rules engine")),
            "expected a 'rules engine: ...' warning, got {:?}",
            out.warnings
        );
    }

    #[test]
    fn standalone_toggle_checkbox_surfaces_transition_warning_without_failing() {
        // #11: standalone body verbs (`toggle_checkbox`, `note_issue`)
        // detect transition-rule violations but emit them as warnings
        // rather than refusing the write — the user wanted the change
        // through; the rule mismatch goes to the caller for them to
        // resolve. The unified `body_ops` PATCH path keeps the strict
        // rejection.
        let tmp = fresh_repo();
        // Set up a rule: `done` requires assignee. Seed a `done` issue
        // without an assignee so the rule is already violated; the
        // checkbox toggle won't change frontmatter so it just inherits.
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(".issuectl/transitions.yaml"),
            "version: 1\nstatus_rules:\n  done:\n    requires_assignee: true\n",
        )
        .unwrap();
        let dir = tmp.path().join("issues/warn-cbx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: task\ncreated: 2026-05-06\nstatus: done\npriority: normal\n---\n\n# T\n\n- [ ] flip me\n",
        )
        .unwrap();
        let out = toggle_checkbox(tmp.path(), "warn-cbx", "flip me", None, false).unwrap();
        assert!(
            !out.warnings.is_empty(),
            "expected at least one warning for the rule violation"
        );
        assert!(
            out.warnings.iter().any(|w| w.contains("assignee")),
            "warnings should mention the missing assignee, got {:?}",
            out.warnings
        );
        // Write went through despite the violation.
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("- [x] flip me"));
    }

    #[test]
    fn set_checkbox_is_idempotent_on_target_state() {
        // A retry of the same set_checkbox op (already at target state)
        // must NOT toggle the box back. This is the central reason
        // body_ops uses set_checkbox rather than the toggle primitive.
        let tmp = fresh_repo();
        let body = "# T\n\n- [x] already checked\n";
        let _ = seed_with_body(tmp.path(), "set-cbx-idem", body);
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::SetCheckbox(SetCheckboxOp {
                match_substring: "already checked".into(),
                checked: true,
            })],
            ..Default::default()
        };
        update_issue(tmp.path(), "set-cbx-idem", req).unwrap();
        let after = fs::read_to_string(tmp.path().join("issues/set-cbx-idem/item.md")).unwrap();
        assert!(
            after.contains("- [x] already checked"),
            "idempotent set must leave box checked, got:\n{after}"
        );
    }

    #[test]
    fn body_ops_validate_rejects_bad_author() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "body-ops-bad-author", "open");
        let req = UpdateIssueRequest {
            body_ops: vec![BodyOp::AppendNote(AppendNoteOp {
                author: "alice\n## Pwned".into(),
                message: "hi".into(),
                section: NoteSection::Comments,
            })],
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "body-ops-bad-author", req).unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("body_ops[0].author")));
    }

    #[test]
    fn dry_run_does_not_create_default_schema() {
        // Regression for review finding #1: `--dry-run` must not
        // bootstrap `.schema.yaml` (the previous version called
        // `ensure_default_written` before the dry-run branch).
        let tmp = fresh_repo();
        let _v0 = seed_issue(tmp.path(), "open", "dryrun-no-schema-x", "open");
        let schema_path = tmp.path().join("issues/.schema.yaml");
        assert!(!schema_path.exists(), "precondition: no schema yet");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "dryrun-no-schema-x", req).unwrap();
        assert!(
            !schema_path.exists(),
            "dry-run must not create issues/.schema.yaml"
        );
    }

    #[test]
    fn dry_run_does_not_migrate_legacy_layout() {
        // Regression for review finding #1: `--dry-run` must not
        // perform the legacy → flat directory rename that
        // `locate_and_migrate` does on real writes.
        let tmp = fresh_repo();
        let legacy_dir = tmp.path().join("issues/open/dryrun-no-migrate-y");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# T\n",
        )
        .unwrap();
        let flat_dir = tmp.path().join("issues/dryrun-no-migrate-y");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let _ = update_issue(tmp.path(), "dryrun-no-migrate-y", req).unwrap();
        assert!(legacy_dir.exists(), "dry-run must not move the legacy dir");
        assert!(
            !flat_dir.exists(),
            "dry-run must not create the flat-layout dir"
        );
    }

    #[test]
    fn dry_run_returns_before_serialized_for_diff() {
        // Regression for review finding #5: `before_serialized` must
        // be filled under the flock so the CLI epilogue can render
        // a diff without re-reading disk outside the lock.
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "dryrun-before-w", "open");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "dryrun-before-w", req).unwrap();
        let before = out.before_serialized.expect("dry-run must capture before");
        let after = out.pending_serialized.expect("dry-run must capture after");
        assert!(before.contains("priority: normal"));
        assert!(after.contains("priority: high"));
        assert_ne!(before, after);
    }

    #[test]
    fn checkbox_state_does_not_panic_on_unicode_marker() {
        // Regression for review finding #4: `checkbox_state` used to
        // panic on `&rest[2..3]` when the bracket content was
        // multibyte (e.g. `[✓]`).
        assert_eq!(checkbox_state("- [✓] task"), None);
        assert_eq!(checkbox_state("- [é] task"), None);
        assert_eq!(checkbox_state("- [ ] task"), Some(false));
        assert_eq!(checkbox_state("- [x] task"), Some(true));
        assert_eq!(checkbox_state("- [X] task"), Some(true));
        // Don't panic with non-ASCII content after the box either.
        assert_eq!(checkbox_state("- [ ] café"), Some(false));
    }

    #[test]
    fn toggle_checkbox_ignores_lines_inside_fenced_code() {
        // Regression for review finding #3: fenced code blocks must
        // NOT be considered when matching checkbox lines.
        let tmp = fresh_repo();
        let body = "# T\n\nIn docs:\n\n```markdown\n- [ ] only example here\n```\n\n\
                    Real:\n\n- [ ] real task\n";
        let _ = seed_with_body(tmp.path(), "fence-target-z", body);
        // The "example" substring is only inside the code fence —
        // the fence-aware scanner should report no match.
        let err =
            toggle_checkbox(tmp.path(), "fence-target-z", "example", None, false).unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("no checkbox")));
        // The real task should still toggle cleanly.
        toggle_checkbox(tmp.path(), "fence-target-z", "real", None, false).unwrap();
        let after = fs::read_to_string(tmp.path().join("issues/fence-target-z/item.md")).unwrap();
        assert!(after.contains("- [x] real task"));
        assert!(after.contains("- [ ] only example here"));
    }

    #[test]
    fn note_validates_against_schema() {
        // Regression for review finding #6: `note_issue` previously
        // skipped schema validation, which `update_body` enforced —
        // making `body set` reject and `note` write through.
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "note-schema-target-q", "open");
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = note_issue(
            tmp.path(),
            "note-schema-target-q",
            "alice",
            "hello",
            crate::body_sections::COMMENTS,
            None,
            false,
        )
        .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(msg.contains("team"), "got {msg:?}"),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn check_validates_against_schema() {
        // Regression for review finding #6.
        let tmp = fresh_repo();
        let body = "# T\n\n- [ ] only one\n";
        let _ = seed_with_body(tmp.path(), "check-schema-target-r", body);
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let err = toggle_checkbox(tmp.path(), "check-schema-target-r", "only one", None, false)
            .unwrap_err();
        match err {
            MutateError::SchemaViolation(msg) => assert!(msg.contains("team"), "got {msg:?}"),
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
    }

    #[test]
    fn epic_slug_shape_validated_in_request() {
        // Regression for review finding #7: `Patch::Set` for epic
        // must reject non-slug-shaped values in `validate()` so the
        // YAML / `set` paths can't bypass the CLI flag's slug check.
        let req = UpdateIssueRequest {
            epic: Patch::Set("Not a slug".into()),
            ..Default::default()
        };
        let err = req.validate().unwrap_err();
        assert!(matches!(err, MutateError::Validation(s) if s.contains("epic")));
    }

    #[test]
    fn apply_yaml_rejects_duplicate_keys() {
        // Regression for review finding #12: `serde_yaml 0.9` rejects
        // duplicate map keys at every depth. The reviewers feared a
        // last-wins silent collapse for `priority: high\npriority: low`;
        // verify the parser rejects it instead.
        let yaml = "slug: a-b\npriority: high\npriority: low\n";
        let err = serde_yaml::from_str::<serde_yaml::Value>(yaml).unwrap_err();
        assert!(
            err.to_string().contains("duplicate"),
            "expected duplicate-key error, got {err}"
        );
        let nested = "slug: a-b\ncustom_fields:\n  k: v\n  k: v2\n";
        let err = serde_yaml::from_str::<serde_yaml::Value>(nested).unwrap_err();
        assert!(
            err.to_string().contains("duplicate"),
            "expected nested duplicate-key error, got {err}"
        );
    }

    #[test]
    fn apply_multi_field_patch_lands_atomically() {
        // Positive test for the apply transaction: priority + add_label
        // + custom_field all advance the canonical hash exactly once.
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "apply-happy-path", "open");
        let req = UpdateIssueRequest {
            expected_version: Some(v0.clone()),
            priority: Patch::Set("high".into()),
            add_labels: vec!["backend".into()],
            custom_fields: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("triage".into(), Patch::Set("P1".into()));
                m
            },
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "apply-happy-path", req).unwrap();
        assert_ne!(out.version, v0);
        let after = fs::read_to_string(tmp.path().join("issues/apply-happy-path/item.md")).unwrap();
        assert!(after.contains("priority: high"));
        assert!(after.contains("backend"));
        assert!(after.contains("triage: P1"));
    }

    // ── Round-2 review regressions ───────────────────────────────

    #[test]
    fn status_clear_validation_rejects_before_any_disk_writes() {
        // Round-2 #1: dropping the CLI-side `status --clear` check
        // exposed a hole — `Patch::Clear` for status passed
        // `validate()` and only got rejected deeper inside
        // `update_issue_under_lock`, *after* `ensure_default_written`
        // and `locate_and_migrate` had already written `.schema.yaml`
        // and migrated legacy directories.
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/status-clear-legacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# T\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            status: Patch::Clear,
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "status-clear-legacy", req).unwrap_err();
        assert!(matches!(err, MutateError::Validation(_)));

        assert!(
            legacy.exists(),
            "validation failure must not migrate the legacy directory"
        );
        assert!(
            !tmp.path().join("issues/status-clear-legacy").exists(),
            "validation failure must not create the flat-layout directory"
        );
        assert!(
            !tmp.path().join("issues/.schema.yaml").exists(),
            "validation failure must not bootstrap the default schema"
        );
    }

    #[test]
    fn dry_run_before_serialized_captures_raw_disk_bytes() {
        // Round-2 #2: `before_serialized` used to be the canonicalised
        // re-serialization of the parsed item, which silently hid
        // formatting changes that the real write would also apply
        // (dropped YAML comments, scalar-style shifts, etc.). Pin
        // that the field now contains the raw on-disk bytes so the
        // dry-run diff is a faithful preview.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/raw-bytes-target");
        fs::create_dir_all(&dir).unwrap();
        let raw = "---\ntype: bug\n# survives only on disk; serde_yaml drops it on round-trip\n\
                   created: 2026-05-06\nstatus: open\npriority: normal\n---\n\n# T\n";
        fs::write(dir.join("item.md"), raw).unwrap();

        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "raw-bytes-target", req).unwrap();
        let before = out.before_serialized.expect("dry-run captures before");
        assert_eq!(
            before, raw,
            "before_serialized must be the raw on-disk bytes, not a canonicalised re-serialization"
        );
        // And the after must NOT contain the comment, demonstrating
        // that a real write would drop it — the dry-run diff visibly
        // reflects that loss because we don't pre-canonicalise the
        // before half.
        let after = out.pending_serialized.expect("dry-run captures after");
        assert!(!after.contains("survives only on disk"));
    }

    #[test]
    fn dry_run_final_dir_predicts_flat_path_for_legacy_issue() {
        // Round-2 #3: dry-run on a legacy-layout issue used to return
        // `issue_dir = issues/open/<slug>` (where the file currently
        // lives) but a real write would migrate to `issues/<slug>`.
        // The JSON envelope's `final_dir` must agree with the real
        // write's destination.
        let tmp = fresh_repo();
        let legacy = tmp.path().join("issues/open/legacy-finaldir");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("item.md"),
            "---\ntype: bug\ncreated: 2026-05-06\nstatus: open\n\
             priority: normal\n---\n\n# T\n",
        )
        .unwrap();

        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            dry_run: true,
            ..Default::default()
        };
        let out = update_issue(tmp.path(), "legacy-finaldir", req).unwrap();
        assert_eq!(
            out.issue_dir,
            tmp.path().join("issues/legacy-finaldir"),
            "dry-run must report the flat-layout path even when the file currently lives at a legacy path"
        );
        // And legacy must remain untouched — no migration.
        assert!(legacy.exists(), "dry-run must not migrate legacy layout");
    }

    #[test]
    fn note_decisions_section_appends_to_decisions() {
        let tmp = fresh_repo();
        let _ = seed_issue(tmp.path(), "open", "note-decision-p", "open");
        note_issue(
            tmp.path(),
            "note-decision-p",
            "alice",
            "go with option B",
            crate::body_sections::DECISIONS,
            None,
            false,
        )
        .unwrap();
        let after = fs::read_to_string(tmp.path().join("issues/note-decision-p/item.md")).unwrap();
        assert!(after.contains("## Decisions"));
        assert!(after.contains("go with option B"));
        assert!(!after.contains("## Comments"));
    }

    // ── write-under-flock coverage ──────────────────────────────────────
    //
    // `remove-web-ui` deleted the six `*_publishes_before_releasing_flock`
    // server tests plus their `install_lock_probe` helpers. Those were the
    // only coverage for the repo-wide serialization invariant: every
    // mutation writes to disk while holding `.issuectl/write.lock`
    // (`WriteLock::acquire` at the top of each entry point in this module).
    // The publish seam left with the web UI; these tests restore the
    // underlying flock guarantees directly at the library level, without an
    // EventHub probe. They are hermetic (tempdir only) and deterministic:
    // each is written so that removing `WriteLock` makes it fail, not hang.

    /// Returns `true` iff the repo-wide write lock is currently unheld.
    ///
    /// Probes with a non-blocking `try_lock_exclusive` on a *fresh* fd so a
    /// leaked lock reports `false` instead of hanging the test — but wraps
    /// it in a short bounded retry loop. On macOS/BSD, `flock(LOCK_EX |
    /// LOCK_NB)` can return a *transient* `EWOULDBLOCK` even when the lock
    /// is actually free whenever many `flock` calls are in flight on the
    /// same filesystem (the full parallel suite hammers it, and rapidly
    /// recycled tempdir inodes widen the window). A single one-shot probe
    /// turns that transient into a spurious "lock is held" — the flake this
    /// helper used to have; instrumentation showed the very next probe on a
    /// fresh fd already succeeded. Retrying `WouldBlock` on a fresh fd
    /// until a short deadline outlasts the transient without any of the
    /// hazards of a blocking acquire (no thread to leak, no `create_dir_all`
    /// side effect, no weakening of "held right now" to "free eventually").
    ///
    /// A *genuinely* leaked lock stays `WouldBlock` for the whole window
    /// and correctly returns `false` at the deadline. Any other error (a
    /// missing lock file, a permissions problem) is a real bug and panics
    /// rather than being silently reclassified as "lock still held".
    fn write_lock_is_free(root: &Path) -> bool {
        use std::io::ErrorKind;
        use std::time::{Duration, Instant};

        let path = root.join(".issuectl/write.lock");
        // The transient clears in microseconds; the deadline only ever
        // bites on a genuine leak, so keep it short enough for fast
        // regression feedback while leaving generous margin over the
        // observed transient window.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // A fresh open file description per attempt — this is what the
            // instrumentation showed clears the transient `EWOULDBLOCK`.
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .expect("lock file should exist after a mutation");
            match f.try_lock_exclusive() {
                Ok(()) => {
                    let _ = FileExt::unlock(&f);
                    return true;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("failed to probe write lock {}: {e}", path.display()),
            }
        }
    }

    #[test]
    fn held_write_lock_serializes_a_second_writer() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "race-target-here", "open");
        let root = tmp.path().to_path_buf();

        // Main thread holds the flock. A concurrent mutation must not be
        // able to complete its write until we release it.
        let lock = WriteLock::acquire(tmp.path()).unwrap();

        let started = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let handle = {
            let started = Arc::clone(&started);
            let done = Arc::clone(&done);
            let root = root.clone();
            thread::spawn(move || {
                started.store(true, Ordering::SeqCst);
                let req = UpdateIssueRequest {
                    priority: Patch::Set("high".into()),
                    ..Default::default()
                };
                update_issue(&root, "race-target-here", req).unwrap();
                done.store(true, Ordering::SeqCst);
            })
        };

        // Wait until the writer thread is actually running, then give it
        // ample time to reach (and block on) `WriteLock::acquire`. If the
        // lock were removed the mutation would finish inside this window.
        while !started.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(250));
        assert!(
            !done.load(Ordering::SeqCst),
            "a mutation completed while the write lock was held — writes are not serialized under the flock"
        );

        // Release; the blocked writer must now proceed to completion.
        drop(lock);
        handle.join().unwrap();
        assert!(
            done.load(Ordering::SeqCst),
            "writer did not complete after the flock was released"
        );
        let after = fs::read_to_string(tmp.path().join("issues/race-target-here/item.md")).unwrap();
        assert!(after.contains("priority: high"), "{after}");
    }

    #[test]
    fn write_lock_released_after_successful_mutation() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "release-ok-here", "open");
        let req = UpdateIssueRequest {
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        update_issue(tmp.path(), "release-ok-here", req).unwrap();
        assert!(
            write_lock_is_free(tmp.path()),
            "flock must be released once a successful mutation returns"
        );
    }

    #[test]
    fn write_lock_released_after_failed_mutation() {
        let tmp = fresh_repo();
        seed_issue(tmp.path(), "open", "release-err-here", "open");
        // A stale `expected_version` fails the optimistic-concurrency
        // check — an error raised *while the flock is held* (the version
        // compare runs after `WriteLock::acquire`). The guard must still
        // drop on the error path.
        let req = UpdateIssueRequest {
            expected_version: Some("sha256:staleversionvalue".into()),
            priority: Patch::Set("high".into()),
            ..Default::default()
        };
        let err = update_issue(tmp.path(), "release-err-here", req).unwrap_err();
        assert!(matches!(err, MutateError::VersionMismatch { .. }), "{err}");
        assert!(
            write_lock_is_free(tmp.path()),
            "flock must be released even when a mutation errors under the lock"
        );
    }

    #[test]
    fn write_lock_serializes_racing_read_modify_write() {
        use std::thread;
        use std::time::Duration;

        // Strong mutual-exclusion proof. Each iteration does a
        // read-modify-write of a shared counter *under the flock* with a
        // deliberate pause between read and write. With serialization the
        // final count is exact; without it the pause guarantees lost
        // updates. This is the test that fails if `WriteLock` is removed.
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        let counter = tmp.path().join(".issuectl/race-counter");
        fs::write(&counter, "0").unwrap();
        let root = tmp.path().to_path_buf();

        const THREADS: u64 = 4;
        const ITERS: u64 = 25;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let root = root.clone();
                let counter = counter.clone();
                thread::spawn(move || {
                    for _ in 0..ITERS {
                        let _lock = WriteLock::acquire(&root).unwrap();
                        let cur: u64 = fs::read_to_string(&counter)
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        // Widen the interleaving window; harmless when the
                        // lock is doing its job, fatal to correctness when
                        // it isn't.
                        thread::yield_now();
                        thread::sleep(Duration::from_micros(50));
                        fs::write(&counter, (cur + 1).to_string()).unwrap();
                        // `_lock` drops here → flock released.
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let final_count: u64 = fs::read_to_string(&counter)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            final_count,
            THREADS * ITERS,
            "lost updates: WriteLock did not serialize concurrent read-modify-write"
        );
    }

    #[test]
    fn optimistic_version_under_contention_yields_exactly_one_winner() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        // Two writers both capture the same base version and then race,
        // each asserting `expected_version == v0`. Serialization forces
        // one to run first (bumping the version) so the second's
        // optimistic check fails: exactly one Ok, exactly one
        // VersionMismatch.
        let tmp = fresh_repo();
        let v0 = seed_issue(tmp.path(), "open", "opt-contend-here", "open");
        let root = tmp.path().to_path_buf();
        let barrier = Arc::new(Barrier::new(2));

        // Classify inside the thread and return a small tag rather than
        // the large `Result<UpdateOutcome, MutateError>` (which trips
        // clippy::result_large_err when returned across the thread
        // boundary): "ok" won the compare, "mismatch" lost it, anything
        // else is an unexpected error we surface in the assertion.
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let root = root.clone();
                let v0 = v0.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let req = UpdateIssueRequest {
                        expected_version: Some(v0),
                        priority: Patch::Set("high".into()),
                        ..Default::default()
                    };
                    barrier.wait();
                    match update_issue(&root, "opt-contend-here", req) {
                        Ok(_) => "ok",
                        Err(MutateError::VersionMismatch { .. }) => "mismatch",
                        Err(_) => "error",
                    }
                })
            })
            .collect();

        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let winners = outcomes.iter().filter(|o| **o == "ok").count();
        let mismatches = outcomes.iter().filter(|o| **o == "mismatch").count();
        assert_eq!(
            winners, 1,
            "exactly one racing writer should win the version compare (outcomes: {outcomes:?})"
        );
        assert_eq!(
            mismatches, 1,
            "the losing writer must be rejected with VersionMismatch (outcomes: {outcomes:?})"
        );
    }
}
