---
created: 2026-05-08
updated: 2026-05-08
type: feature
reporter: jari
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [agent-friendly, v0.6.0-candidate]
---

# issuectl note: accept message via --stdin / --from-file

## Description

Spin-off from review of @overly-dreary-yak (D3). The current 'issuectl note <slug> --as <user> "<msg>"' takes the message as a positional shell arg. Fine for one-liners but hostile for multi-line markdown (code blocks, lists, longer narratives) — quoting and shell escaping become brittle. Mirror 'body set': add --stdin and --from-file PATH flags. Validation rules for empty stdin, agent-vs-human ergonomics, follow-on changes to the skill template.
