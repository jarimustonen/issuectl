---
created: 2026-06-04
updated: 2026-06-04
type: feature
reporter: jari
status: open
priority: normal
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

## Konteksti

- 3DBear-monorepon epic: `especially-brown-field` (plan-dokumentti
  siellä päässä menee yksityiskohtaisempaan toteutus-suunnitteluun).
- Tech-team-palaute 2026-06-04 nosti tarpeen geneerisyyden tarkasteluun
  ("voisiko tämä olla issuectl:n ominaisuus eikä per-repo-konventio").
