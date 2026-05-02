---
created: 2026-04-26
updated: 2026-05-01
type: epic
owner: jari
status: in-progress
priority: high
related: ["#4", "#6", "#22"]
commits:
  - hash: 2cdbda2
    summary: "feat(api): multi-tenant data model and ops layer (phase 1)"
  - hash: cc10fb9
    summary: "feat(api): set-password page replaces broken /verify flow"
  - hash: 8978635
    summary: "fix(api): use ON CONFLICT to avoid pre-check races on email and slug"
  - hash: 1f95da9
    summary: "fix(api): proper email validation and mailer error propagation"
  - hash: ba7c844
    summary: "fix(api): /health probes the database"
  - hash: 33e6514
    summary: "fix(api): verify SMTP credentials at startup"
  - hash: 414fb22
    summary: "feat(api): resend verification endpoint"
  - hash: fcb5ae6
    summary: "feat(api): Cloudflare Turnstile + per-IP rate limit on public endpoints"
  - hash: ccf2868
    summary: "feat(email): add tenant_id/user_id to conversations (phase 2 prep)"
  - hash: 2bd14a2
    summary: "feat(api): web auth foundation — sessions, CSRF, login (phase 2)"
  - hash: 371803f
    summary: "feat(api): admin dashboard + user list"
  - hash: 1277ed1
    summary: "feat(api): user invitation flow"
  - hash: 452dedc
    summary: "feat(api): role change + disable/enable with last-admin protection"
  - hash: 6a84065
    summary: "feat(api): password reset flow (out-of-band token issuance)"
  - hash: a8c12ca
    summary: "feat(gsadmin): password-reset <email> issues a one-time reset link"
  - hash: d1511e0
    summary: "feat(server): multi-tenant Phase 3 — agent admin tools + pending flow + #67 policy"
  - hash: 7645f63
    summary: "fix(server): apply LLM-review findings to multi-tenant Phase 3 (#26 / #67)"
  - hash: 22a998e
    summary: "feat(ops): tx-aware refactor + session status filter + pending sweeper"
  - hash: 00e818b
    summary: "fix(ops): apply round-3 LLM-review findings (#26 / #67)"
---

# E26. Multi-tenant käyttäjähallinta

## Goal

Suunnitella ja toteuttaa monitenanttiinen käyttäjähallintajärjestelmä Grooveservelle. Yritys (tenant) on asiakasyksikkö. Jokaisella yrityksellä on käyttäjiä ja pääkäyttäjä. Hallinta tapahtuu sekä web-portaalista että AI-agentin kautta yhtenäisellä rajapinnalla (Unified Tool Surface).

## Issues

- **#3** Kotisivut ja rekisteröityminen (open)
- **#4** Käyttäjän tunnistaminen agenttisessa loopissa (open)
- **#6** Onboarding-flow — henkilötietojen kerääminen (open)
- **#22** Käyttäjähallinta — pääkäyttäjän ylläpitonäkymä + pending-operaatiot (open)
- **#27** Session cleanup — vanhentuneiden sessioiden siivous (open)
- **#28** Email alias / plus-addressing -käsittely (open)
- **#29** Agent tool_result -virheiden serialisointi (open)
- **#30** Single binary -vikaantumismalli ja supervision (open)
- **#100** ✓ `get_user_context` + profile-snapshot helpers A3-skeemalle (fixed 2026-05-01)

## Phases

### Phase 1: Tietomalli ja API
- [x] Tenant/Company/TenantUsers-taulut ja migraatiot (users + tenant_users -rakenne)
- [x] User_emails, invitations, sessions -taulut (hashatut tokenit)
- [x] Audit_events -taulu
- [x] OpContext-pohjainen operaatiokerros (Unified Tool Surface)
- [x] Nykyinen rekisteröinti integroitu uuteen malliin (services/api/)
- [x] Integraatiotestit ops-kerrokselle (`#[sqlx::test]`, 13/13 vihreitä)

