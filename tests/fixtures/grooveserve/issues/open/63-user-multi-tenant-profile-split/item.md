---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#26", "#36", "#56"]
labels: [schema, multi-tenant, privacy]
---

# 63. user_profiles split — globaali identiteetti + per-tenant agentin muisti

_Source: A3 (#56 Phase 1) /llm-review + assess-findings_

## Description

A3:n yhdistetty skeema (#56) jätti `user_profiles`-taulun **globaalisti
yhteen rivinriviin per käyttäjä**: `user_id` on UNIQUE FK `users(id)`:hen,
ei `(tenant_id, user_id)`. Kun M-to-M-jäsenyys (`tenant_users`) sallii
saman käyttäjän kuulumisen useaan tenanttiin, profiilin kentät vuotavat
tenanttirajan yli.

A3:n vaiheessa päätettiin **olla muuttamatta tätä nyt** — kaikki
käyttäjät saavat globaalin profiilin, ja split tehdään tässä issueessa
kun multi-tenant-membership tulee tuotteeseen tai kun tarpeen
ensimmäinen konkreettinen tapaus ilmestyy.

## Reaalimaailman ongelma

Konsultti Jari kuuluu tenantteihin Acme Oy ja Bobcorp Ltd. Agentti
oppii Acme-viestistä että "Jari preferoi junaa Helsinki–Tampere" ja
kirjoittaa sen `user_profiles.notes_md`:hen. Seuraavalla viikolla
agentti käsittelee Bobcorp-viestin → lukee saman `notes_md`:n → näkee
Acmen sisäisiä matkustustottumuksia. Tietosuojavuoto.

## Kentät kategorioittain

| Kenttä | Per ihminen | Per (tenant, ihminen) | Huom |
|--------|-------------|------------------------|------|
| `home_address`, `work_address`, lat/lng | ✅ | | Yksi koti/työpaikka per ihminen |
| `language` | ✅ | | Sama kieli kummankin asiakkaan suuntaan |
| `default_transport` | ⚠️ | ⚠️ | Voi olla per-tenant (firma-auto Acmella) |
| `default_vehicle` | ⚠️ | ⚠️ | Sama kuin yllä |
| `preferences` (JSONB) | osittain | osittain | Splittaus avain-avaimelta |
| `notes_md` | | ✅ | **Kriittisin** — agentin tenant-derivoima muisti |

## Suositeltu toteutus

Jaa `user_profiles` kahteen tauluun:

1. **`user_profiles` (globaali, säilyy nimellä)**:
   - `user_id` UNIQUE FK
   - `home_address`, `work_address`, lat/lng, `language`,
     `default_vehicle` (jos malli on "ihmisen yksi auto"),
     globaali osuus `preferences`-JSONB:stä
2. **`tenant_user_notes` (per tenant)**:
   - `(tenant_id, user_id)` UNIQUE
   - Composite FK `tenant_users(tenant_id, user_id)`
   - `notes_md`, tenant-spesifit `preferences`-avaimet, mahdollisesti
     `default_transport` (firma-auto)
   - Audit `tenant_user_note_revisions` (tai laajenna nykyinen
     `user_profile_revisions`)

## Riippuvuudet

- **#26** Multi-tenant käyttäjähallinta — tämä split on järkevä vasta
  kun #26:n rekisteröinti/kutsu/jäsenyys on ajossa. Saman käyttäjän
  kutsuminen kahteen tenanttiin on #26:n ominaisuus, ei A3:n.
- **#36** Privacy review — tämä split on osa GDPR-tietosuojan
  ratkaisua (`notes_md`:n eristys per-tenant pienentää right-to-erasure
  -työn pinta-alaa).

## Kuinka A3 jätti tilan

- `crates/ops/migrations/015_create_user_profiles.sql` sisältää
  TODO-headerin joka viittaa tähän issueen.
- `notes_md` ja `preferences` ovat tällä hetkellä globaaleja —
  multi-tenant-membership-tuotteen valmistuessa nämä on aktiivisesti
  refaktoroitava.
- A3:n empty-DB-cutover-ikkuna on suljettu A4:n valmistuttua, joten
  splittaus tehdään uutena ALTER/CREATE-migraationa, ei A3:n migraation
  muokkauksena.

## Avoimet kysymykset

1. **`default_transport` ja `default_vehicle`**: per-ihminen vai
   per-tenant? Riippuu siitä onko firma-auto yleinen tapaus.
2. **`preferences`-JSONB**: koko mappi per-tenanttiin, vai split
   per-avain (esim. `notify_*` globaalisti, `category_defaults`
   per-tenant)?
3. **Audit-historia**: `user_profile_revisions` viittaa nyt
   `users(id)`:hen. Splittauksen myötä se halkeaa kahteen
   audit-tauluun, vai säilyykö yksi joka kantaa molempien diffit?

Vastataan kun tämä tulee toteutusvuoroon.
