---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: open
priority: normal
---

# issuectl dag: stable intra-lane ordering key (lane_seq)

## Description

`issuectl dag` orders issues within a serial lane by "topological on `blocked_by`, then priority, then **slug**". The final slug tie-break is **lexical**, so two lane members with no dependency and equal priority are ordered alphabetically — and renaming an issue silently changes which one is head-of-line.

## Problem
A lane's intended intra-lane precedence is often a soft human judgment ("do the throughput item before the hardening item") that is **not** a hard `blocked_by` dependency. Today the only way to encode it is to fabricate a `blocked_by` edge (corrupts the dependency graph) or accept alphabetical order.

## Proposal
Add an optional coarse sort key — e.g. `lane_seq: <int>` — consulted **after** `blocked_by` topo but **before** the slug tie-break. Absent → today's behaviour. This is the `lane_seq` field sketched in the homebase research doc `research/agent-dag-tool-placement.md`.

## Context
Filed from homebase `adopt-issuectl-dag`. Concretely: the `digest` lane's two live members (`digest-whisper-endpoint-pool` = higher-value, `digest-orchestrate-agent-profile` = non-blocking hardening) invert under slug order.
