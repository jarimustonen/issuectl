---
created: 2026-06-04
updated: 2026-08-20
type: feature
reporter: jari
status: wontfix
priority: normal
closed: 2026-08-10
---

# Focus areas: validoidut pitkäkestoiset teemat issue-hallintaan

_Source: skeema / labels / cli_

## Description

Tarvekuvaus, ei suunnitelma. Tuotu 3DBear-monorepon
`especially-brown-field`-epicistä (Jari, Alisa, Pekka, Rasmus), jossa
on mietitty issue-hallinnan focus-area-näkymää. Tämän issuen tarkoitus
on pohjustaa keskustelu siitä, **onko tämä issuectl-tason ominaisuus**
ja jos on, miten skeemassa esittää niin että toiminnallisuus on
**riittävän geneerinen mutta ei liikaa**.

## Käyttötapaus

Repo, jossa on 100–300 avointa issueta ja 3–6 hengen tiimi. Tiimi
haluaa nähdä työn kahdesta ulottuvuudesta:

1. **Kuka tekee mitä** — assignee, status, prioriteetti. Tämä on
   issuectl:llä jo katettu.
2. **Mitä isoja teemoja työ koskee** — esim. "simuna.net-migraatio",
   "Kurssi-AI:n kehitys", "sisäiset työkalut". Nämä ovat
   **pitkäkestoisia, ihmisten ylläpitämiä focus-alueita**, eivät
   yksittäisiä projekteja tai epicejä.

Toinen ulottuvuus puuttuu. Workaroundina käytetään vapaita labeleita
(`simuna-net`, `kurssi-ai`, `infra`, …), mutta ne ajautuvat:
kohderepossa ~80 erilaista label-tokenia ilman skeemaa.

## Miksi pelkkä label-konventio ei riitä

- **Alueet eivät elä koodissa** vaan tiimin yhteisessä työskentelyssä;
  uusi alue lisätään keskustelussa ilman PR:ää.
- **Validointi puuttuu**: typo `area:simuna_net` vs `area:simuna-net`
  hajoaa hiljaa.
- **Toolingia ei voi rakentaa**: `issuectl ls --label area:simuna-net`
  toimii, mutta `issuectl stats --by-area` ei, koska CLI ei tiedä
  mitkä label-tokenit ovat alueita.
- **AI tarvitsee kuvaukset**: kun uusi issue luodaan, skill-kerros
  päättää area-tagin sen pohjalta mitä focus-alue tarkoittaa.

## Mitä halutaan saavuttaa

- **Yksi paikka jossa focus-alueet on määritelty** (kuvauksineen),
  versioitu repon kanssa.
- **Validointi** kun area-tagi lisätään issueeen — varoitus jos
  alueeseen ei ole määritelty.
- **CLI-tuki** alueiden listaukseen, lisäämiseen, query-suodatukseen
  ja per-area-aggregointiin.
- **AI-luettavat kuvaukset** joista skill-kerros voi päättää mihin
  alueeseen uusi issue todennäköisesti kuuluu.

## Mitä halutaan välttää

- **Liian raskasta skeemaa**, joka pakottaisi joka issuelle area-arvon.
- **1:1-pakkoa**: yksi issue voi koskea useampaa aluetta.
  Frontmatter-kenttä `area: simuna-net` ei tätä salli.
- **Päällekkäisyyttä label-mekanismin kanssa**: jos focus-alueet ovat
  pohjimmiltaan validoituja labeleita kuvauksilla, niiden ei pidä olla
  täysin eri konsepti.
- **Per-projekti-pakottavuutta**: jos repo ei käytä focus-alueita,
  feature ei saa rasittaa sitä.

## Avoimet suunnittelukysymykset

