---
created: 2026-05-06
updated: 2026-05-08
type: feature
status: in-progress
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [config, schema]
commits:
- hash: 6cda8bc
  summary: add schema file with required+enum validation, doctor + new/update enforcement, auto-bootstrap
- hash: f66a6f6
  summary: apply LLM-review fixes (merge, type-strict, --field, atomic bootstrap, body-set validation, error classification)
- hash: 83eeb71
  summary: round-2 review fixes (atomic bootstrap, error split SchemaViolation/SchemaConfig, API custom_fields, load-time guards)
---

# Issues schema file describing fields (required + optional), auto-write defaults

_Source: issues/ (new schema file), src/schema.rs (new), src/cli/{new,update,doctor}.rs_

## Description

Add a schema/description file in the issues/ directory that documents (a) all possible frontmatter fields and (b) which are required. issuectl reads it when creating/updating/validating issues. If the file is missing, issuectl writes a default schema on first use. Doctor uses the schema to flag missing required fields. Open questions: format (YAML/TOML/JSON), filename (issues/schema.yaml? .issuectl/schema.yaml?), how custom user fields interact with built-in ones, whether the schema can constrain enums (e.g. allowed labels, allowed statuses).
