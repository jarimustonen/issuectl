---
created: 2026-08-10
updated: 2026-08-10
type: feature
status: done
priority: normal
closed: 2026-08-10
commits:
- hash: 33c8e93
  summary: unlaned parallel-safe lane sentinel
---

# issuectl dag: explicit parallel-safe (unlaned) sentinel distinct from absent lane

## Description

There is no way to express "confirmed to touch no shared hot file → parallel-safe" as distinct from "not yet classified". A shared `lane` value serializes its members; **absent** `lane` means unscheduled/parallel — but absent also reads as "nobody has laned this yet".

## Problem
The DAG convention distinguishes three states: (a) laned (serialize), (b) **confirmed unlaned** (parallel-safe, "run anytime"), (c) unclassified (the merge must lane it). `issuectl 0.8.0` collapses (b) and (c) into "absent lane". A literal `lane: unlaned` does not help — it becomes just another shared lane and would *serialize* everything tagged with it (the opposite of intended).

## Proposal
A first-class parallel-safe marker: e.g. reserve `lane: unlaned` (or a dedicated boolean/`parallel_safe: true`) that `issuectl dag` treats as "independently spawnable, never serialized with siblings", and let `doctor` / a `dag lint` flag genuinely-unclassified issues (absent marker) separately. Matches the three-state design in the homebase research doc `research/agent-dag-tool-placement.md`.

## Context
Filed from homebase `adopt-issuectl-dag`, where the former "LANE D/F" parallel groupings had to be represented as absent-lane, losing the "confirmed parallel-safe" signal.
