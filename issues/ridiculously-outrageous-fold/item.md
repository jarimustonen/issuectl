---
created: 2026-05-10
updated: 2026-05-10
type: improvement
reporter: jari
status: done
priority: normal
epic: hugely-exciting-spiders
labels: [from-3dbear-0.5.1-feedback]
commits:
- hash: a8dbdd4
  summary: render_text long-list collapse via print_section helper; --verbose flag added
- hash: eedf7cb
  summary: render_section refactored over fmt::Write; behavioural test asserts rendered text across collapse boundaries
closed: 2026-05-10
---

# doctor: collapse repetitive 240-line layout warnings; add --verbose to expand

## Description

When --fix is blocked, doctor prints the entire legacy-layout list every iteration (240 lines of warnings scrolling off the screen during 'fix-something-rerun-doctor' loops). Fix: collapse to '240 issues need layout migration' by default; --verbose / --no-collapse to expand. See @intensely-ill-garden for full feedback context (3DBear monorepo 0.3.1 → 0.5.1 migration, 2026-05-10).
