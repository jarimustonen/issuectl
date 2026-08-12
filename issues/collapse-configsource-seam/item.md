---
created: 2026-08-12
updated: 2026-08-12
type: improvement
status: in-progress
priority: normal
---

# Collapse single-impl ConfigSource seam (post-web-UI cleanup)

## Description

# Collapse the single-impl `ConfigSource` seam (post-web-UI cleanup)

## Tausta
Web-UI:n poiston (`@remove-web-ui`, 0.10.0) jälkeen `RepoConfigCache` on poistettu ja
`ConfigSource`-traitilla on enää yksi toteutus, `UncachedConfig`. Silti `&dyn ConfigSource`
kulkee ~150 kutsupaikan läpi (mutate-kerros + `repo::*_via`/`*_with_config`), ja
`schema::load` / `transitions::load` palauttavat `Arc<T>`:n välimuistille jota ei enää ole.

## Milloin tämä näkyy käyttäjälle
Ei suoraa käyttäjävaikutusta — puhdas sisäinen refaktorointi. Perustelu: dyn-dispatch ja
`Arc`-varaus ovat mitättömiä kustannuksia, ja käyttäytyminen on identtinen.

## Miksi tämä vaatii oman suunnittelunsa
Traitin romauttaminen koskettaa samoja ~150 kutsupaikkaa jotka juuri muutettiin `hub`-poistossa,
plus julkiset `pub fn`-signatuurit (`update_issue`, `new_issue`, `close_issue`, `bulk_update`,
`update_body`, `note_issue`, `toggle_checkbox`, `repo::load_issues_with_*`). Vaihtoehdot: (a)
poista trait kokonaan ja kutsu `schema::load`/`transitions::load` suoraan; (b) sisäistä seam
`pub(crate) *_via` -variantiksi ja pidä julkiset signatuurit kapeina. Molemmat ovat oma
mekaaninen refaktorointinsa, joka kannattaa tehdä samassa 0.10.0-rikkovassa ikkunassa mutta
erillisenä, testattavana muutoksena — ei niputettuna web-poiston jättidiffiin. `load` voi tällöin
palata arvoina (`Schema`/`TransitionRules`) `Arc`:n sijaan.