1. **Validoituja labeleita vai oma konsepti?** Validoidut labelit ovat
   halvempia (ei uutta frontmatter-kenttää), oma konsepti on selvempi
   (`issuectl areas` on luontevampi kuin "labelit joiden prefix on
   `area:`"). Välimuoto: oma kenttä `areas: [...]` skeemassa, joka
   rendaa labeleiksi `area:*` `ls`-näkymässä.
2. **`area`:n suhde `epic`:hen?** Epic = ajallinen projekti joka
   päättyy; area = stabiili leikkaus joka ei pääty. Eri tasot, mutta
   käyttäjälle voi sekoittua.
3. **Per-repo-konfiguraatio `.issuectl/`:n alla** schema-tiedoston
   kanssa, vai erillinen `focus-areas.yaml` repon juuressa? Ensimmäinen
   pitää konfiguraation issuectl:n alaisuudessa; toinen on
   näkyvämpi tiimille joka editoi sitä käsin.
4. **Sub-alueet?** Esim. `course:raksa`, `course:mipa` on 3DBear-spesifi.
   Tukeeko sama mekanismi sekä ylätason focus-alueita että
   hierarkkisia alaluokituksia, vai jätetäänkö sub-tasot vapaaksi
   konventioksi ja standardoidaan vain area?
5. **Validoinnin tiukkuus.** Pakko vai varoitus? Lähtökanta:
   varoitus, mutta per-repo-asetus joka voi nostaa pakoksi.

## Geneerisyys

Tarpeen takana on suomalainen pieni tiimi yhdellä monorepolla, mutta
sama tarve voisi koskea mitä tahansa tiimi-issuetrackingia, jossa:

- Issueta on satoja, ei kymmeniä
- Niitä leikkaa muutama pitkäkestoinen teema (ei epic, ei milestone,
  vaan stabiili "näin me jaamme tämän työn")
- Tiimi tekee säännöllisesti review-kierroksia ("missä mennään X:ssä")
  joissa tarvitsee aggregaatin teemoittain

## Decision

Top-level approach: **option (b) — focus areas modeled as a first-class
`areas: []` list field in the schema, with element-wise validation
against a definition block in `.schema.yaml` and AI-readable
descriptions injected into the `/issue` skill template.** The labels
mechanism is left untouched (open-set, descriptionless). The tiebreaker
was the governance/lifecycle distinction between labels (open-set,
accreting, ad-hoc) and areas (closed-set, team-negotiated, with
descriptions): forcing both into one field would require either bimodal
validation or constraining all labels — both worse than two
lexically-distinct fields with non-overlapping semantics. The internal
schema shape (generic `taxonomies` registry vs inline
`fields.areas.enum_with_descriptions`) is deferred to the
implementation ADR, as are the five open design questions above
(validation strictness, sub-areas, config location, area-vs-epic
distinction, sub-tag standardization).

See [ADR 0001](../../docs/decisions/0001-focus-areas-top-level-approach.md)
for the full reasoning, debate transcript, and constraints handed down
to the implementation ADR.

## Konteksti

- 3DBear-monorepon epic: `especially-brown-field` (plan-dokumentti
  siellä päässä menee yksityiskohtaisempaan toteutus-suunnitteluun).
- Tech-team-palaute 2026-06-04 nosti tarpeen geneerisyyden tarkasteluun
  ("voisiko tämä olla issuectl:n ominaisuus eikä per-repo-konventio").

## Decisions

### 2026-06-04T11:09:33Z · @claude

ADR 0001-focus-areas-top-level-approach records the top-level a/b/c decision: adopt option (b) — focus areas as a first-class areas field in the schema (labels untouched). Open design questions (validation strictness, sub-areas, config location, epic relation, sub-tags) plus the internal schema shape (taxonomies registry vs inline enum_with_descriptions) deferred to the implementation ADR.

## Resolution

### 2026-08-10T10:44:45Z · @issuectl

No current need (2026-08-10). Top-level approach was decided in ADR 0001 (areas: [] schema field); reopen + write the implementation ADR if the need resurfaces.
