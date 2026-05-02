---
created: 2026-04-25
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#26", "#96", "#97", "#98"]
labels: [web, auth]
closed: 2026-05-01
commits:
  - hash: 7ad6338
    summary: "feat(server): onboarding flow with recovery path (#6) [squash of c35bb70..5c31275]"
---

# 6. Onboarding-flow — henkilötietojen kerääminen

_Source: käyttäjähallinta_

## Description

Rekisteröitymisen jälkeen järjestelmä lähettää käyttäjälle sähköpostin jossa on linkki salattuun lomakesivuun. Sivulla kysytään taustatiedot (kotiosoite, syntymäaika, puhelin, työnantaja). Arkaluonteiset tiedot kerätään HTTPS:n yli, ei sähköpostissa.

## Scope

- [x] Käyttäjäkohtaisen salatun linkin generointi (`auth_tokens.purpose='onboarding'`, 7 vrk TTL, single-use)
- [x] Lomakesivu henkilötiedoille (`/onboarding?token=…` GET + POST + `/onboarding/done`)
- [x] Tietojen tallennus tietokantaan (laajennus `user_profiles`-tauluun: `date_of_birth`, `phone_number`, `employer_name`; `home_address` jo olemassa)
- [x] Vahvistusviesti käyttäjälle tietojen vastaanoton jälkeen (yhdistetty welcome+onboarding-mail `assistant@`-tililtä `set_password.rs::submit` ja `invite.rs::accept_submit` -kohdista)

## Toteutus

### Pohja (commit c35bb70)
- Migraatio `024_onboarding_data.sql` (renumeroitu mainin 023:n vuoksi)
- `crates/ops/src/onboarding.rs` — `create_onboarding_token` / `inspect_onboarding_token` / `submit_onboarding`
- `crates/server/src/http/routes/onboarding.rs` — GET form + POST submit + done-page + `send_onboarding_email`-helper
- Wiring: `set_password.rs::submit` ja `invite.rs::accept_submit` mintteittävät tokenin ja lähettävät yhdistetyn welcome+onboarding-mailin (vanha `send_welcome_email` poistettu)
- i18n-stringit (en/fi/sv): onboarding-form-pinta + email-pinta
- AGENTS.md-päivitys (crates/ops, crates/server)

### `/llm-review` FIX bundle (commit d07baf0) — 5 mekaanista korjausta
- **C1** GET-render ei enää palauta DOB/address/phone/employer prefillinä — vain display-name + email. `inspect_onboarding_token` palauttaa pelkät turvalliset kentät; lukko `inspect_does_not_leak_pii_after_submit`-testillä.
- **C5** Sähköposti-Mailbox rakennetaan `Mailbox::new`-metodilla, ei `format!().parse()`-pattern:lla. Sovellettu 4 mailerissa (onboarding, register, invite, resend).
- **M2** Puhelinvalidaattori sallii `.`-erottimen (`040.123.4567`) ja nostaa minimi-digit-rajan 5→7 jotta `+++++12345`-tyyppinen täyte hylätään.
- **M4** DOB-parse-virheessä raw-string echotetaan takaisin lomakkeelle (käyttäjä näkee mitä kirjoitti).
- **M12** `Referrer-Policy: no-referrer` lisätty `Cache-Control: no-store`:n viereen TokenPage/Admin-pinnoille.

### Recovery-path round-2 review FIX bundle (commit 5c31275) — 10 löydöstä

`/llm-review` 045d621-commiille tuotti 26 löydöstä → `/assess-findings`
rajasi 10 FIX + 16 DROP. Toteutus:

- **CR1** Lock-order deadlock: pre-fetch user_id ennen tx, sitten `SELECT
  users FOR UPDATE` ennen `auth_tokens FOR UPDATE`. Molemmat ops nyt
  lock users → auth_tokens. Concurrent submit+resend ei aiheuta 40P01.
- **CR2** `OpError::AlreadyCompleted` + `OpError::RateLimited
  { retry_after_secs: i64 }` -variantit; route mappaa eri i18n-stringeihin
  ja HTTP-statuksiin (409 vs 429).
- **CR3 + MR1** Throttle WHERE `used_at IS NULL` (used tokens eivät enää
  blokkaa retryä); uusi `abandon_onboarding_token`-helper jonka route
  kutsuu SMTP-virheessä → orphan markataan used → throttle ohittaa →
  käyttäjä voi yrittää uudelleen 5 min kuluessa.
- **CR4** Migraatio 024 backfill: `UPDATE user_profiles SET
  onboarding_completed_at = COALESCE(updated_at, NOW())` neljälle
  onboarding-kentälle täytetyt rivit.
- **CR5** Banner fail-open DB-virheessä (admin.rs + me.rs); log-taso
  `error` koska DB-virhe on todellinen ongelma.
