---
created: 2026-05-09
updated: 2026-05-09
type: chore
reporter: jari
status: open
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

## Definition of done

- API `POST /api/issues` and `PATCH /api/issues/<slug>` reject invalid/reserved custom-field keys with the same error message the CLI uses.
- Test pinning the API rejection added to `mutate::tests`.
- No test suite regressions.
