---
created: 2026-05-09
updated: 2026-05-09
type: chore
reporter: jari
status: in-progress
priority: normal
epic: exorbitantly-ill-apples
related: ['@partially-ahead-button']
labels: [release-v0.5.0]
---

# Centralize custom-field-key validation across CLI and API create/update

## Description

Spin-off from @partially-ahead-button /llm-review (round 1, finding M15 from openai).

## Description

`UpdateIssueRequest::validate()` rejects invalid custom-field keys and reserved built-in keys (`status`, `created`, etc.) before letting them reach the frontmatter. `NewIssueRequest` does NOT run the same checks before constructing `NewArgs`:

```rust
// mutate/mod.rs (after the @partially-ahead-button refactor)
custom_fields: req.custom_fields,  // no key-shape validation, no reserved-key check
```

`do_new_locked` rejects only **duplicate** keys. The validation surface is asymmetric:

| Path | Duplicate keys | Invalid key shape (e.g. `team:name`) | Reserved built-in (e.g. `status`) |
| --- | --- | --- | --- |
| CLI new | ✅ (clap parser + duplicate check) | ✅ (`parse_custom_field_key`) | ✅ (`parse_custom_field_key`) |
| CLI update | ✅ | ✅ | ✅ |
| API new | ✅ (after @partially-ahead-button) | ❌ | ❌ |
| API update | ✅ | ✅ | ✅ |

API-side new-issue creation can accept a payload like `{"custom_fields": {"status": "anything"}}` and the frontmatter overwrite ordering happens to mask the damage today, but this is implicit and fragile.

## Fix sketch

Extract a shared validator in the domain module that lives between the CLI parser and `do_new_locked`/`update_issue`:

```rust
pub(crate) fn validate_custom_field_pairs(
    fields: &[(String, String)],
) -> Result<(), String> {
    for (key, value) in fields {
        if !is_valid_custom_field_key(key) { return Err(...); }
        if let Some(hint) = reserved_custom_field_hint(key) { return Err(...); }
        // value: trim/empty checks if applicable
    }
    Ok(())
}
```

Then call it in both:
- `mutate::new_issue` BEFORE constructing `NewArgs` (so the API path runs the same gate as CLI).
- `mutate::update_issue` (already does, but route through the shared validator).

The duplicate-key check stays where it is (in `do_new_locked` for new, in `validate_custom_field_pairs` for update).

## Also in scope: duplicate-key rejection on update

Surfaced by the round-2 /llm-review on @partially-ahead-button.

`NewIssueRequest::custom_fields` now rejects duplicate JSON object
keys at the wire boundary (commit a1debda + follow-up). The update
path still has the silent-dedup behavior:

```rust
// mutate/mod.rs
pub custom_fields: BTreeMap<String, Patch<String>>,
```

A `PATCH {"custom_fields": {"team":"a","team":null}}` keeps whichever
value `serde_json` picks last — undefined which one. The same
`deserialize_custom_fields_no_dups`-style visitor applies, but the
value type is `Patch<String>` instead of `String`. Two implementation
options:

1. Make the visitor generic over `V: Deserialize<'de>` and use it for
   both request shapes (collect into Vec, then convert to BTreeMap
   downstream if map semantics are needed).
2. Add a parallel `deserialize_patch_map_no_dups` for the update path.

Option (1) is cleaner. Either way the test patches mirror the
existing `new_issue_request_*` set on the new path.

## Definition of done

- API `POST /api/issues` and `PATCH /api/issues/<slug>` reject invalid/reserved custom-field keys with the same error message the CLI uses.
- API `PATCH /api/issues/<slug>` rejects duplicate `custom_fields` keys at deserialization, mirroring `POST /api/issues`.
- Test pinning the API rejection added to `mutate::tests` (both create and update paths).
- No test suite regressions.
