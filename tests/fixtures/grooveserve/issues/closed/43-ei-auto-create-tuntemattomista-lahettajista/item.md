---
created: 2026-04-29
updated: 2026-04-30
closed: 2026-04-30
type: bug
reporter: jari
assignee: jari
status: obsolete
priority: high
labels: [security, multi-tenant, auth]
related: ["#26", "#41", "#56"]
commits:
  - hash: 7891b7e
    summary: "fix(email): drop unknown senders instead of auto-creating tenant+user (#43 phase 1)"
---

# 43. Email-service luo automaattisesti tenant+user tuntemattomista lähettäjistä — pysäytettävä

_Source: Roundcube round-trip -testaus paljasti_

## Description

`services/email` luo tällä hetkellä **automaattisesti uuden tenantin
ja käyttäjän** jokaiselle tunnistamattomalle sähköpostin lähettäjälle.
Lokista (`2026-04-29`):

```
"Auto-created tenant and user"
email: "jari@grooveserve.local"
tenant_id: 1
user_id: 1
```

Tämä tapahtuu **ennen spam-tarkistusta** ja **ilman mitään
valtuutusta**. Käytännössä:

- Kuka tahansa joka lähettää sähköpostin `assistant@grooveserve.com`
  -osoitteeseen voi pakottaa järjestelmän luomaan itselleen
  käyttäjätunnuksen (ja tenantin) sähköpostin domainin perusteella.
- Tämä tapahtuu vaikka spam-triage päättäisi olla vastaamatta —
  tietokanta täyttyy "haamutilejä" joista ei ole ketään tavoitettavissa.
- Domain-pohjainen tenant-luonti ohittaa täysin sen organisaatio-
  hallinnan jonka rakennamme web-puolella (rekisteröinti +
  hyväksyntä + admin-rooli).

Tämä on tietoturva- ja eheysongelma. Tuotantokäyttöön asti tämä **ei
saa jäädä**.

## Reproduction

```bash
# Lähetä mailia random@example.com -> assistant@grooveserve.local
# (tai testaa Roundcubessa jari@grooveserve.local -tililtä)
# Tarkista email-puolen DB:
psql grooveserve_email_main_main -c "SELECT id, name, domain FROM tenants;"
# → tenantit luotu lähettäjien perusteella
```

## Korjaussuunnitelma

### Vaihe 1 — pysäytä auto-create heti (oma PR)

- [x] Email-service ei enää luo tenantia eikä käyttäjää tuntemattomalle
      lähettäjälle. Jos lähettäjää ei löydy `users`-taulusta:
  - logataan `event_type="message.skipped"` reason `"unknown sender"`
  - viesti siirretään `Skipped`-kansioon ja merkitään
    `email_processing.status = 'unknown_sender'` (ei tenant/user/thread
    -kirjoituksia)
  - Mattermost-ilmoitus on tällä hetkellä pois (vain log) — voidaan
    lisätä jos havaitaan piikkejä
- [x] Auto-create-koodi (entinen `main.rs:resolve_or_create_user`)
      poistettu kokonaan; tilalla `db::resolve_user`, joka palauttaa
      pelkän lookupin (`Option<(tenant_id, user_id)>`).

### Vaihe 2 — palauta legitiimisten käyttäjien tunnistus

> Vaihe 1 esti auto-createn. Sen jälkeen email-palvelu ei tunnista
> ketään, koska email-puolen `users`-taulu on käytännössä tyhjä —
> rekisteröinti tapahtuu api-puolen erillisessä DB:ssä eri schemalla.
> Tämä vaihe palauttaa legitiimien käyttäjien prosessoinnin **ilman**
> että auto-create palaa.

**Arkkitehtuurinen havainto (selvitys 2026-04-29):**

| | email-puoli | api-puoli |
|---|---|---|
| DB | `grooveserve_email_*` | `grooveserve_api_*` (eri DB) |
| `users` | `(id, tenant_id, email, name, role)` | `(id, name, password_hash)` |
| Tenant ↔ user | `users.tenant_id` (1-1) | `tenant_users` (M-N) |
| Email ↔ user | suoraan `users.email` | `user_emails(user_id, email, ...)` |

