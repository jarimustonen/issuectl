---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: wontfix
priority: normal
closed: 2026-08-14
closed_by: jari
---

# Thread loaded schema/rules through mutate helpers to stop redundant re-parses

## Description

# Thread loaded schema/rules through mutate helpers to stop redundant re-parses

## Tausta
`ConfigSource`-seamin romautuksen jälkeen jokainen mutate-apuri lataa
skeeman/säännöt suoraan levyltä (`crate::schema::load` / `transitions::load`).
Tämä säilytti aiemman käyttäytymisen tarkasti, mutta paljasti että sama
`.schema.yaml` jäsennetään monta kertaa yhtä komentoa kohti:

- `mutate::mod::note_issue` / `toggle_checkbox` lataavat skeeman, sitten
  `validate_against_schema` ja `transition_warnings` lataavat sen uudelleen.
- `intake::file`: `matching_source_ref` → `load_issues` (lataa skeeman) +
  `do_new_locked` (lataa skeeman) → jopa 3 jäsennystä.
- `recurrence::run`: `do_new_locked` lataa skeeman silmukassa (≤50 kertaa).

## Ei suoraa käyttäjävaikutusta — perustelu
Skeema-YAML on pieni (~2 KB) ja CLI-prosessi on lyhytikäinen; kustannus on
mitätön ja `repo_config.rs`:n dokumentti kutsui sitä nimenomaan
"hyväksyttäväksi". Tämä on suorituskyky-/selkeyssiivous, ei bugikorjaus.

## Miksi tämä vaatii oman suunnittelunsa
Korjaus lankittaa `&Schema` / `&TransitionRules` sisään usean apurin
signatuuriin (`validate_against_schema`, `transition_warnings`,
`do_new_locked`, ja `repo::load_issues`, jolla ei ole tapaa ottaa vastaan
esiladattua skeemaa). Se on rakenteellinen refaktorointi omine
kutsupaikkoineen ja testeineen — pidettävä erillään
`collapse-configsource-seam`:in käyttäytymistä-säilyttävästä diffistä. Riippuu
osittain siitä palautetaanko `load` arvona
([[configsource-load-return-value]]).

## Resolution

### 2026-08-14T03:41:55Z · @jari

Wontfix: threads schema through many helper signatures to stop redundant re-parses — a perf/plumbing change, not a readability win (adds coupling). Cost is negligible; the code already calls it acceptable.