### Phase 1.5: Review-pohjaiset korjaukset (2026-04-27)
LLM-review (Gemini, GPT-5.5, Claude Opus, DeepSeek) tunnisti 10 real-impact -löydöstä Phase 1:n jälkeen. Kaikki korjattu:
- [x] Set-password-sivu korvaa rikkinäisen verify-flowin (cc10fb9, ratkaisee #5+#1)
- [x] ON CONFLICT email/slug-races sijaan SELECT-then-INSERT (8978635, ratkaisee #7)
- [x] `lettre::Address`-pohjainen email-validointi + `?` propagaatio mailer-paniikkien sijaan (1f95da9, ratkaisee M.6+M.8)
- [x] `/health` tekee `SELECT 1` -DB-probe (ba7c844, ratkaisee M.9)
- [x] SMTP-creds verifioidaan startupissa, fail-fast (33e6514, ratkaisee M.14)
- [x] `POST /api/register/resend` — enumeration-safe resend-endpoint (414fb22, ratkaisee #10)
- [x] Cloudflare Turnstile + tower_governor per-IP rate limit (fcb5ae6, ratkaisee #12)
- [x] `services/email/migrations/009`: conversations.tenant_id/user_id Phase 2 -valmius (ccf2868, ratkaisee M.3)

Ei korjattu (kosmiset / cosmic-ray / kiistanalaiset): #2 verify_registration multi-row hazard (suojattu MVP-invariantilla), #3 auth_tokens tenant scoping (ei tarpeen — invitations on oma taulu), #6 OpContext::system() footgun (kuollut koodi nyt), #11 audit_events.tenant_id NOT NULL (ei käytössä), #13 audit actor=NULL (näkyvyys vain), #14 redundant idx_tenants_slug, M.1 raw-token-test merkityksetön, M.2 sessions composite FK, M.4-5 token hash CHECK, M.7 expires_at indeksit, M.10 SMTP-fallback, M.11 email_normalize duplicate-pattern, M.12-15-16-17-18 muut. Listattu `history/review-phase1-multi-tenant.md`.

### Phase 2: Autentikaatio ja tunnistaminen + admin-CRUD (2026-04-28)

Phase 2 ja Phase 3:n web-CRUD yhdistetty yhdeksi toimitettavaksi
kokonaisuudeksi: admin saa heti käytettävän portaalin yhdellä iteraatiolla.

- [x] Web-autentikaatio: `ops::auth::login(email, password)` + `GET/POST /login` (form-based, ei JSON-API:a). `ops::password::verify` valmiina, argon2-timing pidetty tasaisena `LazyLock`-dummy-hashilla tuntemattomille emailille.
- [x] Sessioiden hallinta: hashatut tokenit `sessions`-taulussa, HTTP-only Secure SameSite=Lax cookie, `attach_session` middleware injektoi `ResolvedSession` requestin Extensioniin
- [x] CSRF-token muuttaviin pyyntöihin: `csrf_setup` middleware (double-submit, SameSite=Strict cookie + hidden form field, constant-time vertailu)
- [x] Admin-portaali: `/admin` dashboard, `/admin/users` lista (#22)
- [x] Käyttäjien kutsuminen: `ops::invitation::invite_user/inspect/accept`, `/admin/users/invite` (form), `/accept-invitation?token=...` (form)
- [x] Roolin vaihto + deaktivointi/aktivointi: `ops::user::update_role/disable_user/enable_user`, `POST /admin/users/:id/{role,disable,enable}`
- [x] Last-admin -suojaus: CTE + `FOR UPDATE` -lukot tenant_users-rivien päällä, ei vain count-aggregaatissa (Postgres torjuu `FOR UPDATE` aggregaateissa)

**Salasanan palautus (gsadmin-kanava, 2026-04-28):**
- [x] `ops::password_reset` — create/inspect/consume, kertakäyttöinen 24h token, re-issue mitätöi vanhan, konsumointi tuhoaa kaikki sessiot
- [x] `GET/POST /reset-password?token=...` — sama email-scanner-safe pattern kuin /set-password
- [x] `gsadmin password-reset <email>` — generoi auth_tokens-rivin ja tulostaa reset-URL:n operaattorille (ei lähetä sähköpostia, operaattori välittää linkin valitsemallaan kanavalla)

**Lykätty (kosmiset / tarvittaessa lisätään):**
- [ ] "Forgot password?" -lomake login-sivulla (julkinen flow, vaatii Turnstile + per-email rate limit) — operaattorivetoinen reset toimii nyt
- [ ] Login rate limiting per IP + per email — security hardening, erilliseksi issueksi
- [ ] Käyttäjän tunnistaminen sähköpostiosoitteen perusteella (#4) — käytännössä jo tehty email-palvelussa (legacy `users`-taulu services/email DB:ssä); Phase 6.1 db-konsolidoinnin yhteydessä yhdistetään `find_user_by_email` ops-kerrokseen

### Phase 3: Agenttipohjainen käyttäjähallinta + onboarding
- [x] Agenttipohjainen käyttäjähallinta — tool_use:t `invite_user`,
  `enable_user`, `disable_user`, `update_user_role`. Sender resolves
  to admin via `tenant_users`-lookup; muutokset menevät kanonisesti
  `pending_admin_actions`-vahvistuksen kautta (#67 v1.1).
- [x] Onboarding-flow (#6)
- [x] Pending-tila admin-operaatioille email-kanavasta (#22) —
  `pending_admin_actions`-taulu (migraatio 026), ops-funktiot
  `create_pending`/`inspect_pending`/`confirm_pending`/`cancel_pending`,
  web-vahvistuspolku `/admin/pending/:token` + `.../confirm`. **Kaikki**
  neljä admin-toolia menevät pending:in läpi (LLM-review-vahvistettu —
  prompt-injection torjunta `invite_user`/`enable_user`-poluille).

### Phase 4: Laajennukset
- [ ] Tilausten hallinta
- [ ] Laskutustiedot
- [ ] Hyväksyntäkierron konfigurointi
- [ ] Multi-tenant jäsenyys (sama käyttäjä useassa tenantissa)

## Notes

- Suunnitteludokumentti: [design.md](design.md) (v2, LLM-review integroitu)
- Arkkitehtuuriperiaate "Unified Tool Surface" dokumentoitu CLAUDE.md:hen
- Tietomalli: users + tenant_users (many-to-many -valmius, MVP:ssä many-to-one)
- MVP-painotteinen: ei ylisuunnittelua, mutta laajennettava rakenne

### Phase 1 + 1.5 toteutus (2026-04-27)

**Tietomalli (services/api/migrations/):**
- 002 dropaa vanhan users-taulun, 003-008 luo uuden skeeman: tenants, users, tenant_users, user_emails, invitations, sessions, auth_tokens, audit_events
- Migraatioiden ajo `sqlx::migrate!()`-makrolla
- `services/email/migrations/009`: conversations.tenant_id/user_id (ilman FK:ta — schemat eri DB:issä kunnes Phase 6.1 -konsolidointi)

**Operaatiokerros (services/api/src/ops/):**
- `context.rs`: `OpContext { actor_user_id, tenant_id, role, channel }`, `UserRole`, `Channel`, `require_admin()`
- `error.rs`: `OpError` (NotFound, AlreadyExists, Forbidden, InvalidInput, InvalidToken, LastAdminProtection, Database)
- `token.rs`: `generate()` → 32 tavua /dev/urandom + SHA-256 hash, `hash_raw(&str)` lookup-vertailuun
- `password.rs`: argon2id `hash(&str)` + `verify(&str, &str)`, MIN_PASSWORD_LEN=10
- `email.rs`: `normalize(&str)` lettre::Address-pohjainen validointi, vaatii pisteen domainissa
- `audit.rs`: `record(executor, ctx, ...)` + `record_with_email(...)` työskentelevät transaktion sisällä `Executor`-bound:n ansiosta
- `tenant.rs`: `create_tenant`, `inspect_registration_token`, `complete_registration`, `resend_registration_verification`. ON CONFLICT email/slug-races sijaan SELECT-then-INSERT.
- `user.rs`: `find_user_by_email` joka palauttaa user + tenant + role + status (käytetään email-agentin tunnistuksessa Phase 2:ssa)

**HTTP-kerros (services/api/src/):**
- `routes/register.rs`: `POST /api/register` (Turnstile + rate limit)
- `routes/resend.rs`: `POST /api/register/resend` (Turnstile + rate limit, enumeration-safe)
- `routes/set_password.rs`: `GET /set-password` HTML-form (token NOT consumed → email-skannerit OK), `POST /set-password` kuluttaa tokenin + asettaa salasanan
- `turnstile.rs`: Cloudflare siteverify (TURNSTILE_SECRET puuttuessa skipataan + warning)
- `main.rs`: `verify_smtp` startupissa (SMTP_SKIP_STARTUP_CHECK=1 ohittaa), `/health` tekee SELECT 1

**Testit:** 24/24 vihreää (#[sqlx::test], fresh DB per test). Kattaa happy pathin + reunatapaukset (duplikaatti email/slug, käytetty/vanhentunut/tuntematon token, weak password, idle resend, post-active resend).

**Phase 2 valmistelu:**
- `find_user_by_email` valmis email-agentin sender-identifikaatioon — agentti rakentaa `OpContext { channel: EmailAgent }` kun käyttäjä löytyy
- `password::verify` valmis loginiin
- `auth_tokens.purpose='password_reset'` valmis salasanan palautukseen
- `conversations.tenant_id/user_id` valmis Phase 2:n agent-loop -tallennukseen

### Phase 2 toteutus (2026-04-28)

**Web-kerros (services/api/src/):**
- `web.rs`: jaettu HTML-shell (Page, PageKind { Plain, Admin }), Pico-tyylinen inline CSS, csrf hidden input -helper, `Cache-Control: no-store` admin-sivuille
- `middleware.rs`:
  - `attach_session` — cookie → hash → `sessions`-lookup → `ResolvedSession` requestin Extensioniin
  - `require_session` — 303 redirect → `/login` jos ei sessiota
  - `csrf_setup` — yhdistetty middleware: ensimmäisellä kerralla luo csrf-cookien JA injektoi `ExpectedCsrf`-arvon samalle requestille (jotta GET-handlerin renderöimä lomake voi sisällyttää tokenin), seuraavilla pyynnöillä lukee cookien
  - `csrf_ok(expected, submitted)` — constant-time vertailu pituuden + bittien suhteen

**Operaatiokerros (services/api/src/ops/):**
- `auth.rs`: `login(db, email, password, ttl)` — Argon2-timing tasainen lazy-`DUMMY_HASH`:lla, `LoginFailure { InvalidCredentials, AccountDisabled }`. Audit-rivi `login` / `login_blocked`.
- `session.rs`: `create`, `resolve`, `destroy`, `destroy_all_for_user`. Vanhentunut sessio siivotaan `resolve`-pyynnöllä.
- `invitation.rs`: `invite_user(ctx, input)` luo `users` (invited) + `user_emails` + `tenant_users` (invited) + `invitations` + audit. `inspect_invitation` GET-näkymälle, `accept_invitation` POST:lle (kuluttaa tokenin, asettaa salasanan, aktivoi). Admin-roolin eskalointi torjutaan ops-kerroksessa.
- `user.rs`: `list_users(ctx)` admin-suojattu. `update_role`, `disable_user`, `enable_user` — last-admin -suojaus CTE:lla joka lukitsee muut admin-rivit `FOR UPDATE`:lla (Postgres torjuu `FOR UPDATE` aggregaateissa). `disable_user` poistaa target-käyttäjän aktiiviset sessiot.

**HTTP-kerros (services/api/src/routes/):**
- `auth.rs`: `GET /login` (form), `POST /login` (303 → /admin), `POST /logout`. `Set-Cookie: session=...` HTTP-only Secure SameSite=Lax.
- `admin.rs`: `GET /admin` dashboard, `GET /admin/users` taulukko (rooli-select, deaktivoi/aktivoi-painikkeet). Inline action-formit hyödyntävät `csrf` cookieta.
- `invite.rs`: `/admin/users/invite` (form), `/accept-invitation?token=...` (kuluttaa tokenin set-password-tyylillä). Sähköposti reusettaa `verification_html`-templaten.
- `user_actions.rs`: `POST /admin/users/:id/{role,disable,enable}`, kaikki CSRF-suojattuja, mappaa OpError → HTTP-virhesivu.

**Routing (main.rs):**
- `registration_router` (JSON, /api/register*) — säilyttää oman per-IP rate limiterin
- `form_router` — kaikki form-pohjaiset sivut (auth + set-password + accept-invitation), session + csrf middleware
- `admin_router` — kytketty form_routeriin, lisäksi `require_session`

**Testit:** 74/74 vihreää (`#[sqlx::test]`, fresh DB per test). Phase 2 lisäsi 50 uutta testiä:
- ops::session: 5 (create/resolve/expire/destroy/destroy_all)
- ops::auth: 6 (login happy path, case-insensitive email, väärä salasana, tuntematon email, ei salasanaa asetettu, disabled membership)
- ops::user: 11 (list_users + update_role + disable + enable + last-admin + cross-tenant)
- ops::invitation: 9 (invite + inspect + accept + duplikaatit + admin-eskalaatio + expired/single-use/weak password)
- ops::password_reset: 11 (create/inspect/consume + re-issue mitätöi vanhan + token-collision purpose-tarkistus + sessions-cleanup + unknown user/email)
- middleware csrf-tests: 5
- web-tests: 3 (escape, admin pages no-store, plain pages cacheable)

**Salasanan palautus (gsadmin-kanava):**
- `services/api/src/ops/password_reset.rs`: `create_reset_token`, `create_reset_token_by_email` (email→user_id resolution + create), `inspect_reset_token`, `consume_reset_token`. Re-issuance mitätöi aiemman unused-tokenin atomisesti samassa transaktiossa kuin uuden luonti. Konsumointi tuhoaa kaikki käyttäjän sessiot.
- `services/api/src/routes/reset_password.rs`: `GET /reset-password?token=...` validoi ilman kulutusta, `POST` kuluttaa.
- `tools/admin/gsadmin/cmd_password_reset.py`: generoi 32-byte tokenin (samalla muodolla kuin services/api/src/ops/token.rs), SHA-256 hashaa, INSERT INTO auth_tokens psycopg-yhteyden kautta. Tulostaa reset-URLin operaattorille — sähköpostia ei lähetetä, operaattori välittää linkin valitsemallaan kanavalla. `GSADMIN_DIRECT_DB_URL` ohittaa SSH-tunnelin paikallisessa kehityksessä.

## Testaus paikallisesti gsdev:llä

`gsdev`-orkestrointityökalu (#33) hoitaa per-worktree DB:n, mailpitin ja env-tiedostot. Tämä ohje olettaa, että haara on yhdistetty mainiin (jossa gsdev elää) — ennen sitä, ks. *Pre-merge-pikatesti* alla.

### 1. Setup

```bash
# Worktreen juuressa (workmux ajaa tämän automaattisesti post_create-hookissa,
# mutta voi ajaa myös käsin uudelleen — komento on idempotentti):
gsdev instance ensure
```

`ensure` hoitaa:
- Per-instance DB (`grooveserve_api_<slug>`, `grooveserve_email_<slug>`)
- Per-instance Mailpit-kontti
- `services/api/.env.local` ja `services/email/.env.local` rendöröidään `operations/dev/env.*.template`-pohjista
- `direnv allow` services/api ja sites/www -hakemistoissa

`.workmux.yaml`-paneelit käynnistävät:
- `cargo watch services/api` → kuuntelee `LISTEN_ADDR`-portissa
- `pnpm dev sites/www` → kuuntelee `VITE_PORT`-portissa
- (Optional) `cargo watch services/email` jos käytetään `email`-layoutia

```bash
gsdev list                          # näytä instanssin slug + portit
gsdev status                        # health probe (DB:t, mailpit, env-files)
gsdev mail open                     # avaa mailpitin web-UI selaimessa
```

### 2. Skenaariot

**Huom env.api.template**: `BASE_URL=http://localhost:{{www_port}}` osoittaa www-sivustoon, mutta uudet sivut (`/login`, `/set-password`, `/admin`, `/reset-password`, `/accept-invitation`) elävät API:lla. Tarkista mailpitistä saamasi URL — jos portti on www_port mutta sivu antaa 404, vaihda portti `api_port`:iin (näkyy `gsdev list`-komennolla). Pysyvä korjaus: `operations/dev/env.api.template` → `BASE_URL=http://127.0.0.1:{{api_port}}` tai Vite-proxy konfigurointi sites/www:hin.

**1. Yrityksen rekisteröityminen → admin-portaaliin**

```bash
# Hae portit
gsdev list
# käytä rivillä näkyvää api_port-arvoa (esim. 13551), www_port-arvoa, mailpit_ui-arvoa

curl -i -X POST -H "Content-Type: application/json" -H "X-Forwarded-For: 127.0.0.1" \
  -d '{"company_name":"Firma Oy","contact_name":"Anna Admin","email":"anna@firma.fi"}' \
  http://127.0.0.1:<api_port>/api/register
```

- Avaa mailpit (`gsdev mail open`). Tulostuu "Vahvista Grooveserve-tilisi"-viesti
- Klikkaa linkkiä (vaihda portti tarvittaessa, ks. yllä)
- → `/set-password` HTML-lomake. Aseta vahva salasana (≥10 merkkiä), submit
- → "Tili aktivoitu" + tervetulosähköposti mailpittiin
- Kirjaudu `/login`:iin samalla email/salasanalla → 303 `/admin`

Verifikaatio:
- Cookie `session=...` HttpOnly SameSite=Lax (Secure pois jos COOKIE_INSECURE=1)
- DB-tila: `tenants.status='active'`, `tenant_users.status='active'`, `user_emails.verified=true`, `users.password_hash` argon2id-muodossa
- Audit-loki: `create_tenant`, `complete_registration`, `login`-rivit

**2. Käyttäjän kutsuminen (admin → user)**

- Admin-portaali → `Käyttäjät` → `Kutsu uusi käyttäjä`
- Lomake: nimi, sähköposti, rooli (Käyttäjä / Hyväksyjä — Admin-roolia ei voi valita, ops-kerros torjuu erikseen)
- Mailpitistä uusi viesti `Kutsu Grooveserveen — Firma Oy`
- Kutsulinkin avaaminen → `/accept-invitation?token=...` → salasanan asetus → "Tili aktivoitu"
- Kirjaudu uutena käyttäjänä → näkyy `/admin/users`-listalla aktiivisena
- Tarkista: `tenant_users.status='active'`, `user_emails.verified=true`, `invitations.status='accepted'`

**3. Roolin vaihto + deaktivointi**

- `/admin/users` taulukko: ei-admin-käyttäjillä rooli-pudotusvalikko (Käyttäjä↔Hyväksyjä), Deaktivoi-painike
- Vaihda rooli → muutos persistoituu, audit-rivi `update_role` kirjattu
- Deaktivoi → `tenant_users.status='disabled'` ja kohde-käyttäjän kaikki sessiot tuhottu samassa transaktiossa
- Yritä kirjautua deaktivoituna → 401 `Tunnistautuminen epäonnistui` (audit `login_blocked`)
- Aktivoi uudelleen → kirjautuminen toimii
- **Last-admin -suojaus**: yritä deaktivoida ainoa admin (avaa SQL:llä `SELECT id, role FROM tenant_users` ja tunnista oma rivisi) → 409 `Tenantin viimeistä pääkäyttäjää ei voi deaktivoida` (UI rajaa nappia, mutta ops-suojauksen voi laukaista suoraan POST:lla `/admin/users/<oma_id>/disable` → ohjautuu 409:ään)

**4. Salasanan palautus (operaattori → käyttäjä)**

```bash
# Paikallisesti gsdev-instanssin DB:tä vasten
DB_URL=$(grep DATABASE_URL services/api/.env.local | cut -d= -f2-)
GSADMIN_DIRECT_DB_URL="$DB_URL" \
  uv run --directory tools/admin gsadmin password-reset anna@firma.fi \
  --base-url http://127.0.0.1:<api_port>

# Tulostaa kertakäyttöisen URL:n. Jaa käyttäjälle.
```

- Käyttäjä avaa URL:n → `/reset-password`-lomake → asettaa uuden salasanan → "Salasana vaihdettu"
- Vanha salasana → 401, uusi → 303 `/admin`
- Aiemmat sessiot tuhotut: jos toinen selain oli kirjautuneena, seuraava `/admin`-pyyntö → 303 `/login`
- Re-issue-tarkistus: aja `gsadmin password-reset anna@firma.fi` toistamiseen → vanha linkki tuottaa 404 (`Tämä linkki ei ole enää voimassa`), uusi toimii

**5. CSRF-suojaus**

- Avaa `/login`, lähetä form ilman `csrf_token`-kenttää (esim. `curl -d "email=...&password=..."`) → 403 `Lomakkeen turvatarkistus epäonnistui`
- Yritä POST eri originista — esim. selaimen DevToolsista `fetch('http://localhost:<api_port>/login', {method:'POST', body:...})` toiselta domainilta — SameSite=Strict csrf-cookie ei lähde mukaan → 403
- Logout-painike on form joka POST:aa csrf-tokenilla — toimii vain ko. selainsessiosta

**6. Sessio-elinkaari**

- Login → tarkista DevToolsista `session`-cookien Max-Age = 30 päivää, HttpOnly, SameSite=Lax
- Aja SQL: `UPDATE sessions SET expires_at = NOW() - INTERVAL '1 hour'`
- Seuraava `/admin`-kutsu → 303 `/login`. `sessions`-rivi siivottu automaattisesti `resolve`-pyynnöllä
- Logout (admin-navin painike) → DELETE `sessions`-rivi + cookie nollattu

### Pre-merge-pikatesti (tämä haara, ennen mainin yhdistämistä)

Tämä haara ei sisällä gsdev:tä eikä env-templateja. Manuaalinen setup:

```bash
# 1. Postgres-rooli (kerran):
podman exec gsdev-postgres psql -U postgres -c "ALTER ROLE grooveserve PASSWORD 'devpassword';"

# 2. API-server:
cd services/api
DATABASE_URL="postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_api_local_dev_setup_b5e17661" \
SMTP_SKIP_STARTUP_CHECK=1 SMTP_USER=test SMTP_PASSWORD=test \
LISTEN_ADDR=127.0.0.1:13551 COOKIE_INSECURE=1 \
cargo run

# 3. SMTP epäonnistuu → verifikaatiotokenia ei voi avata mailista. Workaround:
#    rekisteröi → aktivoi tenantti SQL:llä → aseta salasana gsadmin-resetillä:
podman exec gsdev-postgres psql -U grooveserve -d grooveserve_api_local_dev_setup_b5e17661 -c "
  UPDATE tenants SET status='active' WHERE id=(SELECT tenant_id FROM tenant_users WHERE user_id=1);
  UPDATE tenant_users SET status='active' WHERE user_id=1;
  UPDATE user_emails SET verified=true WHERE user_id=1;
  UPDATE auth_tokens SET used_at=NOW() WHERE user_id=1 AND used_at IS NULL;
"
GSADMIN_DIRECT_DB_URL="postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_api_local_dev_setup_b5e17661" \
  uv run --directory tools/admin gsadmin password-reset anna@firma.fi \
  --base-url http://127.0.0.1:13551
# Avaa tulostettu URL → aseta salasana → /login → /admin
```

Yllä oleva polku on testattu paikallisesti 2026-04-28 ja toimii päästä päähän (rekisteröinti → SQL-aktivointi → reset-link → login → /admin → vanha pw 401 → uusi pw 303). Mailpit-perusteinen flow toimii vasta merge-jälkeen.

### Tunnetut testausrajoitukset

- **TURNSTILE**: paikallisesti `TURNSTILE_SECRET` jätetään tyhjäksi → tarkistus ohitetaan, warning-loki
- **Rate limiter**: `tower_governor` SmartIp-extractor tarvitsee `X-Forwarded-For`-headerin loopback-pyyntöihin; muuten "Unable To Extract Key!" 500. Selain ei aseta tätä — manuaaliset curl-testit tarvitsevat sen.
- **BASE_URL-portti**: ks. yllä — env.api.template osoittaa www_porttiin, mutta uudet routet ovat API:ssa. Korjaa templaatissa tai vaihda portti URL:eista käsin
- **find_user_by_email cross-DB**: tämän haaran `ops::find_user_by_email` lukee API:n DB:tä, mutta email-palvelu kirjoittaa omaan DB:hen — Phase 6.1 db-konsolidointi yhdistää nämä