`#26 design.md §6.1` linjaa että MVP:ssä email + api ovat **sama
binääri** ja jakavat DB:n. Tätä yhdistämistä ei ole vielä toteutettu.
`ops::find_user_by_email` on jo olemassa api-puolella ja palauttaa
juuri sen mitä tarvitaan (`user_id`, `tenant_id`, `role`,
`tenant_status`, `membership_status`).

**Toteutusvaihtoehdot:**

| | Lähestymistapa | Kustannus | Soveltuvuus |
|---|---|---|---|
| **A** | Yhdistä yhdeksi binääriksi (#26 §6.1), email-taulut → api-schemaan | Erittäin iso | Oma epic |
| **B** | Erilliset binäärit, jaettu DB. Email lukee `user_emails`/`tenant_users` | Iso (FK-migraatio) | Oma epic |
| **C** | Erilliset DB:t, api kirjoittaa rekisteröinnissä myös email-DB:hen | Pieni | **Tämän taskin scope** |
| **D** | Email kysyy api:lta REST:llä per viesti | Kohtalainen + lisää failure modeja | Hylätty |

**Päätös 2026-04-29:** mennään C:llä. Api-puolen rekisteröinti- ja
kutsuflow täydentää myös email-DB:n `tenants`/`users`-taulut. Email-puoli
ei muutu rakenteellisesti — sen `db::resolve_user` toimii sellaisenaan
kun rivit ilmestyvät tauluun. Yhdistetty binääri (A/B) jätetään
omaksi epicikseen.

- [ ] Api-puolen `ops::complete_registration` luo myös email-DB:hen
      `tenants`-rivin (jos ei vielä) ja `users`-rivin samoilla `id`:llä.
- [ ] Api-puolen `ops::accept_invitation` (kutsuttu käyttäjä) tekee
      saman.
- [ ] Yhteensovittaminen: email-puolen `users.id` = api-puolen
      `users.id` (sama BIGSERIAL). Arvioi onko tämä saavutettavissa
      `OVERRIDING SYSTEM VALUE` -INSERT:llä, vai onko parempi mapata
      api↔email käyttäjäID erikseen.
- [ ] Käytä yhteistä DB-yhteyttä api-binäärillä (env: `EMAIL_DATABASE_URL`).
- [ ] Päivitä `gsdev`/playwright-testit varmistamaan että rekisteröinti
      → email round-trip toimii ilman manuaalista DB-täydennystä.

### Vaihe 3 — opt-in verified domains (kontrolloitu auto-create)

- [ ] Tenantille voi asettaa `verified_domains TEXT[]` (esim.
      `['acme.com']`). Pääkäyttäjä hallitsee admin-portaalista.
- [ ] Tenantilla on `auto_create_from_verified_domain BOOLEAN` -flagi
      (oletus `false`).
- [ ] Kun **kaikki** ehdot täyttyvät — flagi päällä, lähettäjän domain
      verified-listassa, tenantilla aktiivinen admin-jäsenyys, viesti
      menee spam-tarkistuksesta läpi — luodaan uusi `users`/
      `tenant_users`-rivi pendingiksi.
- [ ] Audit-rivi `audit_events`-tauluun (`auto_created_from_email`).
- [ ] Ilman ehtoja: vaiheen 1 skip-polku.

## Quick Test

Vaiheen 1 jälkeen:

```bash
# Random sähköposti
curl ... # tai Roundcube
# → DB:ssä ei uutta tenantia eikä useria
psql grooveserve_email_main_main -c "SELECT count(*) FROM tenants;"

# Web-puolen rekisteröity käyttäjä lähettää
# → vasta kun #26 on toteutettu, agentti tunnistaa heidät
```

## Out of scope

- Email- ja api-binäärien yhdistäminen yhdeksi prosessiksi
  (`#26 design.md §6.1`, lähestymistapa A/B yllä) — oma epic.
- Käyttäjän rooli- ja hyväksyjähierarkia → #41.

## Notes

Tämä paljastui kun Roundcube round-trip -demo toimi vaikka tenanttia ei
ollut rekisteröity webissä — agentti vain auto-loi haamutilin ja
prosessoi viestin sitä vasten. Ratkaiseva keskustelu jossa havaittiin:
"järjestelmä ei saa luoda uusi käyttäjätunnuksia automaattisest pelkän
sähköpostin perusteella" (jari, 2026-04-29).

### Vaiheen 1 toteutus (2026-04-29)

- `services/email/src/db.rs`: lisätty `resolve_user` (case-insensitive
  lookup `users`-taulusta, palauttaa `Option<(tenant_id, user_id)>`).
- `services/email/src/main.rs`: poistettu `resolve_or_create_user`,
  lisätty `skip_unknown_sender`. Assistant-tilin claim-polku tunnistaa
  lähettäjän ennen `claim_with_thread`-kutsua; jos `None`, viesti
  siirtyy `Skipped`-kansioon ja `email_processing` saa
  `status = 'unknown_sender'`. Retry-polku ei enää luo identiteettiä —
  jos käyttäjä on poistettu retryn aikana, viesti merkitään failediksi.
- `services/email/tests/unknown_sender.rs`: 7 sqlx-integraatiotestiä
  jotka kattavat mm. `unknown_sender_does_not_create_tenant`,
  `unknown_sender_does_not_create_user`, `case_insensitive_email_match`,
  `known_sender_can_be_claimed_with_thread`.
- Olemassaolevat `claim_with_thread.rs` ja `tools_snapshot.rs` -testit
  pysyvät vihreinä.

`gs_email_cli` (kehitystyökalu) säilyttää oman `resolve_or_create_user`
-funktionsa: se on tarkoituksellinen kehittäjäpolku, jota kutsutaan vain
manuaalisesti CLI-komennoilla — ei vastaanotetuilla viesteillä.

### Jatko (2026-04-29)

Vaihe 1 on commitoitu (`7891b7e`) ja worktree
`ei-auto-create-tuntemattomista-lahettajista` voidaan mergeä mainiin.
Vaiheet 2 ja 3 tehdään tämän saman issuen (#43) alla **uudessa
worktreessa**, koska niiden työ koskee suurelta osin api-puolta
(rekisteröinti-/kutsuflow + admin-näkymä) eikä enää pelkkää
email-palvelua.

### Sulkeminen 2026-04-30 (A4b — #56 Phase 14)

**Status: obsolete.** Vaiheet 2 ja 3 oli alunperin määritelty
"option C":n päälle: api-puolen rekisteröinti kirjoittaa myös
email-puolen erillisen `users`-taulun. Tämä lähestymistapa korvautui
**#56 Phase 1:n yhdistetyllä skeemalla** (A2 + A3 + A4):
sähköposti- ja web-puoli jakavat nyt saman `users` + `tenant_users` +
`user_emails` -triplen, joten "kahden DB:n synkronointi" ei ole
enää relevantti ongelma. Sähköpostin lähettäjän tunnistus tapahtuu
yhden ja saman `ops::user::find_user_by_email`-funktion kautta sekä
ingest-puolelta (uusi `ops::ingest::process_message`,
`A4b`) että web-loginista — ei enää tarvetta kahdennettuun
kirjoitukseen.

Vaihe 1 (auto-createin pysäytys) jää voimaan koodissa: ingest skippaa
tuntemattomat lähettäjät `ProcessOutcome::UnknownSender`-haarassa
ilman tenant/user-luontia. `crates/server/src/ingest/runner.rs`
`skip_unknown_sender` ja `ProcessMessageInput`/`ProcessOutcome`
ovat nyt pinta jonka päälle Phase 2:n (#26 toteutus) ja D-aallon
(#58–#62) instrumentointi rakentuvat.

Vaihe 3 (`auto_create_from_verified_domain`-flagi) pysyy ehdotuksena.
Sitä ei toteuteta ennen kuin tarve näkyy todellisilla asiakkailla;
kun se aikanaan tehdään, se kuuluu `#26` Phase 2:n laajennukseen,
ei tähän bugi-issueen.
