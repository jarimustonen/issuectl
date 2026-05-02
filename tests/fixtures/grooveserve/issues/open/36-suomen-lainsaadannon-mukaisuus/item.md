---
created: 2026-04-28
updated: 2026-04-28
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#9", "#20", "#33"]
labels: [ai, skills, saannot, accounting]
---

# 36. Skillien ja työkalujen mukauttaminen Suomen lainsäädäntöön

_Source: `services/email/src/tools/`_

## Description

Step 3a:n (#33) yhteydessä skill-tiedostoihin päätyi domain-väitteitä Suomen matkalasku- ja kirjanpitokäytännöistä, joita ei ole varmistettu lähteistä eikä toteutettu koodissa. Esimerkkejä tunnistetuista väitteistä:

- `save_receipt.skill.md`: "For hotel-chain breakfast receipts, prefer `accommodation` even though the line items are food; that matches Finnish travel-expense practice." — VAT-kantojen kannalta epäselvä; aamiainen on usein 14 % (ruoka) ja majoitus 10 %.
- `get_user_context.skill.md`: implikoi km-korvauslaskennan käyttäen `home_address`/`work_address`/`default_vehicle` tietoja, vaikka laskentatyökalua ei ole olemassa (#20 osa).
- Kategoria-enum (`food`, `accommodation`, `transport`, `fuel`, `parking`, `software`, `telecom`, `office`, `other`) ei ole sidottu Suomen kirjanpitotileihin eikä Verohallinnon kululuokkiin.
- ALV-kantojen (`vat_rate`, `vat_amount`) validointia ei ole — agent voi tallentaa minkä tahansa luvun ilman, että mikään yhdistää sitä kirjanpitokelpoisiin Suomen ALV-kantoihin (24/14/10/0).

Ongelma on, että agentti voi tehdä laillisesti tai kirjanpidollisesti virheellisiä päätöksiä joiden korjaaminen jälkikäteen on työlästä (väärä ALV-purku, väärä kategoria, väärä km-korvaus).

## Scope

Tämä issue ei toteuta itse sääntöjä — sen tarkoitus on tehdä auditointi ja siirtää säännöt skill-proosasta varsinaiseksi koodiksi tai linkityksiksi olemassa oleviin issueihin (#9, #20). Konkreettiset askeleet:

- [ ] Audit: käy läpi kaikki 10 skilliä ja listaa **jokainen** Suomi-spesifinen domain-väite (kategorisointiohjeet, ALV-vihjeet, km-korvauslaskenta, päivärahojen oletukset, yms.)
- [ ] Lähdekartoitus: jokaisesta auditoidusta väitteestä, mistä se tulee? Verohallinnon ohje, KILA-päätös, kirjanpitolaki, vai pelkkä mututuntuma? Listaa lähteet.
- [ ] Päätös per väite: pidä-skillissä-lähteen-kanssa / siirrä-handler-tason-validaatioon / poista-skillistä-koska-tarvitaan-koodi (linkki #9/#20).
- [ ] Konkretia 1: ALV-kannan validointi handlerissa (`save_receipt`, `add_expense`, `update_*`) — sallituiksi 24/14/10/0 (+ NULL).
- [ ] Konkretia 2: kategoriaenumin sidonta kirjanpitotileihin Netvisor/Procountor-integraatiota varten (#18/#19 yhdyspintaa).
- [ ] Konkretia 3: hotel-breakfast-väitteen tarkistus ja joko verifiointi tai poisto skillistä.

## Notes

Liippaa läheltä:
- **#9 Kirjanpidollisten yksityiskohtien tunnistaminen** — ALV-kantojen tunnistaminen ja kulukategorioiden tarkempi luokittelu. Tämä issue varmistaa, että agentti ei oleta sääntöjä joita #9 ei ole vielä toteuttanut.
- **#20 Verohallinnon päiväraha- ja km-korvausmäärät** — varsinainen laskenta. Tämä issue varmistaa, että skill-teksti ei lupaa sellaista mitä #20 ei tarjoa.
- **#33 Skill-pohjainen tool-arkkitehtuuri** — tämä issue löytyi step 3a:n LLM-katselmuksessa (`history/review-tool-skills-step3a-impl.md`, löydös #23).

## See

- `services/email/src/tools/receipts/save_receipt.skill.md` — hotel-aamiaisväite
- `services/email/src/tools/user/get_user_context.skill.md` — km-laskenta-implikaatio
- `services/email/src/tools/util.rs::VALID_CATEGORIES` — kategoriaenumi
- `history/review-tool-skills-step3a-impl.md` — alkuperäiset löydökset
