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

# Sanitize control chars in human tree/list output

## Description

# Sanitize control characters in human tree/list output

The human-readable renderers (`issuectl epic tree`, `dag`, `show`, `ls`)
print issue titles verbatim. A title containing `\n`, `\r`, a tab, or a
terminal escape sequence breaks the aligned layout and — in the escape
case — can inject misleading terminal output. `--json` is unaffected.

- **Milloin tämä näkyy käyttäjälle** — kun jonkin issuen otsikossa on
  ohjausmerkki tai ANSI-escape (käsin muokattu frontmatter, liitetty
  teksti, tuontidata GitHubista).
- **Miten se näkyy** — puurakenteen sisennys hajoaa, tai pahimmillaan
  otsikko piirtää terminaaliin väärää tekstiä (escape-injektio).
- **Miksi sillä on väliä** — luettavuus kärsii ja teoriassa käyttäjää voi
  harhauttaa väärennetyllä terminaalitulosteella; JSON-polku on turvassa.
- **Miksi tämä vaatii oman suunnittelunsa** — koskee koko koodikantaa
  (kaikki ihmisluettavat renderöijät), ei vain epic-treetä. Oikea korjaus
  on jaettu sanitointiapuri (esim. `issue_fields`-moduuliin) jota kaikki
  tulostuspolut käyttävät yhtenäisesti — laajempi kuin tämä feature.

Suggested approach: add one shared helper that strips/escapes control
characters for terminal display, and route every human renderer through
it. Keep JSON emitting raw values.

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: not hardening titles against deliberate control-char/escape injection — titles shouldn't contain these, and defending against someone deliberately breaking it is not a realistic concern for this tool.