- **MR2** `resend_status_response` ottaa `&ResolvedSession` ja valitsee
  back-linkin: admin → `/admin`, muut → `/me`. Uusi `back_to_my_page`-
  i18n-stringi.
- **MR3** Route-doc-commentin URL korjattu `POST /onboarding/resend`-
  muotoon.
- **MR6** Audit-rivi `"onboarding_link_resent"` `resend_onboarding_token`-
  txin sisällä — peilattu `submit_onboarding`-pintaa.
- **NM4** `/onboarding/done?lang=fi` redirektista — thank-you-sivu
  käyttäjän kielellä, ei aina English.
- **DR3** `gsadmin onboarding-resend --force` -lippu: ohittaa throttle:n
  JA nullaa `onboarding_completed_at`:n niin että jo onboardattu käyttäjä
  voi täyttää lomakkeen uudelleen (muutto, työnantajan vaihto, typo).

**DROP-listalla** (per `/assess-findings`): MR4 (PRG infrastructure),
MR5 (clock skew co-located DB), MR7 (defense-in-depth), MR8 (Rust-Python
drift — Mailgun-rate-limited; track for production), NM1-NM10 (nits).
Raportti: `history/review-onboarding-recovery-path.md`.

### Recovery path (commit 045d621) — C2/M6/D6 yhteenpaketoituna
- Migraatio `025_onboarding_completed_at.sql`: `user_profiles.onboarding_completed_at TIMESTAMPTZ` (kova "tehty"-signaali bannerille; renumeroitu 024:stä).
- `submit_onboarding` setaa kentän `NOW()`:lle UPSERT:in yhteydessä.
- `resend_onboarding_token` ops-funktio: throttle 5 min (`MIN_RESEND_INTERVAL_SECS`), refuusoi `Conflict`-virheellä jos onboarding tehty tai mintti äsken.
- `is_onboarding_completed` helper /admin- ja /me-bannerin tarkistukseen.
- Reitti `POST /onboarding/resend` (session-auth, CSRF-guarded, me_router:ssa).
- /admin- ja /me-dashboardit renderöivät bannerin "Viimeistele profiilisi tiedot [Lähetä onboarding-linkki sähköpostiini]" jos `onboarding_completed_at IS NULL`.
- `gsadmin onboarding-resend <email>` Python-CLI out-of-band-recoveryyn (peilattu `gsadmin password-reset`-pinnasta).

### Testit
- 18 ops-integraatiotestiä `crates/ops/src/onboarding.rs`:ssä pohjavaiheessa
- +5 testiä recovery-path:lle (completion-flip, throttle-reject, throttle-pass-after-window, completion-reject, unknown-user)
- +1 dot-phone-test, päivitetty inspect-PII-testi
- +3 round-2-FIX-testiä: throttle-ignores-used-tokens, abandon-cleanup,
  concurrent-submit/resend-no-deadlock
- 2 testiä uudelleennimetty (returns_rate_limited, returns_already_completed)
- `cargo test --workspace` **571 testiä vihreänä** (oli 562 pohjavaiheessa)

## Päätökset (#56 decision-log 2026-05-01)

- Kentät = full_name + home_address + date_of_birth + phone_number + employer_name; **IBAN ulos** (Procountor/Netvisor)
- Encryption MVP = pelkkä levypohjainen at-rest; kolumni-encryption **SPIN-OFF #96**
- DB-malli = laajennus `user_profiles`-tauluun (kaikki user-data samassa paikassa)
- Agent-permissioning = kaikki muokattavissa MVP:ssä, post-pilot-kategorisaatio **SPIN-OFF #97**
- Sähköposti = yksi yhdistetty welcome+onboarding-viesti

## /llm-review tulos

`history/review-onboarding-flow.md` — 4 reviewerie × 2 kierrosta. 24 löydöstä:
- **5 FIX** (C1, C5, M2, M4, M12) — landattu commit d07baf0
- **3 SPIN-OFF**:
  - C2 + M6 + D6 yhdistettynä → toteutettu samassa worktreessä commit 045d621
  - M9 (validation-virheiden i18n) → **issue #98** post-POC
- **17 DROP** — sis. C3 (race RARE/no-reads), C4 (INCORRECT — `validate::name` accept-test todennettu), C6 (RARE), M1 + M8 (covered by #63), M3 (URL-safe base64 today), M5/M10/M11 (defense-in-depth), M7 (TTL owner-decided), D1-D6
- **C4 INCORRECT**: 3 reviewerie luuli `validate::name`:n hylkäävän `A & B Oy`-tyyppiset yritysnimet — todennetussa ajossa kaikki (sis. `Virtanen-Korhonen`, `O'Brien`, `3 Step IT Oy`) menivät läpi.
