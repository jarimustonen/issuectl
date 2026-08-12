---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: open
priority: normal
epic: hugely-exciting-spiders
labels: [v0.6.0-candidate]
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
