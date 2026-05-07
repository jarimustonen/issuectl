---
created: 2026-05-06
updated: 2026-05-06
type: feature
status: open
priority: normal
reporter: jari
assignee: jari
epic: exorbitantly-ill-apples
labels: [config, schema]
---

# Issues schema file describing fields (required + optional), auto-write defaults

_Source: issues/ (new schema file), src/schema.rs (new), src/cli/{new,update,doctor}.rs_

## Description

Add a schema/description file in the issues/ directory that documents (a) all possible frontmatter fields and (b) which are required. issuectl reads it when creating/updating/validating issues. If the file is missing, issuectl writes a default schema on first use. Doctor uses the schema to flag missing required fields. Open questions: format (YAML/TOML/JSON), filename (issues/schema.yaml? .issuectl/schema.yaml?), how custom user fields interact with built-in ones, whether the schema can constrain enums (e.g. allowed labels, allowed statuses).
