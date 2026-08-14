---
created: 2026-08-14
updated: 2026-08-14
type: improvement
status: open
priority: normal
labels: [cli]
---

# note/close --as should strip a leading '@' instead of rejecting it

## Description

## Comments

### 2026-08-14T04:32:14Z · @jari

Observed (this session, hit twice): `issuectl close <slug> --as "@jari"` and `issuectl note <slug> --as "@jari" …` hard-fail with 'error: validation: author cannot contain '@''. Had to retry with bare 'jari'.

Expected: strip a single leading '@' and accept it (normalize "@jari" → "jari"). Attributions are DISPLAYED as '@jari' (note headers show '· @jari'), so an agent naturally types the '@'; rejecting it is a surprising asymmetry between how the author is shown and how it must be entered. Applies to every author-taking flag (`note --as`, `close --as`, `close --note --as`, and the optional closer attribution). Keep rejecting an '@' that appears mid-string; only strip a single leading one. Small; touches the author-validation/parse helper in issuectl-core (mutate/ author path).
