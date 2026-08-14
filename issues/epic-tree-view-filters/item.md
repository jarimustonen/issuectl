---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate]
closed: 2026-08-14
closed_by: jari
---

# Optional --depth/--status/--limit filters for epic tree

## Description

# Optional filters for `epic tree`: --depth, --status, --limit

`epic tree` currently renders the entire subtree unconditionally. For a
large epic with many children, standard tree-view knobs would help:

- `--depth N` — cap how many nesting levels are shown.
- `--status <s>` — hide closed children (e.g. show only open/in-progress).
- `--limit N` — cap children per node.

- **Milloin tämä näkyy käyttäjälle** — kun epicillä on kymmeniä tai satoja
  lapsia (esim. iso release-epic), koko puu kerralla on liikaa.
- **Miten se näkyy** — tuloste vyöryy ruudulle, olennainen (avoimet työt)
  hukkuu suljettujen joukkoon.
- **Miksi sillä on väliä** — navigointinäkymän arvo laskee isoilla
  epiceillä; suodattimet tekevät siitä käyttökelpoisen triage-työkalun.
- **Miksi tämä vaatii oman suunnittelunsa** — uusi lippupinta ja
  suodatuslogiikka (rakennetaanko koko puu ja karsitaanko jälkikäteen, vai
  suodatetaanko rakennusvaiheessa) on oma pieni suunnittelunsa; `build`
  palauttaa nyt koko puun, joten suodatuksella ei ole saumaa vielä.

Not required for v1 of the read-only view; file as a follow-up enhancement.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: YAGNI — depth/status/limit filters only matter once a large epic exists; none does. File anew if that changes.
