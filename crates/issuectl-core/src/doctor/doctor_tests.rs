use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_repo() -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("issues/open")).unwrap();
        fs::create_dir_all(tmp.path().join("issues/closed")).unwrap();
        tmp
    }

    fn put_legacy(tmp: &TempDir, folder: &str, n: u32, slug: &str, body: &str) {
        let dir = tmp
            .path()
            .join("issues")
            .join(folder)
            .join(format!("{n}-{slug}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
    }

    #[test]
    fn detect_gitignored_canonical_paths_flags_ignored_agents_md() {
        // Regression for #simply-workable-umbrella: a tempdir repo
        // with `.gitignore` masking `.issuectl/` should surface
        // `.issuectl/AGENTS.md` in gitignored_paths after `agents init`.
        let tmp = fresh_repo();
        // Bootstrap a real git repo so `git check-ignore` works.
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .arg("init")
            .arg("--quiet")
            .output()
            .expect("git init");
        // Mask the canonical issuectl files via .gitignore.
        fs::write(
            tmp.path().join(".gitignore"),
            ".issuectl/\nissues/.schema.yaml\n",
        )
        .unwrap();
        // Place the canonical files on disk.
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(tmp.path().join(".issuectl/AGENTS.md"), "# placeholder\n").unwrap();
        fs::write(tmp.path().join("issues/.schema.yaml"), "fields: {}\n").unwrap();

        let hits = detect_gitignored_canonical_paths(tmp.path());
        let joined = hits.join("\n");
        assert!(
            joined.contains(".issuectl/AGENTS.md"),
            "expected hit for .issuectl/AGENTS.md, got {hits:?}"
        );
        assert!(
            joined.contains("issues/.schema.yaml"),
            "expected hit for issues/.schema.yaml, got {hits:?}"
        );

        // Full doctor scan surfaces the warning in the report.
        let report = scan(tmp.path()).unwrap();
        assert!(
            !report.gitignored_paths.is_empty(),
            "expected gitignored_paths populated; got empty"
        );
    }

    #[test]
    fn detect_gitignored_canonical_paths_does_not_flag_tracked_files() {
        // Followup-review regression: with `--no-index`, git reports
        // tracked-but-pattern-matched files as ignored, producing
        // false positives. Without `--no-index` (current behavior),
        // a tracked file is correctly considered visible to teammates
        // even when `.gitignore` would otherwise have masked it.
        let tmp = fresh_repo();
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["init", "--quiet"])
            .output()
            .expect("git init");
        assert!(init.status.success());
        // First track the file, THEN add the ignore rule that would
        // have masked it. Tracked files are not ignored from git's
        // perspective.
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(tmp.path().join(".issuectl/AGENTS.md"), "# x\n").unwrap();
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["add", "-f", ".issuectl/AGENTS.md"])
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add failed: {add:?}");
        // Configure user.email/name so commit can succeed even on a
        // CI host without global git config.
        for (k, v) in [
            ("user.email", "test@example.invalid"),
            ("user.name", "test"),
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(["config", k, v])
                .output()
                .expect("git config");
        }
        let commit = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["commit", "--quiet", "-m", "track AGENTS.md"])
            .output()
            .expect("git commit");
        assert!(commit.status.success(), "git commit failed: {commit:?}");
        fs::write(tmp.path().join(".gitignore"), ".issuectl/\n").unwrap();

        let hits = detect_gitignored_canonical_paths(tmp.path());
        assert!(
            hits.is_empty(),
            "tracked file must NOT be flagged as gitignored; got {hits:?}"
        );
    }

    #[test]
    fn detect_gitignored_canonical_paths_silent_when_not_a_git_repo() {
        // Bare tempdir (no `git init`) — `git check-ignore` returns
        // exit 128. Doctor must not crash and must report no hits.
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(tmp.path().join(".issuectl/AGENTS.md"), "# x\n").unwrap();
        let hits = detect_gitignored_canonical_paths(tmp.path());
        assert!(hits.is_empty(), "got {hits:?}");
    }

    #[test]
    fn scan_detects_legacy_numbered_dirs() {
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            1,
            "alpha",
            "---\nnumber: 1\nstatus: open\n---\n# A\n",
        );
        put_legacy(
            &tmp,
            "closed",
            2,
            "beta",
            "---\nnumber: 2\nstatus: done\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.legacy_dirs.len(), 2);
    }

    #[test]
    fn scan_does_not_migrate_user_slug_starting_with_digits() {
        // Regression: a user-overridden slug `100-things-to-fix` looks like
        // legacy `<NN>-<slug>` but is a legitimate new-format issue. The
        // presence of `slug:` in frontmatter is the discriminator —
        // `issuectl new` always writes it for new issues.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/100-things-to-fix");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nslug: 100-things-to-fix\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.legacy_dirs.is_empty(), "should not detect as legacy");
    }

    #[test]
    fn scan_detects_legacy_when_only_dirname_carries_number() {
        // Pre-`number:` repos (early grooveserve issues) had the number
        // only in the dirname; frontmatter has neither `number:` nor
        // `slug:`. These must still migrate.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/42-old-style");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nstatus: open\ntype: feature\n---\n# Old\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.legacy_dirs.len(), 1);
        assert_eq!(r.legacy_dirs[0].old_number, 42);
    }

    #[test]
    fn fix_renames_dirs_and_rewrites_frontmatter() {
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            1,
            "first",
            "---\nnumber: 1\nstatus: open\nepic: 2\nrelated: [\"#3\"]\nblocked_by: [\"#3\"]\n---\n# E1. First\n",
        );
        put_legacy(
            &tmp,
            "open",
            2,
            "epic-one",
            "---\nnumber: 2\nstatus: open\ntype: epic\n---\n# Epic\n",
        );
        put_legacy(
            &tmp,
            "open",
            3,
            "third",
            "---\nnumber: 3\nstatus: open\n---\n# Third\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.fix_applied());
        assert!(
            outcome.blockers.is_empty(),
            "blockers={:?}",
            outcome.blockers
        );
        // Find the migrated 1-first directory.
        let mig1 = outcome
            .legacy_dirs_migrated
            .iter()
            .find(|m| m.old_number == 1)
            .unwrap();
        let item = mig1.new_path.join("item.md");
        let content = fs::read_to_string(&item).unwrap();
        assert!(content.contains(&format!("slug: {}", mig1.new_slug)));
        assert!(!content.contains("number:"));
        assert!(content.contains("# First"), "heading rewritten: {content}");
        // epic: 2 → epic: <slug-of-2>
        let mig2 = outcome
            .legacy_dirs_migrated
            .iter()
            .find(|m| m.old_number == 2)
            .unwrap();
        assert!(content.contains(&format!("epic: {}", mig2.new_slug)));
        // related: ['#3'] → ['@<slug-of-3>'], blocked_by: ['#3'] → ['@<slug-of-3>']
        let mig3 = outcome
            .legacy_dirs_migrated
            .iter()
            .find(|m| m.old_number == 3)
            .unwrap();
        let parsed = write::read_item(&item).unwrap();
        let expected = serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(format!(
            "@{}",
            mig3.new_slug
        ))]);
        for key in ["related", "blocked_by"] {
            let got = parsed
                .frontmatter
                .get(serde_yaml::Value::String(key.into()))
                .unwrap_or_else(|| panic!("`{key}` missing after rewrite: {content}"));
            assert_eq!(got, &expected, "`{key}` not migrated: {content}");
        }
    }

    #[test]
    fn scan_ok_for_clean_slug_repo() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nstatus: open\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.legacy_dirs.is_empty());
        assert!(r.invalid_slugs.is_empty());
    }

    #[test]
    fn scan_flags_invalid_slugs() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/UPPER_NOT_KEBAB");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\n---\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.invalid_slugs.len(), 1);
    }

    #[test]
    fn rewrite_text_swaps_refs_and_paths() {
        let mut nm = BTreeMap::new();
        nm.insert(7, "amber-loud-fox".to_string());
        let mut dm = BTreeMap::new();
        dm.insert("7-something".to_string(), "amber-loud-fox".to_string());
        let amb = BTreeSet::new();
        let text = "See #7 in [link](../7-something/item.md) and #99.\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert!(out.contains("@amber-loud-fox"));
        assert!(out.contains("../amber-loud-fox/item.md"));
        assert!(out.contains("#99"), "unknown number left as-is");
    }

    #[test]
    fn rewrite_text_skips_fenced_code_blocks() {
        let mut nm = BTreeMap::new();
        nm.insert(7, "amber-loud-fox".to_string());
        let dm = BTreeMap::new();
        let amb = BTreeSet::new();
        let text = "Outside #7.\n```rust\n// inside #7\n```\nAfter #7.\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert!(out.contains("Outside @amber-loud-fox"));
        assert!(out.contains("// inside #7"), "code block content untouched");
        assert!(out.contains("After @amber-loud-fox"));
    }

    #[test]
    fn rewrite_text_skips_inline_code_spans() {
        // Inline code is documentation, not a live reference: a
        // user explaining `the old #7 syntax` doesn't want it
        // silently rewritten to `the old @amber-loud-fox syntax`.
        let mut nm = BTreeMap::new();
        nm.insert(7, "amber-loud-fox".to_string());
        let dm = BTreeMap::new();
        let amb = BTreeSet::new();
        let text = "use `#7` literally, but rewrite #7 here.\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert_eq!(
            out,
            "use `#7` literally, but rewrite @amber-loud-fox here.\n"
        );
    }

    #[test]
    fn rewrite_text_still_rewrites_paths_inside_link_urls() {
        // Doctor intentionally rewrites intra-repo paths inside link
        // URLs — that's the whole point of the dir-rename step.
        // (Contrast `refs::rewrite_body_refs`, which DOES skip URLs.)
        let nm = BTreeMap::new();
        let mut dm = BTreeMap::new();
        dm.insert("7-something".to_string(), "amber-loud-fox".to_string());
        let amb = BTreeSet::new();
        let text = "see [link](../7-something/item.md).\n";
        let out = rewrite_text(text, &nm, &dm, &amb);
        assert!(
            out.contains("../amber-loud-fox/item.md"),
            "link-URL path must still be rewritten by doctor: {out:?}"
        );
    }

    #[test]
    fn migrate_notes_heading_renames_outside_fences() {
        let body =
            "---\nstatus: open\n---\n\n# T\n\n## Notes\n\nfirst\n\n```\n## not a heading\n```\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert!(out.contains("## Comments"));
        assert!(!out.contains("## Notes\n"), "Notes heading must be renamed");
        // The fenced `## not a heading` is content and stays put.
        assert!(out.contains("```\n## not a heading\n```"));
    }

    #[test]
    fn migrate_notes_heading_merges_when_both_exist() {
        // Issue @doctor-fix-merge-notes-comments: one `## Notes` and one
        // `## Comments` auto-merge (no manual conflict). `## Notes`
        // preceded `## Comments`, so its entry lands first (document
        // order preserved) and `## Notes` is dropped.
        let body = "## Notes\n\nx\n\n## Comments\n\ny\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict, "both-exist is auto-merged, not a conflict");
        assert_eq!(out, "## Comments\n\nx\n\ny\n");
        assert!(!out.contains("## Notes"), "## Notes must be dropped");
    }

    #[test]
    fn migrate_notes_heading_merge_preserves_document_order_notes_after() {
        // When `## Comments` precedes `## Notes`, the Comments entries
        // stay first and the Notes entries are appended — document
        // order is preserved regardless of which section came first.
        let body = "## Comments\n\ny\n\n## Notes\n\nx\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert_eq!(out, "## Comments\n\ny\n\nx\n");
    }

    #[test]
    fn migrate_notes_heading_merge_preserves_intervening_section() {
        // A section between `## Notes` and `## Comments` is preserved in
        // place; only `## Notes` is folded away.
        let body = "## Notes\n\nx\n\n## Decisions\n\nd\n\n## Comments\n\ny\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert_eq!(out, "## Decisions\n\nd\n\n## Comments\n\nx\n\ny\n");
    }

    #[test]
    fn migrate_notes_heading_flags_conflict_when_multiple_notes() {
        // Round-2 finding G5/O5: rewriting two `## Notes` would
        // produce two `## Comments`, leaving the second stranded.
        let body = "## Notes\n\na\n\n## Decisions\n\nx\n\n## Notes\n\nb\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(conflict, "multiple ## Notes must flag a conflict");
        assert_eq!(out, body);
    }

    #[test]
    fn doctor_scan_surfaces_pending_notes_migrations() {
        // Round-2 finding O6: read-only scan must populate
        // `notes_to_rename` and `notes_conflicts` so users see the
        // work even before running --fix.
        let tmp = fresh_repo();
        let safe = tmp.path().join("issues/safe-rename");
        fs::create_dir_all(&safe).unwrap();
        fs::write(
            safe.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\nold\n",
        )
        .unwrap();
        // One `## Notes` + one `## Comments` is now an auto-merge, so it
        // joins `notes_to_rename`, not `notes_conflicts`.
        let merge = tmp.path().join("issues/has-both");
        fs::create_dir_all(&merge).unwrap();
        fs::write(
            merge.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\nx\n\n## Comments\n\ny\n",
        )
        .unwrap();
        // Multiple `## Notes` stays an ambiguous conflict.
        let conflict = tmp.path().join("issues/two-notes");
        fs::create_dir_all(&conflict).unwrap();
        fs::write(
            conflict.join("item.md"),
            "---\nstatus: open\n---\n\n## Notes\n\na\n\n## Decisions\n\nd\n\n## Notes\n\nb\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        let mut to_rename = r.notes_to_rename.clone();
        to_rename.sort();
        assert_eq!(
            to_rename,
            vec!["has-both".to_string(), "safe-rename".to_string()]
        );
        assert_eq!(r.notes_conflicts, vec!["two-notes".to_string()]);
    }

    #[test]
    fn migrate_notes_heading_skips_fenced_only_occurrence() {
        let body = "```\n## Notes\n```\n";
        let (out, conflict) = migrate_notes_heading(body);
        assert!(!conflict);
        assert_eq!(out, body, "fenced ## Notes is content, not a heading");
    }

    #[test]
    fn doctor_fix_renames_notes_to_comments_in_flat_layout() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/legacy-notes-here");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n## Notes\n\nold note\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(after.contains("## Comments"));
        assert!(!after.contains("## Notes"));
        assert!(after.contains("old note"));
        assert_eq!(outcome.notes_renamed, vec!["legacy-notes-here".to_string()]);
    }

    #[test]
    fn doctor_fix_merges_notes_into_comments_when_both_exist() {
        // Issue @doctor-fix-merge-notes-comments: a body with BOTH
        // `## Notes` and `## Comments` is auto-merged by `--fix`
        // (document order preserved, `## Notes` dropped) — it no longer
        // surfaces as a manual-merge conflict, so nothing lands in
        // `notes_conflicts_at_apply` and the apply completes cleanly.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/has-both");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n# T\n\n## Notes\n\nfirst\n\n## Comments\n\nsecond\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        assert!(
            r.notes_conflicts.is_empty(),
            "both-exist must not be a scan conflict, got {:?}",
            r.notes_conflicts
        );
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("item.md")).unwrap();
        assert!(
            !after.contains("## Notes"),
            "## Notes must be dropped: {after}"
        );
        assert_eq!(after.matches("## Comments").count(), 1, "single Comments");
        // Document order preserved: the Notes entry precedes the
        // existing Comments entry.
        let first = after.find("first").expect("Notes entry retained");
        let second = after.find("second").expect("Comments entry retained");
        assert!(first < second, "Notes entry must come first: {after}");
        assert_eq!(outcome.notes_renamed, vec!["has-both".to_string()]);
        assert!(
            outcome.notes_conflicts_at_apply.is_empty(),
            "no manual-merge leftovers: {:?}",
            outcome.notes_conflicts_at_apply
        );
        // A second doctor run is a clean no-op (idempotent merge) and
        // leaves the file byte-for-byte unchanged.
        let mut r2 = scan(tmp.path()).unwrap();
        assert!(r2.notes_to_rename.is_empty() && r2.notes_conflicts.is_empty());
        let actions2 = DoctorActions::from_findings(&mut r2);
        apply(
            tmp.path(),
            actions2,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after_second = fs::read_to_string(dir.join("item.md")).unwrap();
        assert_eq!(after, after_second, "second --fix must not mutate the file");
    }

    #[test]
    fn fix_does_not_touch_files_outside_issues() {
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            1,
            "alpha",
            "---\nnumber: 1\nstatus: open\n---\n# A\n",
        );
        // CHANGELOG references `#1` legitimately (release note style).
        let changelog = tmp.path().join("CHANGELOG.md");
        fs::write(&changelog, "# CHANGELOG\n\n- Fixed #1 regression\n").unwrap();
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after = fs::read_to_string(&changelog).unwrap();
        assert!(
            after.contains("Fixed #1 regression"),
            "CHANGELOG outside issues/ must not be rewritten, got: {after}"
        );
    }

    #[test]
    fn scan_flags_schema_violation_for_missing_required_field() {
        let tmp = fresh_repo();
        // Issue missing `priority` (required by default schema).
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("priority")),
            "expected `priority` violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn scan_flags_schema_violation_for_invalid_enum() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: nonsense\ncreated: 2026-01-01\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("type") && msg.contains("nonsense")),
            "expected enum violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn scan_reports_schema_missing_when_file_absent() {
        let tmp = fresh_repo();
        let r = scan(tmp.path()).unwrap();
        assert!(r.schema_missing);
        assert!(r.schema_parse_error.is_none());
    }

    #[test]
    fn fix_writes_default_schema_when_missing() {
        let tmp = fresh_repo();
        let mut r = scan(tmp.path()).unwrap();
        assert!(r.schema_missing);
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let path = tmp.path().join("issues/.schema.yaml");
        assert!(path.is_file(), "schema file should be auto-written");
        // Should contain the canonical built-in fields.
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type:"));
        assert!(content.contains("status:"));
        // Bug #3: schema bootstrap must surface in `fix_applied`.
        // Previously a `--fix` that only wrote `.schema.yaml` reported
        // `fix_applied: false`; with `ApplyOutcome::schema_bootstrapped`
        // pulled into the predicate, it now reports `true`.
        assert!(
            outcome.schema_bootstrapped,
            "expected schema bootstrap to be recorded"
        );
        assert!(
            outcome.fix_applied(),
            "schema-only --fix must report fix_applied=true"
        );
    }

    #[test]
    fn scan_skips_legacy_dirs_for_schema_violations() {
        // A legacy <NN>-<slug> dir is rewritten by --fix; flagging it
        // as schema-violating would just be noise.
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            7,
            "alpha",
            "---\nnumber: 7\nstatus: open\n---\n# A\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations.is_empty(),
            "legacy dirs should not generate schema violations, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn schema_walk_reports_malformed_yaml_as_parse_error() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        // Frontmatter that the lenient `Mapping` parser also rejects.
        fs::write(dir.join("item.md"), "---\nfoo: : :\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.parse_errors.iter().any(|e| e.message.contains("YAML")
                || e.message.contains("yaml")
                || e.message.contains("invalid")),
            "expected parse error report, got {:?}",
            r.parse_errors
        );
        // Bug #6: hard parse errors are typed at the source — no
        // substring matching. Re-wording the parser message no longer
        // reclassifies a hard fail as a soft warn.
        assert!(
            r.parse_errors
                .iter()
                .any(|e| e.severity == ParseSeverity::Hard),
            "unparseable frontmatter must classify as Hard: {:?}",
            r.parse_errors
        );
    }

    #[test]
    fn schema_walk_uses_repo_relative_paths_not_flat_prefix() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        // Missing `priority`. Location must be a real path, not "flat/...".
        let (loc, _) = r
            .schema_violations
            .iter()
            .find(|(_, msg)| msg.contains("priority"))
            .expect("expected priority violation");
        assert!(loc.contains("issues/quiet-brave-otter"), "got {loc:?}");
        assert!(!loc.starts_with("flat/"), "got {loc:?}");
    }

    #[test]
    fn schema_walk_does_not_skip_flat_issue_with_legacy_shape_name() {
        // A user who passes `--slug 12-things-to-do` ends up with a
        // flat-layout issue whose name matches the legacy `<NN>-<slug>`
        // shape. `issuectl new` writes a `slug:` field for new issues,
        // and that `slug:` is the typed signal that suppresses the
        // numbered-legacy classification — without it, doctor would
        // queue this dir for the NN-rename pipeline and skip schema
        // checks. With `slug:` present, schema validation must run.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/12-things-to-do");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nslug: 12-things-to-do\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.legacy_dirs.is_empty(),
            "modern flat issue with `slug:` must not be queued for NN-rename, got {:?}",
            r.legacy_dirs
        );
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("priority")),
            "expected violation on flat NN-shaped slug, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn flat_issue_with_legacy_shape_name_and_no_slug_field_is_legacy() {
        // Mirror image of the test above: a flat-layout dir whose
        // name matches `<NN>-<slug>` but whose frontmatter omits the
        // `slug:` field is classified legacy and queued for NN-rename.
        // This is the canonical "old hand-authored issue" case — the
        // user's intended dir name is canonicalised by `--fix`.
        // Lints (schema/refs/timestamps/...) are SUPPRESSED on this
        // dir because `--fix` rewrites its frontmatter wholesale;
        // surfacing them would refuse the very fix designed to heal
        // them.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/12-things-to-do");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert_eq!(
            r.legacy_dirs.len(),
            1,
            "flat NN-shape with no `slug:` must be classified legacy, got {:?}",
            r.legacy_dirs
        );
        assert_eq!(r.legacy_dirs[0].old_number, 12);
        assert!(
            r.schema_violations.is_empty(),
            "lints must be suppressed for legacy-classified dirs, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn stray_number_field_with_slug_does_not_classify_modern_issue_as_legacy() {
        // Regression: a modern flat issue carrying `slug:` AND a stray
        // `number:` field (left over from a botched manual edit) must
        // NOT be classified legacy. `slug:` short-circuits before
        // `number:` in `legacy_number_from_mapping`, so this dir keeps
        // its name and gets full schema validation.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\nslug: quiet-brave-otter\nnumber: 7\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.legacy_dirs.is_empty(),
            "modern issue with stray `number:` must not be queued for NN-rename, got {:?}",
            r.legacy_dirs
        );
    }

    fn put_flat(tmp: &TempDir, slug: &str, body: &str) {
        let dir = tmp.path().join("issues").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), body).unwrap();
    }

    #[test]
    fn deferred_label_is_detected_without_confusing_deferred_status() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "deferred-labelled-issue",
            "---\ncreated: 2020-01-01\nupdated: 2020-01-01\ntype: feature\nreporter: test\nstatus: in-progress\npriority: normal\nlabels: [deferred, ui]\n---\n# Labelled\n",
        );
        put_flat(
            &tmp,
            "status-only-issue",
            "---\ncreated: 2020-01-01\nupdated: 2020-01-01\ntype: feature\nreporter: test\nstatus: deferred\npriority: normal\n---\n# Status only\n",
        );

        let report = scan(tmp.path()).unwrap();
        assert_eq!(
            report
                .deferred_labels
                .iter()
                .map(|(slug, _)| slug.as_str())
                .collect::<Vec<_>>(),
            vec!["deferred-labelled-issue"]
        );
        assert!(
            critical_blockers(&report).is_empty(),
            "retired labels are warning-only: {:?}",
            critical_blockers(&report)
        );
        let json = render_json(&report, None, false, tmp.path());
        assert_eq!(
            json["deferred_labels"],
            serde_json::json!(["deferred-labelled-issue"])
        );
    }

    #[test]
    fn deferred_label_preserves_legacy_intake_migration_evidence() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "pending-intake-migrate",
            "---\ncreated: 2020-01-01\nupdated: 2020-01-01\ntype: feature\nreporter: test\nstatus: open\npriority: normal\nlabels: [deferred]\n---\n# Pending\n",
        );

        let mut report = scan(tmp.path()).unwrap();
        assert_eq!(report.deferred_labels.len(), 1);
        assert_eq!(report.deferred_labels_require_intake_migrate.len(), 1);
        let actions = DoctorActions::from_findings(&mut report);
        assert!(actions.deferred_labels.is_empty());
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.deferred_labels_removed.is_empty());
        let text =
            fs::read_to_string(tmp.path().join("issues/pending-intake-migrate/item.md")).unwrap();
        assert!(text.contains("status: open"), "{text}");
        assert!(text.contains("labels: [deferred]"), "{text}");

        let rescanned = scan_issues(tmp.path()).unwrap();
        let issue = &rescanned.issues[0].parsed.as_ref().unwrap().issue;
        let plan = crate::mutate::intake_migrate::plan_issue(issue, &schema::default_schema())
            .expect("legacy migration must remain actionable");
        assert_eq!(plan.status_change, Some(("open".into(), "deferred".into())));
    }

    #[test]
    fn deferred_label_fix_removes_only_that_label() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "deferred-labelled-issue",
            "---\ncreated: 2020-01-01\nupdated: 2020-01-01\ntype: feature\nreporter: test\nstatus: in-progress\npriority: normal\nlabels: [deferred, ui]\n---\n# Labelled\n",
        );

        let mut report = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut report);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(
            outcome.deferred_labels_removed,
            vec!["deferred-labelled-issue"]
        );
        assert!(outcome.fix_applied());
        let text =
            fs::read_to_string(tmp.path().join("issues/deferred-labelled-issue/item.md")).unwrap();
        assert!(!text.contains("deferred"), "{text}");
        assert!(
            text.contains("labels: [ui]"),
            "unrelated label must remain: {text}"
        );
        assert!(scan(tmp.path()).unwrap().deferred_labels.is_empty());
    }

    #[test]
    fn deferred_label_failure_retains_partial_progress_in_outcome() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "first-valid-label",
            "---\ntype: feature\nstatus: in-progress\npriority: normal\nlabels: [deferred]\n---\n# First\n",
        );
        put_flat(
            &tmp,
            "second-broken-label",
            "---\nlabels: [deferred\n---\n# Broken\n",
        );
        let mut actions = DoctorActions {
            deferred_labels: vec![
                (
                    "first-valid-label".into(),
                    tmp.path().join("issues/first-valid-label/item.md"),
                ),
                (
                    "second-broken-label".into(),
                    tmp.path().join("issues/second-broken-label/item.md"),
                ),
            ],
            ..Default::default()
        };
        let mut outcome = ApplyOutcome::default();

        apply_deferred_label_removal(&mut actions, &mut outcome);

        assert_eq!(outcome.deferred_labels_removed, vec!["first-valid-label"]);
        assert!(outcome
            .apply_error
            .as_deref()
            .is_some_and(|error| error.contains("second-broken-label")));
        assert!(
            !fs::read_to_string(tmp.path().join("issues/first-valid-label/item.md"))
                .unwrap()
                .contains("deferred")
        );
    }

    #[test]
    fn deferred_label_fix_is_a_noop_when_no_issue_uses_it() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "plain-other-label",
            "---\ncreated: 2020-01-01\nupdated: 2020-01-01\ntype: feature\nreporter: test\nstatus: open\npriority: normal\nlabels: [ui]\n---\n# Plain\n",
        );

        let mut report = scan(tmp.path()).unwrap();
        assert!(report.deferred_labels.is_empty());
        let actions = DoctorActions::from_findings(&mut report);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.deferred_labels_removed.is_empty());
        assert!(scan(tmp.path()).unwrap().deferred_labels.is_empty());
    }

    #[test]
    fn unknown_reviewer_is_flagged_unless_a_known_user_elsewhere() {
        // alice is the assignee of issue-one → known user.
        // bob is only ever referenced as a reviewer → flagged.
        // alice as reviewer on issue-two → accepted.
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "alpha-known-user",
            "---\ntype: bug\nstatus: open\npriority: normal\nassignee: alice\n---\n# One\n",
        );
        put_flat(
            &tmp,
            "beta-known-reviewer",
            "---\ntype: bug\nstatus: open\npriority: normal\nreviewer: alice\nreview_status: requested\n---\n# Two\n",
        );
        put_flat(
            &tmp,
            "gamma-unknown-reviewer",
            "---\ntype: bug\nstatus: open\npriority: normal\nreviewer: bob\nreview_status: in-review\n---\n# Three\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.unknown_reviewers
                .iter()
                .any(|(slug, who)| slug == "gamma-unknown-reviewer" && who == "bob"),
            "expected bob flagged, got {:?}",
            r.unknown_reviewers
        );
        assert!(
            !r.unknown_reviewers.iter().any(|(_, who)| who == "alice"),
            "alice is a known user; must not be flagged: {:?}",
            r.unknown_reviewers
        );
        // review_status must not show up under unknown_keys — the schema
        // declares it.
        assert!(
            !r.unknown_keys
                .iter()
                .any(|(_, k)| k == "review_status" || k == "reviewer"),
            "reviewer/review_status are schema-known: {:?}",
            r.unknown_keys
        );
    }

    #[test]
    fn flags_custom_closing_status_without_closed_field() {
        // Schema declares `archived` as closing. An issue at
        // `status: archived` without a `closed:` date must be flagged
        // by status_consistency, just like a built-in `done` would be.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, archived]\nstatus_classes:\n  archived: closing\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "alpha-issue-here",
            "---\ntype: bug\nstatus: archived\npriority: normal\n---\n# A\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency
                .iter()
                .any(|(slug, msg)| slug == "alpha-issue-here" && msg.contains("archived")),
            "expected archived-without-closed flagged, got {:?}",
            r.status_consistency
        );
    }

    #[test]
    fn flags_custom_active_status_carrying_closed_field() {
        // Schema declares `verified` as active. An issue with
        // `status: verified` AND `closed: <date>` must be flagged —
        // active statuses must not carry `closed:`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, verified]\nstatus_classes:\n  verified: active\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "beta-issue-here",
            "---\ntype: bug\nstatus: verified\nclosed: 2026-05-06\npriority: normal\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency
                .iter()
                .any(|(slug, msg)| slug == "beta-issue-here" && msg.contains("verified")),
            "expected verified-with-closed flagged, got {:?}",
            r.status_consistency
        );
    }

    #[test]
    fn flags_active_status_carrying_closed_by() {
        // A `closed_by:` on an active issue is self-inconsistent (the
        // close path scrubs it on the active edge), so doctor flags it
        // alongside the `closed:` heal — even when `closed:` itself is
        // already absent.
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "stranded-closer-here",
            "---\ntype: bug\nstatus: open\npriority: normal\nclosed_by: alice\n---\n# S\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency
                .iter()
                .any(|(slug, msg)| slug == "stranded-closer-here" && msg.contains("closed_by")),
            "expected stranded closed_by flagged, got {:?}",
            r.status_consistency
        );
    }

    #[test]
    fn read_only_detects_status_alias_and_suppresses_enum_violation() {
        // A legacy `status: closed` value (built-in alias → done) must
        // show up as a pending coercion, NOT as an enum schema
        // violation (the coercion is the fix).
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legacy-status-issue",
            "---\ntype: bug\nstatus: closed\npriority: normal\n---\n# L\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.alias_coercions
                .iter()
                .any(|(slug, field, from, to, _)| slug == "legacy-status-issue"
                    && field == "status"
                    && from == "closed"
                    && to == "done"),
            "expected status closed→done coercion, got {:?}",
            r.alias_coercions
        );
        assert!(
            !r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("\"closed\"") && msg.contains("status")),
            "aliasable status must not be reported as an enum violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn fix_coerces_status_alias_and_stamps_closed() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legacy-status-issue",
            "---\ntype: bug\nstatus: closed\npriority: normal\n---\n# L\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .alias_coercions_applied
                .iter()
                .any(|(slug, field, from, to)| slug == "legacy-status-issue"
                    && field == "status"
                    && from == "closed"
                    && to == "done"),
            "expected applied coercion, got {:?}",
            outcome.alias_coercions_applied
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/legacy-status-issue/item.md")).unwrap();
        assert!(
            after.contains("status: done"),
            "status not coerced:\n{after}"
        );
        // `done` is a closing status, so `closed:` must be backfilled.
        assert!(
            after.contains("closed:"),
            "closed: not stamped on coerced closing status:\n{after}"
        );
    }

    #[test]
    fn fix_coerces_type_alias_without_touching_closed() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legacy-type-issue",
            "---\ntype: enhancement\nstatus: open\npriority: normal\n---\n# T\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .alias_coercions_applied
                .iter()
                .any(|(_, field, from, to)| field == "type"
                    && from == "enhancement"
                    && to == "improvement"),
            "expected type coercion, got {:?}",
            outcome.alias_coercions_applied
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/legacy-type-issue/item.md")).unwrap();
        assert!(
            after.contains("type: improvement"),
            "type not coerced:\n{after}"
        );
        // Active status → no `closed:` stamped.
        assert!(
            !after.contains("closed:"),
            "closed: must not appear:\n{after}"
        );
    }

    #[test]
    fn fix_stamps_closed_from_git_commit_date_not_today() {
        // A legacy issue closed long ago: coercing `status: closed` →
        // `done` must backfill `closed:` from the file's last git commit
        // date, not today() — otherwise a years-old issue gets a brand
        // new closed date.
        let tmp = fresh_repo();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(args)
                .output()
                .expect("git");
            assert!(st.status.success(), "git {args:?} failed");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Test"]);
        put_flat(
            &tmp,
            "ancient-closed-issue",
            "---\ntype: bug\nstatus: closed\npriority: normal\n---\n# A\n",
        );
        git(&["add", "."]);
        // Pin BOTH author and committer date so `%aI` is deterministic.
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args([
                "commit",
                "--quiet",
                "-m",
                "import",
                "--date=2020-01-15T12:00:00",
            ])
            .env("GIT_COMMITTER_DATE", "2020-01-15T12:00:00")
            .output()
            .expect("git commit");

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let after =
            fs::read_to_string(tmp.path().join("issues/ancient-closed-issue/item.md")).unwrap();
        assert!(
            after.contains("status: done"),
            "status not coerced:\n{after}"
        );
        assert!(
            after.contains("closed: 2020-01-15"),
            "expected closed date from git commit, got:\n{after}"
        );
    }

    #[test]
    fn derive_closed_date_falls_back_to_mtime_when_untracked() {
        // Not a git repo (no .git): git_last_commit_date returns None,
        // so the mtime fallback supplies a valid YYYY-MM-DD.
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "untracked-issue",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# U\n",
        );
        let path = tmp.path().join("issues/untracked-issue/item.md");
        let date = derive_closed_date(&path);
        assert!(
            chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_ok(),
            "expected a valid YYYY-MM-DD, got {date:?}"
        );
        // A file just created in this test has an mtime of ~now, so the
        // mtime tier should resolve to today (both use local time).
        assert_eq!(date, write::today(), "mtime fallback should be today");
    }

    #[test]
    fn fixed_clock_supplies_legacy_closed_date_when_history_is_unavailable() {
        use chrono::TimeZone;

        let tmp = fresh_repo();
        let clock = crate::clock::FixedClock::new(
            chrono::Utc.with_ymd_and_hms(2026, 2, 28, 12, 0, 0).unwrap(),
        );
        assert_eq!(
            derive_closed_date_via(&tmp.path().join("missing/item.md"), &clock),
            "2026-02-28"
        );
    }

    #[test]
    fn fix_batches_status_and_type_coercions_for_one_issue() {
        // An issue with BOTH a status and a type coercion is read once
        // and written once. The behavioral proof: both fields land
        // correctly in the same file and `closed:` is stamped exactly
        // once (a per-field read+write could double-stamp or clobber).
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "double-coercion-issue",
            "---\ntype: enhancement\nstatus: closed\npriority: normal\n---\n# D\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        // Scan must plan both coercions for the one issue.
        let planned: Vec<_> = r
            .alias_coercions
            .iter()
            .filter(|(slug, ..)| slug == "double-coercion-issue")
            .map(|(_, field, ..)| field.clone())
            .collect();
        assert!(
            planned.contains(&"status".to_string()) && planned.contains(&"type".to_string()),
            "expected both status+type coercions planned, got {planned:?}"
        );

        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let applied_fields: Vec<_> = outcome
            .alias_coercions_applied
            .iter()
            .filter(|(slug, ..)| slug == "double-coercion-issue")
            .map(|(_, field, ..)| field.clone())
            .collect();
        assert!(
            applied_fields.contains(&"status".to_string())
                && applied_fields.contains(&"type".to_string()),
            "expected both coercions applied, got {applied_fields:?}"
        );

        let after =
            fs::read_to_string(tmp.path().join("issues/double-coercion-issue/item.md")).unwrap();
        assert!(
            after.contains("status: done"),
            "status not coerced:\n{after}"
        );
        assert!(
            after.contains("type: improvement"),
            "type not coerced:\n{after}"
        );
        assert_eq!(
            after.matches("closed:").count(),
            1,
            "closed: must be stamped exactly once:\n{after}"
        );
    }

    #[test]
    fn reconciliation_stamps_closed_from_git_commit_date() {
        // Status/folder reconciliation (closed/<slug> carrying an active
        // status) must also derive its backfilled `closed:` from git
        // history rather than today().
        let tmp = fresh_repo();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(args)
                .output()
                .expect("git");
            assert!(st.status.success(), "git {args:?} failed");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Test"]);
        // Legacy layout: an active status sitting under issues/closed/.
        let dir = tmp.path().join("issues/closed/legacy-folder-issue");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: in-progress\npriority: normal\n---\n# F\n",
        )
        .unwrap();
        git(&["add", "."]);
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args([
                "commit",
                "--quiet",
                "-m",
                "import",
                "--date=2019-06-10T09:00:00",
            ])
            .env("GIT_COMMITTER_DATE", "2019-06-10T09:00:00")
            .output()
            .expect("git commit");

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        // After flat-layout migration the file lives at issues/<slug>/.
        let after =
            fs::read_to_string(tmp.path().join("issues/legacy-folder-issue/item.md")).unwrap();
        assert!(
            after.contains("closed: 2019-06-10"),
            "expected reconciled closed date from git, got:\n{after}"
        );
    }

    #[test]
    fn custom_required_when_field_surfaces_as_schema_violation() {
        // Regression: doctor must NOT swallow a user-declared
        // `required_when` on a field other than `closed` — only the
        // built-in closed/closing rule is suppressed (it has a separate
        // reporting channel). A custom `resolution` required-when-closing
        // field has no other channel, so a missing value must show up in
        // `schema_violations`.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  resolution:\n    required_when:\n      status_class: closing\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "needs-resolution",
            "---\ntype: bug\nstatus: done\npriority: normal\nclosed: 2026-05-06\n---\n# R\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(loc, msg)| loc.contains("needs-resolution") && msg.contains("resolution")),
            "custom required_when must surface in schema_violations, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn value_valid_in_user_enum_is_not_coerced() {
        // A repo that adds a built-in alias KEY to its own status enum
        // makes that value canonical — it must not be silently coerced.
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  status:\n    required: true\n    enum: [open, in-progress, resolved, done]\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "keeps-resolved",
            "---\ntype: bug\nstatus: resolved\npriority: normal\n---\n# K\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.alias_coercions.is_empty(),
            "value present in the user enum must not be coerced, got {:?}",
            r.alias_coercions
        );
        assert!(
            !r.schema_violations
                .iter()
                .any(|(loc, _)| loc.contains("keeps-resolved")),
            "a canonical (enum-valid) value must not be flagged, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn flags_broken_epic_ref() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: nonexistent-ghost-fox\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_refs
                .iter()
                .any(|(s, k, t)| s == "quiet-brave-otter"
                    && k == "epic"
                    && t == "nonexistent-ghost-fox"),
            "broken_refs={:?}",
            r.broken_refs
        );
    }

    #[test]
    fn flags_numeric_legacy_ref_in_flat_repo() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: 5\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_refs
                .iter()
                .any(|(_, k, t)| k == "epic" && t.contains("legacy")),
            "expected legacy-numeric flag, got {:?}",
            r.broken_refs
        );
    }

    #[test]
    fn conflict_marker_check_skips_fenced_blocks() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n```\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n```\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.conflict_markers.is_empty(),
            "got {:?}",
            r.conflict_markers
        );
    }

    #[test]
    fn detect_cycles_visits_each_node_once() {
        // Acyclic diamond: A→B, A→C, B→D, C→D. Without the visited
        // set, D was traversed from both B and C; with it, the
        // traversal stops after the first complete walk.
        let mut g: BTreeMap<String, Vec<String>> = BTreeMap::new();
        g.insert("a".into(), vec!["b".into(), "c".into()]);
        g.insert("b".into(), vec!["d".into()]);
        g.insert("c".into(), vec!["d".into()]);
        g.insert("d".into(), vec![]);
        let cycles = detect_cycles(&g);
        assert!(cycles.is_empty(), "no cycles in DAG, got {cycles:?}");
    }

    #[test]
    fn does_not_flag_existing_epic_ref() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "real-epic-here",
            "---\ntype: epic\nstatus: open\npriority: normal\n---\n# E\n",
        );
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: real-epic-here\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.broken_refs.is_empty(), "got {:?}", r.broken_refs);
    }

    #[test]
    fn flags_broken_blocked_by_ref() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@nope-not-here']\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .broken_refs
            .iter()
            .any(|(_, k, t)| k == "blocked_by" && t == "nope-not-here"));
    }

    #[test]
    fn detects_blocked_by_cycle() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@beta-bright-cat']\n---\n# A\n",
        );
        put_flat(
            &tmp,
            "beta-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@alpha-bright-cat']\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.blocked_by_cycles.len(), 1);
        let cycle = &r.blocked_by_cycles[0];
        assert_eq!(cycle[0], "alpha-bright-cat");
        assert!(cycle.contains(&"beta-bright-cat".to_string()));
    }

    #[test]
    fn detects_blocked_by_self_dependency() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "self-loop-target",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@self-loop-target']\n---\n# S\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(r.blocked_by_self, vec!["self-loop-target".to_string()]);
        // The 1-node "cycle" must not also be reported as a cycle:
        // the self-dep branch claims it as its own finding so the user
        // gets a focused message.
        assert!(
            r.blocked_by_cycles.is_empty(),
            "self-dep should be deduped from cycle list: {:?}",
            r.blocked_by_cycles
        );
    }

    #[test]
    fn no_cycle_for_acyclic_chain() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nblocked_by: ['@beta-bright-cat']\n---\n# A\n",
        );
        put_flat(
            &tmp,
            "beta-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# B\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.blocked_by_cycles.is_empty());
    }

    #[test]
    fn flags_closing_status_without_closed_date() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: done\npriority: normal\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .status_consistency
            .iter()
            .any(|(_, m)| m.contains("closing") && m.contains("closed")));
    }

    #[test]
    fn flags_active_status_with_closed_date() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .status_consistency
            .iter()
            .any(|(_, m)| m.contains("active") && m.contains("closed")));
    }

    #[test]
    fn does_not_flag_consistent_status() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: done\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.status_consistency.is_empty(),
            "{:?}",
            r.status_consistency
        );
    }

    #[test]
    fn flags_created_after_updated() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-05-01\nupdated: 2026-04-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .timestamp_issues
            .iter()
            .any(|(_, m)| m.contains("created") && m.contains("after")));
    }

    #[test]
    fn flags_future_dates() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2999-01-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.timestamp_issues.iter().any(|(_, m)| m.contains("future")));
    }

    #[test]
    fn does_not_flag_sane_dates() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\nupdated: 2026-02-01\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.timestamp_issues.is_empty(), "{:?}", r.timestamp_issues);
    }

    #[test]
    fn flags_unknown_frontmatter_key() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nwhimsy: 1\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.unknown_keys.iter().any(|(_, k)| k == "whimsy"));
    }

    #[test]
    fn does_not_flag_schema_known_custom_key() {
        let tmp = fresh_repo();
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: false\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\nteam: payments\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            !r.unknown_keys.iter().any(|(_, k)| k == "team"),
            "team is schema-known: {:?}",
            r.unknown_keys
        );
    }

    #[test]
    fn preflight_aggregates_blockers_in_one_message() {
        // Repo with TWO independent blockers: a slug present in both
        // legacy folders + a file with conflict markers. The user
        // should see both in a single bail, not have to iterate.
        let tmp = fresh_repo();
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        let conflicted = tmp.path().join("issues/alpha-bright-cat");
        fs::create_dir_all(&conflicted).unwrap();
        fs::write(
            conflicted.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> b\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        // Bug #1: preflight blockers MUST NOT bail — they ride on
        // ApplyOutcome.blockers so `--json --fix` consumers receive
        // structured output instead of an anyhow stderr blob.
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!outcome.blockers.is_empty(), "expected preflight blockers");
        let joined = outcome.blockers.join("\n");
        assert!(
            joined.contains("BOTH"),
            "missing both-folders blocker: {joined}"
        );
        assert!(
            joined.contains("merge-conflict markers"),
            "missing conflict-marker blocker: {joined}"
        );
        // Schema bootstrap fires unconditionally before preflight
        // refusal (issue: @unreasonably-attractive-star), so a fresh
        // repo always reports `schema_bootstrapped: true` even on the
        // preflight-blocked path. The contract is that NO OTHER write
        // landed — the preflight blockers still gate every other
        // phase.
        assert!(
            outcome.schema_bootstrapped,
            "schema bootstrap is unconditional, must run even on preflight bail"
        );
        assert!(
            outcome.legacy_dirs_migrated.is_empty()
                && outcome.flat_layout_migrated.is_empty()
                && outcome.notes_renamed.is_empty()
                && outcome.orphan_tempfiles_removed.is_empty()
                && outcome.status_reconciled.is_empty()
                && outcome.files_rewritten == 0
                && !outcome.agents_md_regenerated
                && !outcome.issues_agents_md_rewritten,
            "preflight-blocked apply must not run any phase beyond schema bootstrap"
        );
    }

    #[test]
    fn preflight_does_not_block_on_soft_parse_warnings() {
        // A legacy-numeric epic ref produces a parser warning (now
        // categorised as "soft") but should NOT prevent --fix from
        // running its migration pass.
        let tmp = fresh_repo();
        put_legacy(
            &tmp,
            "open",
            7,
            "alpha",
            "---\nnumber: 7\nstatus: open\nepic: 12\n---\n# A\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        // Pre-fix: there are likely parser warnings in `parse_errors`.
        // None of them should trip the hard-error preflight check.
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .expect("--fix should not refuse on soft parse warnings");
        assert!(
            outcome.blockers.is_empty(),
            "soft parse warnings must not block: {:?}",
            outcome.blockers
        );
    }

    #[test]
    fn flags_conflict_markers_and_apply_refuses() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n",
        );
        let mut r = scan(tmp.path()).unwrap();
        assert!(r.conflict_markers.iter().any(|s| s == "quiet-brave-otter"));
        let before =
            fs::read_to_string(tmp.path().join("issues/quiet-brave-otter/item.md")).unwrap();
        // Preflight blocks before any mutation against the conflict
        // file — but produces a structured `ApplyOutcome.blockers`
        // rather than an Err. Schema bootstrap can land (it precedes
        // preflight, see @unreasonably-attractive-star); the conflict
        // file itself MUST NOT be touched.
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .blockers
                .iter()
                .any(|b| b.contains("merge-conflict markers")),
            "got: {:?}",
            outcome.blockers
        );
        let after =
            fs::read_to_string(tmp.path().join("issues/quiet-brave-otter/item.md")).unwrap();
        assert_eq!(before, after, "conflict markers must not be auto-fixed");
    }

    #[test]
    fn does_not_flag_clean_file() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(r.conflict_markers.is_empty());
    }

    #[test]
    fn detects_and_removes_orphan_tempfiles_with_fix() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        );
        let orphan = tmp
            .path()
            .join("issues/quiet-brave-otter/.issuectl-tmp-XYZ");
        fs::write(&orphan, "leftover").unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.orphan_tempfiles.iter().any(|p| p == &orphan));
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!orphan.exists(), "tempfile should be removed by --fix");
        assert!(outcome
            .orphan_tempfiles_removed
            .iter()
            .any(|p| p == &orphan));
    }

    #[test]
    fn flags_both_open_and_closed_present() {
        let tmp = fresh_repo();
        let s = "quiet-brave-otter";
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join(s);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        let r = scan(tmp.path()).unwrap();
        assert!(r.both_open_and_closed.iter().any(|x| x == s));
    }

    #[test]
    fn reconciles_closed_with_active_status() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/closed/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        assert!(r
            .closed_with_active_status
            .iter()
            .any(|(s, _, _)| s == "quiet-brave-otter"));
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        // Flat-layout migration runs in the same apply pass and moves
        // the file from `issues/closed/<slug>/` to `issues/<slug>/`.
        let migrated = tmp.path().join("issues/quiet-brave-otter/item.md");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(after.contains("status: done"), "got: {after}");
        assert!(after.contains("closed:"));
    }

    #[test]
    fn reconciles_open_with_closing_status() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: done\npriority: normal\nclosed: 2026-01-01\n---\n# T\n",
        )
        .unwrap();
        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        let migrated = tmp.path().join("issues/quiet-brave-otter/item.md");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(after.contains("status: open"), "got: {after}");
        assert!(
            !after.contains("closed:"),
            "closed should be dropped: {after}"
        );
    }

    /// Issue @doctor-fix-noop: notes/comments conflicts are NOT
    /// preflight blockers. They surface in `outcome.notes_conflicts_at_apply`
    /// and let other phases (NN-rename, alias coercion, AGENTS.md
    /// regen) run normally. The post-apply rescan still picks up the
    /// conflict via `findings.notes_conflicts` so the user sees it.
    #[test]
    fn post_flat_layout_notes_conflict_surfaces_without_blocking() {
        let tmp = fresh_repo();
        let foo = tmp.path().join("issues/open/foo-bar");
        fs::create_dir_all(&foo).unwrap();
        // Multiple `## Notes` — an ambiguous shape that stays a manual
        // conflict (the unambiguous both-exist case now auto-merges).
        fs::write(
            foo.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\n\nfirst\n\n## Notes\n\nsecond\n",
        )
        .unwrap();
        let old = tmp.path().join("issues/closed/3-old");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# Old\n",
        )
        .unwrap();

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(
            outcome.stop_phase,
            StopPhase::Ok,
            "notes/comments conflict must not bail the pipeline: {outcome:?}"
        );
        assert!(
            outcome.blockers.is_empty(),
            "no blockers: {:?}",
            outcome.blockers
        );
        assert_eq!(outcome.flat_layout_migrated.len(), 2);
        assert!(
            outcome
                .notes_conflicts_at_apply
                .iter()
                .any(|s| s == "foo-bar"),
            "conflict must surface in notes_conflicts_at_apply, got {:?}",
            outcome.notes_conflicts_at_apply
        );
        // NN-rename of `3-old` MUST run despite the unrelated
        // notes conflict (the whole point of this fix).
        assert_eq!(
            outcome.legacy_dirs_migrated.len(),
            1,
            "NN-rename must proceed despite an unrelated notes conflict"
        );
        // Post-fix scan still surfaces the conflict so the user
        // sees it as forward work (drives exit 1 via critical_blockers).
        let after = scan(tmp.path()).unwrap();
        assert!(
            after.notes_conflicts.iter().any(|s| s == "foo-bar"),
            "post-fix scan must still report the conflict"
        );
        let decision = classify_exit(&after, Some(&outcome), true);
        assert_eq!(decision.code, 1);
        assert_eq!(decision.error_code, "doctor-partial");
    }

    #[test]
    fn apply_renames_notes_for_pre_migration_legacy_folder_in_one_pass() {
        // Regression: `## Notes` in a body still under
        // `issues/open/<slug>/` is invisible to the pre-migration
        // scan (`populate_notes_migration` walks only flat-folder
        // dirs). The phase-3 rename in `apply` therefore did
        // nothing for this issue, and the user had to invoke
        // `doctor --fix` a second time. After the post-migration
        // re-scan now feeds `rename_notes_to_comments`, a single
        // `--fix` invocation must lift the dir AND rename the
        // heading.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/legacy-notes-slug");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\n\nhello\n",
        )
        .unwrap();

        let mut r = scan(tmp.path()).unwrap();
        assert!(
            r.notes_to_rename.is_empty(),
            "pre-migration scan must not see the legacy-folder Notes heading, got {:?}",
            r.notes_to_rename
        );
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        let migrated = tmp.path().join("issues/legacy-notes-slug/item.md");
        assert!(migrated.is_file(), "flat-layout migration must run");
        let after = fs::read_to_string(&migrated).unwrap();
        assert!(
            after.contains("## Comments"),
            "## Notes must be renamed in the same --fix pass, got: {after}"
        );
        assert!(
            !after.contains("## Notes\n"),
            "## Notes heading must be gone, got: {after}"
        );
        assert_eq!(
            outcome.notes_renamed,
            vec!["legacy-notes-slug".to_string()],
            "outcome must record the post-migration rename"
        );
    }

    #[test]
    fn apply_renames_notes_for_numbered_legacy_folder_in_one_pass() {
        // Companion to the slug-named regression above: verify the
        // post-migration rename also fires for a numbered-legacy
        // dir that lived under `issues/open/` and goes through
        // BOTH flat-layout migration AND NN-rename in the same
        // `--fix` pass. The body must end up at the canonical slug
        // path with `## Comments`. We intentionally do not assert
        // on `outcome.notes_renamed` slug identity here — that's a
        // pre-existing reporting skew tracked separately.
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/3-foo-bar");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\ncreated: 2026-01-01\n---\n# T\n\n## Notes\n\nhello\n",
        )
        .unwrap();

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(
            outcome.legacy_dirs_migrated.len(),
            1,
            "NN-rename must run on the lifted numbered-legacy dir"
        );
        let new_slug = &outcome.legacy_dirs_migrated[0].new_slug;
        let final_item = tmp.path().join("issues").join(new_slug).join("item.md");
        assert!(
            final_item.is_file(),
            "expected file at canonical slug path, got missing: {}",
            final_item.display()
        );
        let after = fs::read_to_string(&final_item).unwrap();
        assert!(
            after.contains("## Comments"),
            "## Notes must be renamed at the canonical slug location, got: {after}"
        );
        assert!(
            !after.contains("## Notes\n"),
            "## Notes heading must be gone, got: {after}"
        );
    }

    #[test]
    fn apply_preserves_partial_flat_layout_migration_on_mid_loop_failure() {
        // Phase-5 mid-loop failure must surface as `Ok(outcome)` with
        // `flat_layout_migrated` carrying the move(s) that landed and
        // `apply_error` carrying the failure cause — NOT propagate as
        // `Err` (which would strand the partial progress inside an
        // anyhow text blob and bypass `--json` consumers).
        let tmp = fresh_repo();
        // Two flat-eligible issues. Slugs sorted alphabetically by the
        // BTreeMap inside `plan_migrate_layout` → `aaa-foo` is move #1,
        // `zzz-bar` is move #2.
        for slug in ["aaa-foo", "zzz-bar"] {
            let dir = tmp.path().join("issues/open").join(slug);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }

        let mut r = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut r);

        // Sabotage move #2 *after* the planner has classified the
        // moves: planting a regular file at the destination of the
        // second rename. `fs::rename(<dir>, <regular_file>)` returns
        // ENOTDIR on Unix / equivalent on Windows. Pre-creating before
        // planning would have been caught by `plan_migrate_layout`'s
        // `symlink_metadata` conflict check; doing it after lets the
        // failure surface inside `execute_migrate_layout_plan`.
        fs::write(tmp.path().join("issues/zzz-bar"), "blocker\n").unwrap();

        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .expect("apply must return Ok with partial outcome, not Err");

        assert_eq!(
            outcome.flat_layout_migrated.len(),
            1,
            "first move should have landed before the failure, got {:?}",
            outcome.flat_layout_migrated
        );
        assert_eq!(outcome.flat_layout_migrated[0].slug, "aaa-foo");
        assert!(
            tmp.path().join("issues/aaa-foo/item.md").is_file(),
            "first move should be visible on disk"
        );
        let err_msg = outcome
            .apply_error
            .as_ref()
            .expect("apply_error must carry the failure cause");
        assert!(
            err_msg.contains("zzz-bar") || err_msg.contains("rename"),
            "apply_error should mention the failed rename, got {err_msg:?}"
        );

        // The structured `--json --fix` envelope must echo both pieces
        // so scripts can recover without parsing stderr.
        let json = render_json(&scan(tmp.path()).unwrap(), Some(&outcome), true, tmp.path());
        let envelope = json
            .get("apply_outcome")
            .expect("apply_outcome present on --fix runs");
        assert_eq!(
            envelope
                .get("flat_layout_migrated")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1),
        );
        assert!(envelope
            .get("apply_error")
            .map(|v| !v.is_null())
            .unwrap_or(false));
    }

    #[cfg(unix)]
    #[test]
    fn detects_symlinked_issue_dir() {
        // Symlink target need not exist meaningfully; we just check
        // that doctor surfaces the symlink.
        let tmp = fresh_repo();
        let target = tmp.path().join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("item.md"), "---\n---\n# x\n").unwrap();
        let link = tmp.path().join("issues/quiet-brave-otter");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .symlinked_dirs
            .iter()
            .any(|s| s.contains("quiet-brave-otter")));
    }

    #[cfg(unix)]
    #[test]
    fn detects_broken_symlinked_issue_dir() {
        let tmp = fresh_repo();
        let link = tmp.path().join("issues/quiet-brave-otter");
        std::os::unix::fs::symlink("/nonexistent/target/path", &link).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r
            .symlinked_dirs
            .iter()
            .any(|s| s.contains("quiet-brave-otter")));
    }

    #[test]
    fn schema_validation_honours_user_edited_required_field() {
        let tmp = fresh_repo();
        // Custom schema requires a `team` field.
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields:\n  team:\n    required: true\n",
        )
        .unwrap();
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: open\npriority: normal\n---\n# T\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.schema_violations
                .iter()
                .any(|(_, msg)| msg.contains("team")),
            "expected `team` violation, got {:?}",
            r.schema_violations
        );
    }

    #[test]
    fn scan_surfaces_transition_warnings_and_missing_sections() {
        let tmp = fresh_repo();
        fs::create_dir_all(tmp.path().join(".issuectl")).unwrap();
        fs::write(
            tmp.path().join(".issuectl/transitions.yaml"),
            "version: 1\nstatus_rules:\n  done:\n    requires_assignee: true\n",
        )
        .unwrap();
        // Body-section requirements moved to schema (C6).
        fs::write(
            tmp.path().join("issues/.schema.yaml"),
            "version: 1\nfields: {}\nbody_sections:\n  bug: [Steps to Reproduce, Expected, Actual]\n",
        )
        .unwrap();
        // Issue is `done` without an assignee → transition warning.
        // Bug is missing the required body sections → body section warning.
        let dir = tmp.path().join("issues/legacy-bug");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\ncreated: 2026-01-01\nstatus: done\npriority: normal\n---\n\n# T\n\n## Description\n\nx\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.transition_warnings
                .iter()
                .any(|(s, m)| s == "legacy-bug" && m.contains("assignee")),
            "expected assignee warning, got {:?}",
            r.transition_warnings
        );
        let missing: Vec<_> = r
            .missing_body_sections
            .iter()
            .filter(|(s, _)| s == "legacy-bug")
            .map(|(_, sec)| sec.clone())
            .collect();
        assert!(missing.contains(&"Steps to Reproduce".to_string()));
        assert!(missing.contains(&"Expected".to_string()));
        assert!(missing.contains(&"Actual".to_string()));
    }

    #[test]
    fn agents_md_drift_not_flagged_when_file_absent() {
        let tmp = fresh_repo();
        let r = scan(tmp.path()).unwrap();
        assert!(!r.agents_md_drift);
    }

    #[test]
    fn agents_md_drift_detected_after_schema_change() {
        let tmp = fresh_repo();
        // Write a fresh AGENTS.md against the default schema.
        agents::run_init(tmp.path(), false, false).unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(!r.agents_md_drift, "freshly-written file is in sync");

        // Mutate the schema so the rendered block no longer matches.
        let schema_path = tmp.path().join("issues/.schema.yaml");
        fs::write(
            &schema_path,
            "version: 1\nbody_sections:\n  bug: [Reproduction]\n",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.agents_md_drift, "drift after schema edit");
    }

    #[test]
    fn agents_md_fix_regenerates_block_preserving_prose() {
        let tmp = fresh_repo();
        let path = tmp.path().join(agents::AGENTS_RELATIVE_PATH);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Hand-written file with a stale managed block + custom prose.
        let custom = format!(
            "# My custom heading\n\nMy hand-written notes.\n\n{}\n\nstale body\n{}\n\nClosing prose.\n",
            agents::MANAGED_START,
            agents::MANAGED_END
        );
        fs::write(&path, &custom).unwrap();

        let mut report = scan(tmp.path()).unwrap();
        assert!(report.agents_md_drift);
        let actions = DoctorActions::from_findings(&mut report);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.agents_md_regenerated);

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("# My custom heading\n\nMy hand-written notes.\n\n"));
        assert!(after.contains("Closing prose.\n"));
        assert!(!after.contains("stale body"));
        assert!(after.contains(agents::MANAGED_START));
        assert!(after.contains(agents::MANAGED_END));
    }

    #[test]
    fn legacy_issues_agents_md_is_detected_and_rewritten() {
        let tmp = fresh_repo();
        let issues_dir = tmp.path().join("issues");
        fs::create_dir_all(&issues_dir).unwrap();
        let path = issues_dir.join("AGENTS.md");
        // Pre-v0.5.0 scaffold marker.
        fs::write(
            &path,
            "# Issues\n\n## Issue Numbering\n\nIssue numbers are sequential...\n",
        )
        .unwrap();

        let mut report = scan(tmp.path()).unwrap();
        assert!(report.legacy_issues_agents_md);

        let actions = DoctorActions::from_findings(&mut report);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.issues_agents_md_rewritten);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            crate::skill::ISSUES_AGENTS_TEMPLATE
        );
    }

    #[test]
    fn customized_issues_agents_md_is_left_alone() {
        let tmp = fresh_repo();
        let issues_dir = tmp.path().join("issues");
        fs::create_dir_all(&issues_dir).unwrap();
        let path = issues_dir.join("AGENTS.md");
        let custom = "# Our team's policy\n\nWe write our own rules here.\n";
        fs::write(&path, custom).unwrap();

        let report = scan(tmp.path()).unwrap();
        assert!(!report.legacy_issues_agents_md);
        assert_eq!(fs::read_to_string(&path).unwrap(), custom);
    }

    /// Single-pass `scan_issues` powers every check. This fixture wires
    /// up many independent findings in one repo and asserts the merged
    /// `DoctorFindings` looks the same as the multi-walk produced — a
    /// regression guard for the D7 refactor.
    #[test]
    fn single_pass_scan_surfaces_all_categories() {
        let tmp = fresh_repo();
        // Legacy <NN>-<slug> dir under issues/open/.
        put_legacy(
            &tmp,
            "open",
            7,
            "old-style",
            "---\nnumber: 7\nstatus: open\n---\n# E7. Old\n",
        );
        // Flat-layout issue with: broken epic ref + future timestamp +
        // unknown frontmatter key + ## Notes that needs renaming.
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\npriority: normal\n\
             epic: nonexistent-ghost-fox\ncreated: 2999-01-01\n\
             whimsy: 1\n---\n# T\n\n## Notes\n\nold note\n",
        )
        .unwrap();
        // Symlink + orphan tempfile.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                tmp.path().join("issues/quiet-brave-otter"),
                tmp.path().join("issues/symlinked-thing"),
            )
            .unwrap();
        }
        fs::write(
            tmp.path()
                .join("issues/quiet-brave-otter/.issuectl-tmp-XYZ"),
            "leftover",
        )
        .unwrap();

        let r = scan(tmp.path()).unwrap();

        assert_eq!(r.legacy_dirs.len(), 1, "legacy dir detected");
        assert!(
            r.broken_refs
                .iter()
                .any(|(_, k, t)| k == "epic" && t == "nonexistent-ghost-fox"),
            "broken_refs={:?}",
            r.broken_refs
        );
        assert!(
            r.timestamp_issues.iter().any(|(_, m)| m.contains("future")),
            "timestamp_issues={:?}",
            r.timestamp_issues
        );
        assert!(
            r.unknown_keys.iter().any(|(_, k)| k == "whimsy"),
            "unknown_keys={:?}",
            r.unknown_keys
        );
        assert!(
            r.notes_to_rename.iter().any(|s| s == "quiet-brave-otter"),
            "notes_to_rename={:?}",
            r.notes_to_rename
        );
        assert!(
            r.orphan_tempfiles
                .iter()
                .any(|p| p.to_string_lossy().contains(".issuectl-tmp-XYZ")),
            "orphan_tempfiles={:?}",
            r.orphan_tempfiles
        );
        #[cfg(unix)]
        assert!(
            r.symlinked_dirs
                .iter()
                .any(|s| s.contains("symlinked-thing")),
            "symlinked_dirs={:?}",
            r.symlinked_dirs
        );
        // Schema violations should ignore the legacy dir.
        assert!(
            !r.schema_violations
                .iter()
                .any(|(loc, _)| loc.contains("7-old-style")),
            "schema_violations should skip legacy dirs: {:?}",
            r.schema_violations
        );
    }

    /// Golden-snapshot test for the `render_json` output. Intentionally
    /// avoids any non-deterministic input (no legacy `<NN>-<slug>` dirs
    /// — those go through `slug::generate_unique`, no symlinks — paths
    /// differ across platforms). Verifies the byte shape downstream
    /// JSON consumers depend on.
    #[test]
    fn render_json_matches_golden_snapshot() {
        let tmp = fresh_repo();
        // Issue with: broken epic ref + future timestamp + unknown key.
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\n\
             epic: nonexistent-ghost-fox\ncreated: 2999-01-01\n\
             whimsy: 1\n---\n# A\n",
        );
        // Issue with closing status but no `closed:` (status consistency).
        put_flat(
            &tmp,
            "beta-quiet-otter",
            "---\ntype: bug\nstatus: done\npriority: normal\n---\n# B\n",
        );
        // Empty dir → missing_item_md.
        fs::create_dir_all(tmp.path().join("issues/charlie-empty-dir")).unwrap();

        let report = scan(tmp.path()).unwrap();
        let json = render_json(&report, None, false, tmp.path());
        let actual = serde_json::to_string_pretty(&json).unwrap();
        // Normalise the tempdir prefix so the snapshot is portable.
        let actual = actual.replace(tmp.path().to_str().unwrap(), "<TMP>");

        let expected = r#"{
  "agents_md_check_skipped": null,
  "agents_md_drift": false,
  "agents_md_malformed": null,
  "agents_md_missing": true,
  "agents_md_regenerated": false,
  "alias_coercions": [],
  "blocked_by_cycles": [],
  "blocked_by_self": [],
  "both_open_and_closed": [],
  "broken_attachment_refs": [],
  "broken_refs": [
    {
      "kind": "epic",
      "slug": "alpha-bright-cat",
      "target": "nonexistent-ghost-fox"
    }
  ],
  "closed_with_active_status": [],
  "conflict_markers": [],
  "deferred_labels": [],
  "deferred_labels_removed": [],
  "deferred_labels_require_intake_migrate": [],
  "duplicate_slugs": [],
  "files_rewritten": 0,
  "fix_applied": false,
  "flat_layout_conflicts": [],
  "flat_layout_migrated": [],
  "flat_layout_planned": [],
  "gitignored_paths": [],
  "invalid_slugs": [],
  "issues_agents_md_rewritten": false,
  "large_binaries": [],
  "legacy_issues_agents_md": false,
  "migrations": [],
  "missing_body_sections": [],
  "missing_item_md": [
    "flat/charlie-empty-dir"
  ],
  "non_avif_images": [],
  "notes_conflicts": [],
  "notes_renamed": [],
  "notes_to_rename": [],
  "open_with_closing_status": [],
  "orphan_epic_refs": [
    {
      "epic": "nonexistent-ghost-fox",
      "slug": "alpha-bright-cat"
    }
  ],
  "orphan_tempfiles": [],
  "orphan_tempfiles_removed": [],
  "parse_errors": [],
  "schema_missing": true,
  "schema_parse_error": null,
  "schema_violations": [],
  "status_consistency": [
    {
      "message": "closing status \"done\" requires `closed:` date",
      "slug": "beta-quiet-otter"
    }
  ],
  "status_reconciled": [],
  "symlinked_dirs": [],
  "timestamp_issues": [
    {
      "message": "created date 2999-01-01 is in the future",
      "slug": "alpha-bright-cat"
    }
  ],
  "transition_warnings": [],
  "unknown_keys": [
    {
      "key": "whimsy",
      "slug": "alpha-bright-cat"
    }
  ],
  "unknown_reviewers": []
}"#;
        assert_eq!(
            actual, expected,
            "render_json output drifted from the golden snapshot.\n\
             If the change is intentional, update the snapshot."
        );
    }

    /// Bug #1 (`apply()` returns `Result<()>` and `bail!`s — `--json
    /// --fix` when preflight blocks → no JSON, anyhow text on stderr):
    /// the new `apply` returns `Ok(outcome)` with `outcome.blockers`
    /// populated instead of `Err`, and the JSON envelope carries the
    /// blockers under `apply_outcome` so scripted callers can read a
    /// structured response.
    #[test]
    fn json_fix_with_preflight_block_emits_structured_outcome() {
        let tmp = fresh_repo();
        // Slug present in BOTH legacy folders → preflight blocker.
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!outcome.blockers.is_empty());
        assert_eq!(outcome.stop_phase, StopPhase::Preflight);
        // `fix_applied: true` here reflects the unconditional schema
        // bootstrap that fires before preflight refusal (issue:
        // @unreasonably-attractive-star). No other phase ran — the
        // BOTH-folders blocker still gates everything else.
        assert!(outcome.schema_bootstrapped);
        assert!(outcome.legacy_dirs_migrated.is_empty());
        assert!(outcome.flat_layout_migrated.is_empty());

        let json = render_json(&findings, Some(&outcome), true, tmp.path());
        let ao = json
            .get("apply_outcome")
            .expect("apply_outcome must be present on --fix");
        assert_eq!(ao["fix_applied"], serde_json::Value::Bool(true));
        assert_eq!(
            ao["stop_phase"],
            serde_json::Value::String("preflight".into())
        );
        let blockers = ao["blockers"].as_array().unwrap();
        assert!(!blockers.is_empty(), "blockers must surface in JSON");
        assert!(
            blockers
                .iter()
                .any(|v| v.as_str().unwrap_or("").contains("BOTH")),
            "expected `BOTH issues/open/...` blocker, got {blockers:?}"
        );
    }

    /// Clean-success path with writes: when `apply` runs to completion
    /// with no blockers AND at least one write happened (fresh repo
    /// triggers schema bootstrap), `stop_phase: "ok"` MUST coexist
    /// with `fix_applied: true`. JSON consumers should not have to
    /// infer the success case from `blockers.is_empty()`.
    #[test]
    fn clean_success_with_writes_envelope_carries_ok_and_fix_applied_true() {
        let tmp = fresh_repo();
        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.blockers.is_empty(), "clean repo: no blockers");
        assert_eq!(outcome.stop_phase, StopPhase::Ok);
        assert!(
            outcome.schema_bootstrapped,
            "fresh repo: schema bootstrap landed"
        );
        assert!(
            outcome.fix_applied(),
            "schema_bootstrapped flips fix_applied"
        );

        let scan_after = scan(tmp.path()).unwrap();
        let json = render_json(&scan_after, Some(&outcome), true, tmp.path());
        assert_eq!(
            json["apply_outcome"]["stop_phase"],
            serde_json::Value::String("ok".into())
        );
        assert_eq!(
            json["apply_outcome"]["fix_applied"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            json["apply_outcome"]["schema_bootstrapped"],
            serde_json::Value::Bool(true)
        );
        let blockers = json["apply_outcome"]["blockers"].as_array().unwrap();
        assert!(blockers.is_empty(), "no blockers on clean success");
    }

    /// Clean-success path with NO writes: schema already bootstrapped
    /// from a prior run, no findings ⇒ `apply` is a no-op.
    /// `stop_phase: "ok"` MUST coexist with `fix_applied: false`.
    /// This pins the second `(ok, fix_applied)` combination — the
    /// matrix is undertested without it.
    #[test]
    fn clean_success_no_writes_envelope_carries_ok_and_fix_applied_false() {
        let tmp = fresh_repo();
        // Pre-bootstrap the schema so the second `apply` writes nothing.
        schema::ensure_default_written(tmp.path()).unwrap();

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(outcome.blockers.is_empty());
        assert_eq!(outcome.stop_phase, StopPhase::Ok);
        assert!(
            !outcome.fix_applied(),
            "no-op apply must not flip fix_applied"
        );

        let scan_after = scan(tmp.path()).unwrap();
        let json = render_json(&scan_after, Some(&outcome), true, tmp.path());
        assert_eq!(
            json["apply_outcome"]["stop_phase"],
            serde_json::Value::String("ok".into())
        );
        assert_eq!(
            json["apply_outcome"]["fix_applied"],
            serde_json::Value::Bool(false)
        );
    }

    /// Bug #4 (manual splice list): the post-apply rendering pulls
    /// from `ApplyOutcome` directly. Adding a new applied-action
    /// variant means extending `ApplyOutcome` + `DoctorActions::
    /// fix_applied` — no field-by-field copy in `run`.
    #[test]
    fn fix_applied_predicate_is_centralised_on_outcome() {
        let mut o = ApplyOutcome::default();
        assert!(!o.fix_applied(), "default outcome reports false");
        o.schema_bootstrapped = true;
        assert!(
            o.fix_applied(),
            "schema_bootstrapped alone must flip fix_applied (bug #3)"
        );
        let mut o = ApplyOutcome::default();
        o.notes_renamed.push("foo".into());
        assert!(o.fix_applied(), "notes_renamed must flip fix_applied");
    }

    /// Bug #5 (preflight ↔ has_critical_findings drift): the two
    /// call sites share a single `blockers_for` core. The alignment
    /// is now intentional-and-narrower: preflight uses the layout-
    /// fatal subset (`apply_blockers`), which is a strict subset of
    /// the exit-code set (`critical_blockers`). Layout-fatal
    /// findings (here: conflict markers) appear in BOTH lists —
    /// preflight refuses on them. Schema-shape findings appear in
    /// `critical_blockers` only — they drive exit-1 but do NOT
    /// refuse `--fix` (issue: @staggeringly-important-zoo). The
    /// drift bug is still gone because both lists derive from one
    /// function with one set of decisions.
    #[test]
    fn critical_blockers_aligns_preflight_with_exit_code() {
        let tmp = fresh_repo();
        // Conflict markers: layout-fatal AND exit-1. The two sets
        // agree on this class.
        put_flat(
            &tmp,
            "quiet-brave-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> branch\n",
        );
        let findings = scan(tmp.path()).unwrap();
        let crit = critical_blockers(&findings);
        let pre = apply_blockers(&findings);
        assert!(!crit.is_empty(), "conflict markers should be a blocker");
        assert_eq!(crit, pre, "layout-fatal blockers appear in both views");

        let mut findings_for_apply = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings_for_apply);
        assert_eq!(
            pre, actions.preflight_blockers,
            "preflight_blockers must equal apply_blockers output"
        );
    }

    /// Issue @staggeringly-important-zoo: schema violations no
    /// longer block `--fix`. The flat-layout migration is the
    /// safest, most mechanical operation in the toolbox; gating it
    /// on schema cleanliness inverted the priority. After this
    /// change a repo with concurrent layout AND schema violations
    /// migrates the layout in one pass and reports the remaining
    /// schema violations on the post-migration state.
    #[test]
    fn schema_violations_do_not_block_layout_migration() {
        let tmp = fresh_repo();
        // Issue at legacy `issues/open/<slug>/` with a body missing
        // the schema-required `priority` field. Pre-fix scan must
        // see both: a layout migration AND a schema violation. The
        // schema violation is in `critical_blockers` (exit-1) but
        // NOT in `apply_blockers` (layout-fatal), so `--fix` should
        // run the layout migration anyway.
        let dir = tmp.path().join("issues/open/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\n---\n# T\n",
        )
        .unwrap();

        let mut findings = scan(tmp.path()).unwrap();
        assert!(
            !findings.schema_violations.is_empty(),
            "expected schema violation pre-fix"
        );
        assert!(
            !critical_blockers(&findings).is_empty(),
            "schema violation must remain in exit-1 set"
        );
        assert!(
            apply_blockers(&findings).is_empty(),
            "schema violation must NOT be in apply-preflight set"
        );

        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        // Layout migration ran despite the schema violation.
        assert!(
            !outcome.flat_layout_migrated.is_empty(),
            "flat-layout migration must run when only schema violations remain"
        );
        assert!(tmp
            .path()
            .join("issues/quiet-brave-otter/item.md")
            .is_file());
        assert_eq!(outcome.stop_phase, StopPhase::Ok);

        // Post-migration scan still surfaces the unresolved schema
        // violation against the post-migration path — exit-1 still
        // fires, but as forward work rather than a blocker.
        let after = scan(tmp.path()).unwrap();
        assert!(
            !after.schema_violations.is_empty(),
            "schema violation persists post-fix and surfaces against post-migration path"
        );
        assert!(!critical_blockers(&after).is_empty());
    }

    /// Issue @ridiculously-outrageous-fold: long warning lists
    /// collapse to a one-liner when not `--verbose`. The downstream project
    /// migration printed 240 layout-migration entries every
    /// iteration of "fix-something-rerun-doctor" loops; this
    /// verifies the actual rendered text on both sides of the
    /// threshold and the verbose escape hatch. Asserting against
    /// the rendered string (rather than just pinning the constant)
    /// catches regressions in wording, formatting, or wiring of the
    /// `verbose` flag through `print_section`'s callers.
    #[test]
    fn render_section_collapses_long_lists_unless_verbose() {
        let exactly_at_limit: Vec<i32> = (0..RENDER_FULL_LIST_LIMIT as i32).collect();
        let one_over: Vec<i32> = (0..(RENDER_FULL_LIST_LIMIT + 1) as i32).collect();

        // Empty: nothing rendered, no leading newline.
        let mut buf = String::new();
        render_section(
            &mut buf,
            "Title:",
            &Vec::<i32>::new(),
            false,
            "thing(s)",
            |i| i.to_string(),
        );
        assert!(
            buf.is_empty(),
            "empty list must render nothing, got {buf:?}"
        );

        // Exactly LIMIT entries: full list, not collapsed.
        let mut buf = String::new();
        render_section(
            &mut buf,
            "Title:",
            &exactly_at_limit,
            false,
            "thing(s)",
            |i| i.to_string(),
        );
        assert!(buf.contains("Title:"), "expected title, got {buf:?}");
        for i in &exactly_at_limit {
            assert!(buf.contains(&i.to_string()), "missing entry {i}: {buf:?}");
        }
        assert!(
            !buf.contains("re-run with --verbose"),
            "must not collapse at exactly LIMIT entries: {buf:?}"
        );

        // LIMIT+1 entries, non-verbose: collapsed to a one-liner
        // with the count and the verb phrase.
        let mut buf = String::new();
        render_section(&mut buf, "Title:", &one_over, false, "thing(s)", |i| {
            i.to_string()
        });
        assert!(
            buf.contains(&format!("{} thing(s)", one_over.len())),
            "expected collapsed count line, got {buf:?}"
        );
        assert!(
            buf.contains("re-run with --verbose to list"),
            "expected verbose hint, got {buf:?}"
        );
        assert!(
            !buf.contains("Title:"),
            "collapsed render must omit the title, got {buf:?}"
        );

        // LIMIT+1 entries, verbose: full list, no collapse hint.
        let mut buf = String::new();
        render_section(&mut buf, "Title:", &one_over, true, "thing(s)", |i| {
            i.to_string()
        });
        assert!(buf.contains("Title:"), "verbose must print title: {buf:?}");
        for i in &one_over {
            assert!(buf.contains(&i.to_string()), "verbose missing {i}: {buf:?}");
        }
        assert!(
            !buf.contains("re-run with --verbose"),
            "verbose must not show the collapse hint: {buf:?}"
        );
    }

    /// Regression for @intake-bug-issuectl-06c42e2d1123: the summary,
    /// JSON error message, and JSON finding lists must agree on the number
    /// of unresolved entries. `critical_blockers` intentionally groups the
    /// eight status violations into one diagnostic, so it is not a count.
    #[test]
    fn fix_remaining_summary_counts_every_listed_finding() {
        let mut findings = DoctorFindings::default();
        findings.status_consistency = (0..8)
            .map(|n| {
                (
                    format!("issue-{n}"),
                    "closing status needs closed date".to_string(),
                )
            })
            .collect();
        findings
            .unknown_keys
            .push(("issue-extra".to_string(), "deliverable".to_string()));
        let outcome = ApplyOutcome::default();

        let json = render_json(&findings, Some(&outcome), true, Path::new("/repo"));
        let listed = json["status_consistency"].as_array().unwrap().len()
            + json["unknown_keys"].as_array().unwrap().len();
        assert_eq!(listed, 9, "fixture must match the reported incident");
        assert_eq!(remaining_finding_count(&findings), listed);

        let summary = fix_summary(&findings, &outcome);
        assert!(
            summary.contains("9 unfixable finding(s) remain"),
            "summary must count the entries it rendered: {summary}"
        );
        let decision = classify_exit(&findings, Some(&outcome), true);
        assert!(
            decision.message.contains("9 unfixable finding(s) remain"),
            "JSON error envelope message must match the human summary: {}",
            decision.message
        );
    }

    #[test]
    fn json_exposes_self_dependencies_counted_as_remaining_findings() {
        let findings = DoctorFindings {
            blocked_by_self: vec!["issue-self".to_string()],
            ..DoctorFindings::default()
        };
        let json = render_json(
            &findings,
            Some(&ApplyOutcome::default()),
            true,
            Path::new("/repo"),
        );
        assert_eq!(
            json["blocked_by_self"],
            serde_json::json!(["issue-self"]),
            "JSON must expose every finding that the summary counts"
        );
    }

    /// `apply_blockers` must always be a SUBSET of
    /// `critical_blockers`. The two functions share a single
    /// `blockers_for(scope)` core, but the manual `!layout_only`
    /// guards on each schema-shape branch make it possible for a
    /// future change to accidentally classify a finding as
    /// preflight-only (would refuse `--fix` for something that
    /// doesn't drive exit-1) or omit it from both. Pinning the
    /// subset relation with a fixture that produces every
    /// schema-shape finding catches the most likely regression
    /// shape: a new finding category added to one branch of
    /// `blockers_for` and forgotten in the other.
    #[test]
    fn apply_blockers_is_a_subset_of_critical_blockers() {
        let tmp = fresh_repo();
        // Issue with multiple schema-shape problems: missing
        // required `priority`, broken `epic` cross-reference (a
        // valid slug shape but no such issue), and timestamps that
        // disagree with status (`closed:` set while `status: open`).
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("item.md"),
            "---\ntype: bug\nstatus: open\nepic: alpha-bright-cat\nclosed: 2026-01-02\ncreated: 2026-01-01\nupdated: 2026-01-01\n---\n# T\n",
        )
        .unwrap();
        let findings = scan(tmp.path()).unwrap();
        // Sanity: at least one schema-shape finding fired.
        assert!(
            !findings.schema_violations.is_empty()
                || !findings.broken_refs.is_empty()
                || !findings.status_consistency.is_empty(),
            "fixture must produce a schema-shape finding"
        );

        let crit: BTreeSet<String> = critical_blockers(&findings).into_iter().collect();
        let pre: BTreeSet<String> = apply_blockers(&findings).into_iter().collect();
        assert!(
            pre.is_subset(&crit),
            "apply_blockers must be a subset of critical_blockers.\n  pre: {pre:?}\n  crit: {crit:?}"
        );
        // The schema-shape findings must be in `critical_blockers`
        // (drive exit-1) but NOT in `apply_blockers` (don't refuse
        // `--fix`). At least one ExitCode-only finding must exist
        // in this fixture.
        assert!(
            crit.len() > pre.len(),
            "fixture must exercise an ExitCode-only finding (crit > pre): crit={crit:?} pre={pre:?}"
        );
    }

    /// `--fix` must run the legacy NN-rename phase against a
    /// post-flat-layout fresh scan even when the fresh scan
    /// surfaces schema-shape findings (schema violations, broken
    /// refs, status inconsistencies, timestamp issues). Before
    /// this bundle, the post-apply re-check used
    /// `critical_blockers` and would bail with `StopPhase::PostApply`
    /// the moment any of those appeared on the post-migration
    /// state. Now it uses `apply_blockers`, so NN-rename proceeds
    /// — schema findings remain visible as forward work in the
    /// final scan and drive exit-1, but they don't strand the
    /// migration.
    #[test]
    fn nn_rename_runs_when_post_migration_scan_has_schema_findings() {
        let tmp = fresh_repo();
        // Numbered-legacy issue under `issues/closed/`. Body has no
        // schema-required `priority`, so the post-migration rescan
        // (after both flat-layout migration AND the NN-rename's
        // `rewrite_item_frontmatter`) will surface a schema
        // violation against the new canonical slug — proving the
        // path completed end-to-end despite the schema-shape
        // finding.
        let old = tmp.path().join("issues/closed/3-old");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("item.md"),
            "---\nnumber: 3\ntype: bug\nstatus: open\n---\n# Old\n",
        )
        .unwrap();

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        // Layout migration ran AND NN-rename ran (post-migration
        // schema findings did not bail PostApply).
        assert_eq!(outcome.stop_phase, StopPhase::Ok);
        assert_eq!(outcome.flat_layout_migrated.len(), 1);
        assert_eq!(
            outcome.legacy_dirs_migrated.len(),
            1,
            "NN-rename must run despite post-migration schema findings: {outcome:?}"
        );

        // Post-fix scan still surfaces the unresolved schema
        // violation against the new canonical slug — exit-1 still
        // fires (caller asserts), but as forward work, not a
        // pipeline bail.
        let after = scan(tmp.path()).unwrap();
        assert!(
            !after.schema_violations.is_empty(),
            "expected lingering schema violation against post-rename path"
        );
    }

    /// Issue @unreasonably-attractive-star: schema bootstrap fires
    /// unconditionally on `--fix`, even when other preflight
    /// blockers are present. Prior behavior advertised
    /// auto-creation in the read-only output but failed to deliver
    /// because preflight bailed before bootstrap.
    #[test]
    fn schema_bootstrap_runs_even_when_preflight_blocks() {
        let tmp = fresh_repo();
        // Slug present in both legacy folders → preflight blocker.
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }
        // Sanity: `.schema.yaml` does not exist yet.
        assert!(!tmp.path().join("issues/.schema.yaml").exists());

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();

        assert_eq!(outcome.stop_phase, StopPhase::Preflight);
        assert!(!outcome.blockers.is_empty());
        // The promise: bootstrap landed despite preflight refusal.
        assert!(
            outcome.schema_bootstrapped,
            "schema bootstrap must precede preflight refusal"
        );
        assert!(
            tmp.path().join("issues/.schema.yaml").is_file(),
            "schema file must be on disk after preflight-blocked --fix"
        );
    }

    /// Round-2 regression #1 (state destruction on preflight-blocked
    /// path): `DoctorActions::from_findings` drains the to-do data via
    /// `mem::take`. Before the fix, `run` only re-scanned when
    /// `outcome.fix_applied()` was true — so a preflight-blocked run
    /// rendered an empty findings object and the user saw the blocker
    /// message but none of the pending lists. The fix unconditionally
    /// re-scans after apply.
    #[test]
    fn preflight_blocked_render_path_does_not_lose_pending_work() {
        let tmp = fresh_repo();
        // One legitimate legacy migration that scan should surface,
        // plus a slug present in BOTH legacy folders → preflight
        // blocker. After apply returns Ok(outcome) with blockers
        // populated, the rescanned `findings` MUST still contain the
        // legacy migration entry.
        put_legacy(
            &tmp,
            "open",
            7,
            "alpha",
            "---\nnumber: 7\nstatus: open\n---\n# A\n",
        );
        for f in ["open", "closed"] {
            let dir = tmp.path().join("issues").join(f).join("quiet-brave-otter");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("item.md"),
                "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n",
            )
            .unwrap();
        }

        let mut findings = scan(tmp.path()).unwrap();
        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(!outcome.blockers.is_empty(), "expected preflight blocker");
        // Simulate `run`'s post-apply rescan.
        let final_findings = scan(tmp.path()).unwrap();
        assert!(
            !final_findings.legacy_dirs.is_empty(),
            "rescan must surface the legacy migration even when preflight blocked"
        );
        // The JSON envelope on this path must carry both the blockers
        // AND the to-do lists.
        let json = render_json(&final_findings, Some(&outcome), true, tmp.path());
        let migrations = json["migrations"].as_array().unwrap();
        assert!(
            !migrations.is_empty(),
            "migrations field must not be empty on preflight-blocked path"
        );
        let blockers = json["apply_outcome"]["blockers"].as_array().unwrap();
        assert!(!blockers.is_empty());
    }

    /// Round-2 regression #2 (legacy numeric refs in flat-layout
    /// blocking `--fix`): a flat-layout issue with `epic: 7` produces
    /// a `broken_refs` entry of kind "(legacy numeric ref)". Before
    /// the fix, `critical_blockers` treated this as a refusal. The
    /// migration is supposed to heal it via `rewrite_item_frontmatter`.
    #[test]
    fn legacy_numeric_refs_in_flat_layout_do_not_block_fix() {
        let tmp = fresh_repo();
        // Flat-layout issue with a stale numeric epic ref — this is
        // exactly the state a partially-migrated repo will have.
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: 7\n---\n# A\n",
        );
        let findings = scan(tmp.path()).unwrap();
        // Sanity: scan flagged it.
        assert!(
            findings
                .broken_refs
                .iter()
                .any(|(_, k, t)| k == "epic" && t.contains("(legacy numeric ref)")),
            "expected legacy-numeric ref to be flagged: {:?}",
            findings.broken_refs
        );
        // critical_blockers must NOT contain a "broken cross-references"
        // entry that would refuse `--fix`.
        let blockers = critical_blockers(&findings);
        assert!(
            !blockers
                .iter()
                .any(|b| b.contains("broken cross-references")),
            "legacy numeric refs must not block --fix: {:?}",
            blockers
        );
    }

    /// Round-2 regression #3 (notes apply-time conflict silently
    /// dropped): when scan classified a file as `SafeRename` but a
    /// concurrent edit between scan and apply added a `## Comments`
    /// heading, the fix is skipped. The skip MUST be recorded in
    /// `outcome.notes_conflicts_at_apply` so JSON consumers see that
    /// planned work was deferred.
    #[test]
    fn notes_conflict_at_apply_is_recorded_in_outcome() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/legacy-notes-here");
        fs::create_dir_all(&dir).unwrap();
        let item = dir.join("item.md");
        // Initially: SafeRename (one ## Notes, no ## Comments).
        fs::write(
            &item,
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n## Notes\n\nold\n",
        )
        .unwrap();
        let mut findings = scan(tmp.path()).unwrap();
        assert!(findings
            .notes_to_rename
            .iter()
            .any(|s| s == "legacy-notes-here"));

        // Concurrent edit: a user appends a SECOND `## Notes` section
        // before apply runs — an ambiguous shape. `migrate_notes_heading`
        // will now classify this as Conflict at apply time. (Adding a
        // single `## Comments` would instead auto-merge; multiple
        // `## Notes` is the shape that still needs a human.)
        fs::write(
            &item,
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n\n## Notes\n\nold\n\n## Notes\n\nnewer\n",
        )
        .unwrap();

        let actions = DoctorActions::from_findings(&mut findings);
        let outcome = apply(
            tmp.path(),
            actions,
            &crate::mutate::WriteLock::acquire(tmp.path()).unwrap(),
        )
        .unwrap();
        assert!(
            outcome
                .notes_conflicts_at_apply
                .iter()
                .any(|s| s == "legacy-notes-here"),
            "TOCTOU conflict must surface in outcome: {:?}",
            outcome
        );
        assert!(outcome.notes_renamed.is_empty(), "no rename when conflict");

        // The conflict MUST also surface in the JSON envelope, not
        // only on the typed outcome — the field's docstring promises
        // `--json --fix` consumers a signal that some planned work
        // didn't run.
        let scan_after = scan(tmp.path()).unwrap();
        let json = render_json(&scan_after, Some(&outcome), true, tmp.path());
        let conflicts = json["apply_outcome"]["notes_conflicts_at_apply"]
            .as_array()
            .expect("notes_conflicts_at_apply must be in apply_outcome envelope");
        assert!(
            conflicts
                .iter()
                .any(|v| v.as_str() == Some("legacy-notes-here")),
            "JSON envelope must surface notes_conflicts_at_apply, got {conflicts:?}"
        );
    }

    /// Round-2 regression #4 (hard parse errors on legacy dirs): a
    /// legacy issue with unparseable frontmatter MUST surface as a
    /// Hard parse_error so `critical_blockers` refuses `--fix`. The
    /// alternative is mid-apply panic when `write::read_item` hits
    /// the same broken YAML.
    #[test]
    fn hard_parse_errors_on_legacy_dirs_block_fix() {
        let tmp = fresh_repo();
        let dir = tmp.path().join("issues/open/7-broken-legacy");
        fs::create_dir_all(&dir).unwrap();
        // Legacy frontmatter that parses-as-mapping fails.
        fs::write(dir.join("item.md"), "---\nfoo: : :\n---\n# T\n").unwrap();
        let r = scan(tmp.path()).unwrap();
        // Soft warnings on legacy issues are still suppressed (the
        // intentional skip), but Hard errors must surface.
        assert!(
            r.parse_errors
                .iter()
                .any(|e| e.severity == ParseSeverity::Hard),
            "Hard parse error on legacy must be surfaced: {:?}",
            r.parse_errors
        );
        let blockers = critical_blockers(&r);
        assert!(
            blockers
                .iter()
                .any(|b| b.contains("unparseable issue file")),
            "Hard parse error must block --fix: {:?}",
            blockers
        );
    }

    /// Bug #6 (substring matcher): typed `ParseSeverity` set at the
    /// push site means re-wording the parser's message no longer
    /// reclassifies a hard fail as a soft warn. The legacy-numeric
    /// epic-ref warning is emitted Soft, the unparseable frontmatter
    /// is emitted Hard.
    #[test]
    fn parse_error_severity_is_typed_not_substring_matched() {
        let tmp = fresh_repo();
        // Soft: legacy numeric epic ref on a flat-layout issue.
        put_flat(
            &tmp,
            "alpha-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\nepic: 7\n---\n# A\n",
        );
        // Hard: unparseable frontmatter.
        let dir = tmp.path().join("issues/quiet-brave-otter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("item.md"), "---\nfoo: : :\n---\n# T\n").unwrap();

        let r = scan(tmp.path()).unwrap();
        let any_soft = r
            .parse_errors
            .iter()
            .any(|e| e.severity == ParseSeverity::Soft);
        let any_hard = r
            .parse_errors
            .iter()
            .any(|e| e.severity == ParseSeverity::Hard);
        assert!(
            any_soft,
            "legacy numeric ref must be Soft: {:?}",
            r.parse_errors
        );
        assert!(
            any_hard,
            "unparseable YAML must be Hard: {:?}",
            r.parse_errors
        );

        // Soft alone does NOT block; only the Hard entries appear in
        // critical_blockers.
        let blockers = critical_blockers(&r);
        let hard_blocker = blockers
            .iter()
            .find(|b| b.contains("unparseable issue file"));
        assert!(
            hard_blocker.is_some(),
            "hard parse error must produce a blocker, got {:?}",
            blockers
        );
    }

    #[test]
    fn flags_large_binaries_non_avif_and_broken_refs() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "noisy-bright-cat",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![shot](attachments/shot.png)\n\
             [missing](attachments/gone.avif)\n",
        );
        let issue_dir = tmp.path().join("issues/noisy-bright-cat");
        let att = issue_dir.join("attachments");
        fs::create_dir_all(&att).unwrap();
        // Non-AVIF image AND > 1 MiB → flagged by both checks.
        fs::write(att.join("shot.png"), vec![0u8; (1 << 20) + 10]).unwrap();
        // Small AVIF fixture that the body does NOT reference: clean.
        fs::create_dir_all(issue_dir.join("fixtures")).unwrap();
        fs::write(issue_dir.join("fixtures/ok.bin"), b"tiny").unwrap();

        let r = scan(tmp.path()).unwrap();

        assert_eq!(
            r.non_avif_images,
            vec![(
                "noisy-bright-cat".to_string(),
                "issues/noisy-bright-cat/attachments/shot.png".to_string()
            )]
        );
        assert_eq!(r.large_binaries.len(), 1);
        assert_eq!(r.large_binaries[0].0, "noisy-bright-cat");
        assert!(r.large_binaries[0].2 > (1 << 20));
        // `shot.png` resolves; `gone.avif` does not.
        assert_eq!(
            r.broken_attachment_refs,
            vec![(
                "noisy-bright-cat".to_string(),
                "attachments/gone.avif".to_string()
            )]
        );

        // Warning-only: none of these are critical blockers.
        assert!(critical_blockers(&r).is_empty());
    }

    #[test]
    fn clean_issue_with_avif_attachment_has_no_attachment_warnings() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "calm-quiet-otter",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![shot](attachments/shot.avif)\n",
        );
        let att = tmp.path().join("issues/calm-quiet-otter/attachments");
        fs::create_dir_all(&att).unwrap();
        fs::write(att.join("shot.avif"), b"small").unwrap();

        let r = scan(tmp.path()).unwrap();
        assert!(r.non_avif_images.is_empty());
        assert!(r.large_binaries.is_empty());
        assert!(r.broken_attachment_refs.is_empty());
    }

    /// Regression: `broken_attachment_refs` must not flag link/image
    /// syntax that lives inside a backtick code span. The author is
    /// describing the syntax, not using it. Class 1 of issue
    /// @doctor-attachment-refs-false-positives.
    #[test]
    fn broken_refs_skips_link_syntax_inside_code_span() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "code-span-with-image-syntax",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             Use `![alt](path)` syntax for images, and `[text](url)` for links.\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_attachment_refs.is_empty(),
            "link syntax inside backticks must not be flagged: {:?}",
            r.broken_attachment_refs
        );
    }

    /// Regression: a `..`-escaping link target (the path leaves the
    /// issue dir) is already rejected by `normalize_relative_ref`'s
    /// component check, regardless of the cross-file-pointer logic.
    /// Pin that path explicitly so a future change to either layer
    /// doesn't silently start flagging cross-dir links.
    #[test]
    fn broken_refs_skips_parent_dir_escape_link() {
        let tmp = fresh_repo();
        fs::write(tmp.path().join("foo.ts"), b"// stub\n").unwrap();
        put_flat(
            &tmp,
            "parent-dir-escape",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             See [foo.ts:10-20](../foo.ts#L10-L20).\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_attachment_refs.is_empty(),
            "parent-dir escape must not surface as a broken ref: {:?}",
            r.broken_attachment_refs
        );
    }

    /// Regression: a non-escaping repo-relative pointer with a
    /// GitHub-style `#L<n>` line anchor — the actual Class-2 shape
    /// from the downstream project bug report. The path resolves under the issue
    /// dir (not via `..`), so the heuristic that gates the repo-root
    /// existence check on the anchor shape is what saves it.
    #[test]
    fn broken_refs_skips_repo_relative_code_pointer_with_line_anchor() {
        let tmp = fresh_repo();
        // Mirror the downstream project shape: a real source file lives at the
        // repo root and is referenced from the issue body with a
        // `#L<n>` permalink fragment.
        fs::create_dir_all(tmp.path().join("kurssi-ai-server/src/cli")).unwrap();
        fs::write(
            tmp.path().join("kurssi-ai-server/src/cli/sops.ts"),
            b"// stub\n",
        )
        .unwrap();
        put_flat(
            &tmp,
            "code-pointer-with-line-anchor",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             `loadMoodleAdminPassword()` in \
             [sops.ts:87-98](kurssi-ai-server/src/cli/sops.ts#L87-L98).\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert!(
            r.broken_attachment_refs.is_empty(),
            "GitHub-permalink-shaped code pointer must not be flagged: {:?}",
            r.broken_attachment_refs
        );
    }

    /// Regression for the silent-false-negative class identified in
    /// review: a missing sibling attachment whose filename collides
    /// with a repo-root file (`README.md`, `Cargo.toml`, …) must STILL
    /// be flagged. The earlier "exists at repo root → skip" heuristic
    /// silently masked these; the line-anchor gate is what keeps the
    /// bare-filename case honest.
    #[test]
    fn broken_refs_still_flags_when_filename_collides_with_repo_root() {
        let tmp = fresh_repo();
        fs::write(tmp.path().join("README.md"), b"# repo readme\n").unwrap();
        put_flat(
            &tmp,
            "collides-with-repo-root",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![logo](README.md)\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(
            r.broken_attachment_refs,
            vec![(
                "collides-with-repo-root".to_string(),
                "README.md".to_string()
            )],
            "missing sibling attachment must not be masked by a repo-root collision"
        );
    }

    /// Positive case: a genuinely missing sibling attachment must
    /// still be flagged after the parser/scope refactor.
    #[test]
    fn broken_refs_flags_legit_missing_sibling_attachment() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legit-missing-attachment",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![screenshot](missing.png)\n",
        );
        let r = scan(tmp.path()).unwrap();
        assert_eq!(
            r.broken_attachment_refs,
            vec![(
                "legit-missing-attachment".to_string(),
                "missing.png".to_string()
            )]
        );
    }

    /// Sibling attachment that exists must not be flagged.
    #[test]
    fn broken_refs_clean_for_existing_sibling_attachment() {
        let tmp = fresh_repo();
        put_flat(
            &tmp,
            "legit-existing-attachment",
            "---\ntype: bug\nstatus: open\npriority: normal\n---\n# T\n\n\
             ![ok](existing.avif)\n",
        );
        fs::write(
            tmp.path()
                .join("issues/legit-existing-attachment/existing.avif"),
            b"x",
        )
        .unwrap();
        let r = scan(tmp.path()).unwrap();
        assert!(r.broken_attachment_refs.is_empty());
    }

    /// Issue @doctor-fix-noop, success criterion D: pin the exit-code
    /// contract via `classify_exit`. Unit-testable so the mapping
    /// doesn't drift behind `run`'s `std::process::exit` site.
    #[test]
    fn classify_exit_maps_apply_outcomes_to_envelope_codes() {
        // Clean Ok + no manual leftovers → exit 0.
        let findings = DoctorFindings::default();
        let oc = ApplyOutcome::default();
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 0, "clean Ok must exit 0");

        // Realistic Ok + notes leftovers: `findings.notes_conflicts`
        // ALSO contains the slug (because the post-apply rescan
        // surfaces unmergeable bodies), so the dead-code regression
        // (gemini #1, opus 1.2) requires this assertion to hit the
        // specific notes-merge branch despite `crit` being non-empty.
        let mut findings_with_notes = DoctorFindings::default();
        findings_with_notes.notes_conflicts.push("foo".into());
        let mut oc = ApplyOutcome::default();
        oc.notes_conflicts_at_apply.push("foo".into());
        let d = classify_exit(&findings_with_notes, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-partial");
        assert!(
            d.message.contains("manual") && d.message.contains("Notes"),
            "message must call out the manual notes/comments merge, got: {}",
            d.message
        );

        // Preflight → doctor-blocked.
        let oc = ApplyOutcome {
            stop_phase: StopPhase::Preflight,
            blockers: vec!["dup".into()],
            ..ApplyOutcome::default()
        };
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-blocked");
        assert!(d.message.contains("preflight"));

        // PostApply → doctor-partial.
        let oc = ApplyOutcome {
            stop_phase: StopPhase::PostApply,
            blockers: vec!["x".into()],
            ..ApplyOutcome::default()
        };
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-partial");
        assert!(d.message.contains("post-apply"));

        // apply_error → doctor-apply-error.
        let oc = ApplyOutcome {
            apply_error: Some("oops".into()),
            ..ApplyOutcome::default()
        };
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-apply-error");

        // Ok + generic critical findings (no notes leftover) →
        // doctor-partial with the generic "unfixable" message.
        let mut findings = DoctorFindings::default();
        findings.duplicate_slugs.push("dup".into());
        let oc = ApplyOutcome::default();
        let d = classify_exit(&findings, Some(&oc), true);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-partial");
        assert!(d.message.contains("unfixable"));

        // Read-only with critical findings → doctor-unhealthy.
        let d = classify_exit(&findings, None, false);
        assert_eq!(d.code, 1);
        assert_eq!(d.error_code, "doctor-unhealthy");
    }
}
