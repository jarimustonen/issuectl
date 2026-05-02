---
created: 2026-04-29
updated: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: in-progress
priority: normal
labels: [i18n, email, multi-tenant]
related: ["#25", "#40"]
---

# 42. Monikielisyys (fi / sv / en) sähköposteille ja UI:lle, organisaatio- ja käyttäjätason kielipreferenssi

_Source: Paavon kutsu-sähköposti tuli väärällä kielellä (suomi vahvistusviestin templaattia)_

## Description

Sovelluksen oletuskieli on **englanti**. Suomenkielinen markkina on tärkeä,
mutta tuotantoasiakkaat ovat lähtökohtaisesti kansainvälisiä, joten
oletusvalinnan pitäisi heijastaa sitä.

Lisäksi:

- **Organisaatio (tenant)** voi asettaa oman oletuskielensä ylläpitoliittymässä.
  Kaikki organisaation lähtevät viestit (ja UI) käyttävät sitä, ellei
  käyttäjäkohtaista preferenssiä ole asetettu.
- **Käyttäjä** voi asettaa oman kielen joka voittaa organisaation oletuksen.
- Tuetut kielet MVP:ssä: **fi**, **sv**, **en**. Kanta on suunniteltava niin,
  että lisääminen on triviaalia.

Konkreettisesti tämä tarkoittaa:

- Kaikki API:n nykyiset suomenkieliset stringit (jotka näkyvät käyttäjälle)
  pitää lokalisoida — sähköpostien sisältö, server-rendered HTML-sivut
  (`/login`, `/set-password`, `/accept-invitation`, `/reset-password`,
  `/admin`, `/admin/users`, `/me`, virhesivut), mukaan lukien painikkeet,
  otsikot, validointiviestit.
- `tenants`-tauluun **default_locale**-kenttä (fi/sv/en, default `en`).
- `users`-tauluun **locale**-kenttä (Option, fallback tenantin oletukseen).
- Pääkäyttäjälle `/admin/settings` (tai vastaava paikka) jossa voi vaihtaa
  organisaation oletuskielen.
- Käyttäjälle myöhemmin oma asetus omasta kielestä (`/me/settings` tms),
  alustavasti vähintään tietokantakentän tasolla.

## Scope (vaihe 1)

- [ ] Migraatio: `tenants.default_locale TEXT NOT NULL DEFAULT 'en'` +
      check-constraint `IN ('fi','sv','en')`. `users.locale TEXT NULL`
      sama constraint.
- [ ] `ops::session::ResolvedSession` (tai erillinen resolver) palauttaa
      efektiivisen kielen: `user.locale.unwrap_or(tenant.default_locale)`.
- [ ] Lokalisointi-infra: yksinkertainen `Locale`-enum, `t!()`-makro tai
      tavallinen mappi-funktio (`fn t(locale, key) -> &str`). MVP:ssä
      ei tarvita varsinaista i18n-kirjastoa — staattinen
      `&'static str`-mappi joka kieli erikseen riittää.
- [ ] **Sähköpostien lokalisointi**:
      - registration verification (`set-password`-linkki)
      - invitation (sis. "X kutsuu sinut Y:hyn" -kontekstia, _erillinen
        templaatti_ verifikaatiolta — ks. issue #3 / nykyinen sessio)
      - password reset
      - welcome from `assistant@`
      - Aineisto englanniksi, suomeksi ja ruotsiksi.
- [ ] **HTML-sivujen lokalisointi**: kaikki nykyiset strings
      (`web.rs`, `routes/*.rs`).
- [ ] Admin-UI:
      - `/admin/settings` (uusi sivu) tai `/admin`-dashboardin
        laajentaminen: dropdown jolla pääkäyttäjä asettaa
        `tenants.default_locale`.
      - `/admin/users`-listalla per-käyttäjä-rooli-rivin viereen
        nykyisen `Käyttäjä/Hyväksyjä`-selectorin tapaan: kieli-select.
- [ ] Default-arvo uusille tenanteille: `en`.
- [ ] Olemassa olevat tenantit: migraatio asettaa `en` kaikille (tai
      `fi` jos haluamme migraatioajoksi parempaa, päätettävissä).

## Out of scope (myöhempiin issueihin)

- Kielen valinta käyttäjälle itselleen UI:ssa (vaihe 2 — `/me/settings`).
- Asiakaspuolen JS-bundlen i18n (`sites/www`, `sites/app`) — eri pino.
- Alueellinen formatointi (numerot, päivämäärät, valuutta) — toistaiseksi
  käytetään ISO 8601 ja euroja symboleina.
- Markkinointisivun (`grooveserve.com`) lokalisointi.

## Quick Test

```bash
# 1. Self-register tenantti — vahvistusviesti tulee oletuksena englanniksi
gsdev mail clean
curl -X POST http://localhost:53004/api/register \
  -H "Content-Type: application/json" \
  -d '{"company_name":"Acme","contact_name":"John","email":"john@acme.com"}'
# Mailpitistä: viesti englanniksi.

# 2. Pääkäyttäjä vaihtaa organisaation kieleksi suomi /admin/settings:istä.
# 3. Pääkäyttäjä kutsuu uuden käyttäjän:
# 4. Mailpitistä: kutsu-viesti suomeksi.
# 5. Käyttäjäasetuksiin ruotsi (DB-tasolla).
# 6. Salasanan resetointi → ruotsiksi.
```

## Toteutusvinkkejä

- Älä keksi i18n-kirjastoa MVP:hen. Pelkkä `match locale { Fi => "...",
  Sv => "...", En => "..." }`-haarautuminen on kestävä.
- Jokainen string-konstantti omaksi nimetyksi funktioksi
  (`fn email_subject_invitation(locale, inviter_name, tenant_name)`)
  joka palauttaa kielen mukaan oikean version. Helpompi käännöstyö
  ja koodi-grepattavuus parempi kuin avain-pohjainen `t!("foo.bar")`.
- Sähköposteissa: testaa Mailpitin kautta että kaikki kolme kieltä
  rendautuvat siististi (ei tyhjiä paikkamerkkejä, oikeat erikoismerkit).
- Käytä olemassa olevaa Playwright-spec:iä pohjana — toistaiseksi se
  ajaa vain `/`-kielellä (käytännössä englanniksi muutoksen jälkeen).
  Kopioi se kahdeksi muuksi varianttina (fi/sv) kun lokalisointi on
  paikallaan.
