# Multi-Tenant User Management — Design Document

_Issue: #26 | Created: 2026-04-26 | Updated: 2026-04-26 | Author: jari_

## 1. Overview

Grooveserve tarvitsee monitenanttiisen käyttäjähallintajärjestelmän B2B-matkalaskupalvelua varten. Tämä dokumentti kattaa:

1. **Käyttäjäpolut** — admin, tavallinen käyttäjä, agenttivälitteinen hallinta
2. **Tietomalli** — tenant, käyttäjä, kutsu, sähköpostiosoitteet
3. **Unified Tool Surface** — yhtenäinen operaatiokerros agentille ja webille
4. **Tekninen arkkitehtuuri** — API, autentikaatio, integraatio email-palveluun
5. **Admin-portaali** — UX ja toiminnallisuudet

---

## 2. Arkkitehtuuriperiaate: Unified Tool Surface

**Ydinsääntö:** Web-käyttöliittymä ja AI-agentti käyttävät samoja domain-operaatioita (`ops::*`), mutta jokaisella kanavalla on oma auktorisaatio- ja vahvistuspolitiikkansa.

```
┌──────────────┐     ┌──────────────┐
│   Web UI     │     │  AI Agent    │
│  (REST API)  │     │  (tool_use)  │
└──────┬───────┘     └──────┬───────┘
       │                    │
       ▼                    ▼
┌──────────────────────────────────┐
│       Operations Layer           │
│  (Rust module: ops::*)           │
│                                  │
│  invite_user()  list_users()     │
│  disable_user() update_role()    │
│  get_tenant()                    │
└──────────────┬───────────────────┘
               │
               ▼
┌──────────────────────────────────┐
│       PostgreSQL 17              │
│  tenants, users, invitations     │
└──────────────────────────────────┘
```

**Käytännössä tämä tarkoittaa:**

- Jokainen operaatio on Rust-funktio `ops`-moduulissa
- REST API kutsuu näitä funktioita suoraan
- AI-agentin tool_use-handler kutsuu samoja funktioita
- Operaatiot saavat `OpContext`-structin joka sisältää autentikoidun käyttäjän, tenantin ja kanavan
- Kanavakohtainen politiikka (esim. sähköpostiagentti vaatii web-vahvistuksen joillekin operaatioille) toteutetaan kutsujakerrokssa

**Miksi ei erillisiä "agent API"- ja "web API" -kerroksia:**

- Yksi paikka bisneslogiikalle — ei divergenssiä
- Yksi paikka auktorisoinnille — ei tuplasääntöjä
- Testattavuus: operaatiot testataan kerran, molemmat kanavat toimivat

---

## 3. Käyttäjäpolut

### 3.1 Yrityksen rekisteröityminen (Admin Flow)

```
1. Admin menee grooveserve.com → "Aloita ilmainen kokeilu"
2. Cloudflare Turnstile -tarkistus ("I'm human")
3. Rekisteröitymislomake (vain perustiedot):
   - Yrityksen nimi
   - Adminin nimi ja sähköposti
4. Tenant luodaan statuksella 'pending_verification'
5. Sähköpostiin lähetetään linkki set-password-sivulle
6. Admin klikkaa linkkiä → server-rendered HTML-sivu pyytää salasanan
7. Salasanan POST kuluttaa tokenin, hashaa salasanan (argon2id) ja
   atomisesti aktivoi: tenant → 'active', admin-käyttäjä → 'active',
   user_emails → verified
8. Admin ohjataan admin-portaaliin
9. Admin voi kutsua käyttäjiä
```

**Miksi salasana asetetaan vahvistuslinkin takana, ei rekisteröintilomakkeessa:**
- Sähköpostiskannerit (Microsoft Safe Links, Proofpoint, Mimecast) prefetchaavat sähköpostien linkkejä — GET /set-password renderöi vain HTML-lomakkeen eikä kuluta tokenia. POST kuluttaa.
- Yhdistää sähköpostin vahvistamisen ja salasanan asettamisen yhteen vaiheeseen — admin pääsee suoraan kirjautumaan ilman erillistä "lähetetty toinen viesti, klikkaa vielä kerran" -kierrettä.
- Magic-link -vaihtoehto (assistantin generoima) lisätään myöhemmin tämän rinnalle, ei tilalle.

**Tietokantaoperaatiot (services/api/src/ops/tenant.rs):**
- `ops::create_tenant(input)` → tenant (pending) + user (no password) + email (unverified) + auth_token (purpose='email_verification')
- `ops::inspect_registration_token(raw_token)` → tarkistaa tokenin ilman kulutusta, palauttaa email + user_id (set-password-sivun GET-renderöintiä varten)
- `ops::complete_registration(raw_token, raw_password)` → kuluttaa tokenin, hashaa salasanan argon2:lla, aktivoi tenantin/jäsenyyden/emailin, kirjaa audit_event
- `ops::resend_registration_verification(email)` → mitätöi vanhat unused-tokenit, luo uuden, palauttaa raw token kutsujalle (resend-endpointti lähettää uuden sähköpostin)

### 3.2 Käyttäjän kutsuminen (Web)

```
1. Admin avaa admin-portaalin → "Käyttäjät" → "Kutsu käyttäjä"
2. Lomake: nimi, sähköposti, rooli (käyttäjä/hyväksyjä)
3. Järjestelmä luo käyttäjän (status='invited'), kutsun, ja lähettää kutsusähköpostin
4. Kutsuttu klikkaa linkkiä → asettaa salasanan → tili aktivoituu
```

**Tietokantaoperaatiot:**
- `ops::invite_user(ctx, input)` → user (invited) + user_email (unverified) + invitation + outbound_email
- `ops::accept_invitation(token)` → user active, email verified, invitation accepted

**Invariantti:** `invite_user` luo aina `users`-rivin statuksella `invited` (Model B). Näin kutsuttu mutta ei-aktivoitunut käyttäjä voidaan tunnistaa sähköpostista ja muistuttaa aktivoinnista.

### 3.3 Käyttäjän kutsuminen (Agentti)

```
1. Admin lähettää sähköpostin:
   "Lisää käyttäjä Matti Meikäläinen, matti@firma.fi"
2. Agentti tunnistaa adminin → lataa tenant-kontekstin
3. Agentti käyttää invite_user-työkalua:
   tool_use: {
     name: "invite_user",
     input: {
       email: "matti@firma.fi",
       name: "Matti Meikäläinen",
       role: "user"
     }
   }
4. Operaatiokerros: ops::invite_user(ctx, ...) — sama funktio kuin webissä
5. Agentti vastaa: "Käyttäjä Matti Meikäläinen kutsuttu. Kutsu lähetetty osoitteeseen matti@firma.fi."
```

**Huom:** Sähköpostiagentti käyttää samoja operaatioita kuin web. Joidenkin operaatioiden kanavakohtainen vahvistuspolitiikka (esim. pending-tila ennen web-vahvistusta) käsitellään issuessa #22.

### 3.4 Tunnistamaton lähettäjä

```
1. Tuntematon sähköposti lähettää viestin assistant@grooveserve.com:iin
2. Agentti etsii lähettäjää käyttäjätietokannasta → ei löydy
3. Agentti vastaa kohteliaasti:
   "En tunnista sähköpostiosoitettasi. Jos yrityksesi käyttää Grooveservea,
    pyydä pääkäyttäjääsi lisäämään sinut. Muussa tapauksessa voit rekisteröidä
    yrityksesi osoitteessa grooveserve.com."
4. Ei conversation-historiaa — ei tallenneta tuntemattomien viestejä
```

**Huom:** Vastaus tuntemattomille vain jos SPF/DKIM/DMARC läpäisty. Sähköpostin spooffauksen esto käsitellään issuessa #15. Backscatter-riski huomioitava.

### 3.5 Tavallisen käyttäjän arki

```
1. Käyttäjä lähettää kuitin sähköpostiin assistant@grooveserve.com
2. Agentti tunnistaa käyttäjän → lataa profiilin ja tenant-kontekstin
3. Agentti käsittelee kuitin tenanttikohtaisilla säännöillä
4. Vastaus käyttäjälle: kuitti käsitelty, matkalaskuun lisätty
```

---

## 4. Tietomalli

### 4.1 Yleiset apufunktiot

```sql
-- updated_at-trigger kaikkiin tauluihin joissa on updated_at-kenttä
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
```

### 4.2 Taulut

#### `tenants` — Yritys/organisaatio

```sql
CREATE TABLE tenants (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,      -- URL-ystävällinen tunniste
    status      TEXT NOT NULL DEFAULT 'pending_verification'
                CHECK (status IN ('pending_verification', 'active', 'suspended', 'deleted')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_tenants_slug ON tenants (slug);

CREATE TRIGGER trg_tenants_updated_at
    BEFORE UPDATE ON tenants
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
```

**Huomiot:**
- `slug` generoidaan nimestä (esim. "Firma Oy" → "firma-oy"), uniikki
- `status`: pending_verification (rekisteröity, ei vahvistettu), active (normaali), suspended (maksu myöhässä), deleted (poistettu)
- Ei billing-kenttiä MVP:ssä — lisätään myöhemmin

#### `users` — Käyttäjän identiteetti

```sql
CREATE TABLE users (
    id              BIGSERIAL PRIMARY KEY,
    name            TEXT NOT NULL,
    password_hash   TEXT,                    -- NULL until user sets password
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
```

**Huom:** `users` on identiteettitaulu — ei sisällä tenant-tietoa. Tenanttijäsenyys on `tenant_users`-taulussa. Tämä mahdollistaa tulevaisuudessa saman käyttäjän kuulumisen useaan tenanttiin.

#### `tenant_users` — Käyttäjän jäsenyys tenantissa

```sql
CREATE TABLE tenant_users (
    id          BIGSERIAL PRIMARY KEY,
    tenant_id   BIGINT NOT NULL REFERENCES tenants(id),
    user_id     BIGINT NOT NULL REFERENCES users(id),
    role        TEXT NOT NULL DEFAULT 'user'
                CHECK (role IN ('admin', 'user', 'approver')),
    status      TEXT NOT NULL DEFAULT 'invited'
                CHECK (status IN ('invited', 'active', 'disabled')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_tenant_users_membership ON tenant_users (tenant_id, user_id);
CREATE INDEX idx_tenant_users_tenant ON tenant_users (tenant_id);
CREATE INDEX idx_tenant_users_user ON tenant_users (user_id);
CREATE INDEX idx_tenant_users_tenant_role ON tenant_users (tenant_id, role);

CREATE TRIGGER trg_tenant_users_updated_at
    BEFORE UPDATE ON tenant_users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
```

**Roolit:**
- `admin` — pääkäyttäjä, voi hallita käyttäjiä ja organisaatiota
- `user` — tavallinen käyttäjä, voi lähettää kuitteja
- `approver` — hyväksyjä, voi hyväksyä matkalaskuja

**Status:**
- `invited` — kutsu lähetetty, salasanaa ei asetettu
- `active` — aktiivinen käyttäjä
- `disabled` — deaktivoitu (admin poistanut, mutta data säilytetään)

**MVP-rajoitus:** Vaikka tietomalli tukee many-to-many -suhdetta (käyttäjä voi kuulua useaan tenanttiin), MVP:ssä käyttäjä kuuluu vain yhteen tenanttiin. Jos sama email yrittää liittyä toiseen tenanttiin, annetaan virheilmoitus.

#### `user_emails` — Käyttäjän sähköpostiosoitteet

```sql
CREATE TABLE user_emails (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id),
    email       TEXT NOT NULL,
    is_primary  BOOLEAN NOT NULL DEFAULT false,
    verified    BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_user_emails_email ON user_emails (LOWER(email));
CREATE INDEX idx_user_emails_user ON user_emails (user_id);
CREATE UNIQUE INDEX idx_user_emails_one_primary ON user_emails (user_id) WHERE is_primary;
```

**Miksi erillinen taulu:**
- Käyttäjällä voi olla useita sähköpostiosoitteita (työ + henkilökohtainen)
- Agentin tunnistus toimii millä tahansa rekisteröidyllä osoitteella
- Uniikki indeksi estää saman osoitteen käytön kahdella käyttäjällä
- `LOWER(email)` — normalisointi on tietokantatasolla, ei sovelluksessa
- Partial unique index `idx_user_emails_one_primary` varmistaa max yksi ensisijainen osoite per käyttäjä

**MVP-rajoitus:** Globaali uniikki email tarkoittaa, että sama osoite ei voi olla kahdessa tenantissa. Tämä on tietoinen rajoitus — konsultit/kirjanpitäjät tarvitsevat eri osoitteet eri yrityksille. Myöhemmin voidaan muuttaa `(tenant_id, LOWER(email))` -uniikiksi kun multi-tenant-reititys on ratkaistu.

#### `invitations` — Kutsut

```sql
CREATE TABLE invitations (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),  -- pre-created invited user
    token_hash      BYTEA NOT NULL UNIQUE,                 -- SHA-256 hash of token
    invited_by      BIGINT NOT NULL REFERENCES users(id),
    role            TEXT NOT NULL DEFAULT 'user'
                    CHECK (role IN ('admin', 'user', 'approver')),
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'accepted', 'expired', 'cancelled')),
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at     TIMESTAMPTZ
);

CREATE INDEX idx_invitations_tenant ON invitations (tenant_id);
CREATE UNIQUE INDEX idx_invitations_pending_user ON invitations (user_id) WHERE status = 'pending';
```

**Huomiot:**
- `token_hash`: SHA-256 hash tokenista (32 tavua). Raakaa tokenia ei tallenneta tietokantaan — se lähetetään vain sähköpostissa.
- `user_id`: viittaa pre-created `users`-riviin (Model B). Kutsun hyväksyminen aktivoi käyttäjän.
- `expires_at`: oletuksena 7 päivää kutsun luomisesta
- Partial unique index `idx_invitations_pending_user` estää useita aktiivisia kutsuja samalle käyttäjälle

#### `sessions` — Web-istunnot

```sql
CREATE TABLE sessions (
    id_hash     BYTEA PRIMARY KEY,              -- SHA-256 hash of session token
    user_id     BIGINT NOT NULL REFERENCES users(id),
    tenant_id   BIGINT NOT NULL REFERENCES tenants(id),  -- active tenant context
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_sessions_user ON sessions (user_id);
CREATE INDEX idx_sessions_expires ON sessions (expires_at);
```

**Huomiot:**
- `id_hash`: SHA-256 hash session-tokenista. Cookie saa raaka-tokenin, tietokanta vain hashin.
- `tenant_id`: aktiivinen tenant-konteksti (MVP: ainoa tenant johon käyttäjä kuuluu)
- Vanhojen sessioiden siivous: erillinen issue (#27)

#### `auth_tokens` — Verifikaatio- ja reset-tokenit

```sql
CREATE TABLE auth_tokens (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id),
    purpose     TEXT NOT NULL CHECK (purpose IN (
                    'email_verification',
                    'password_reset'
                )),
    token_hash  BYTEA NOT NULL UNIQUE,         -- SHA-256 hash of token
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ
);
```

#### `audit_events` — Auditointilokit

```sql
CREATE TABLE audit_events (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    actor_user_id   BIGINT REFERENCES users(id),
    actor_email     TEXT,
    channel         TEXT NOT NULL CHECK (channel IN ('web', 'email_agent', 'internal')),
    action          TEXT NOT NULL,              -- 'invite_user', 'disable_user', 'update_role', etc.
    target_type     TEXT NOT NULL,              -- 'user', 'tenant', 'invitation'
    target_id       BIGINT,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_events_tenant_created ON audit_events (tenant_id, created_at DESC);
CREATE INDEX idx_audit_events_actor ON audit_events (actor_user_id, created_at DESC);
```

**Auditoidaan vähintään:**
- Käyttäjän kutsuminen, aktivointi, deaktivointi
- Roolin muuttaminen
- Tenantin luominen, asetusten muuttaminen
- Kirjautuminen (onnistunut/epäonnistunut)
- Agentin tool_use-kutsut

### 4.3 Olemassa olevien taulujen muutokset

#### `conversations` — lisätään tenant_id ja user_id

```sql
ALTER TABLE conversations
    ADD COLUMN tenant_id BIGINT REFERENCES tenants(id),
    ADD COLUMN user_id BIGINT REFERENCES users(id);

CREATE INDEX idx_conversations_tenant ON conversations (tenant_id);
CREATE INDEX idx_conversations_tenant_user ON conversations (tenant_id, user_id);
```

**Migraatiostrategia:**
- `tenant_id` ja `user_id` ovat aluksi NULL (vanhat viestit ennen käyttäjähallintaa)
- Uudet viestit tunnistetuilta käyttäjiltä saavat aina molemmat
- `sender`-kenttä säilytetään yhteensopivuuden vuoksi ja fallbackina

### 4.4 ER-kaavio

```
┌─────────┐     ┌──────────────┐     ┌───────────┐     ┌──────────────┐
│ tenants │────<│ tenant_users │>────│   users   │────<│ user_emails  │
│         │     │              │     │           │     │              │
│ id      │     │ id           │     │ id        │     │ id           │
│ name    │     │ tenant_id    │     │ name      │     │ user_id      │
│ slug    │     │ user_id      │     │ password  │     │ email        │
│ status  │     │ role         │     └─────┬─────┘     │ is_primary   │
└────┬────┘     │ status       │           │           │ verified     │
     │          └──────────────┘      ┌────┼────┐      └──────────────┘
     │                                │    │    │
     ▼                                ▼    ▼    ▼
┌──────────────┐  ┌──────────────┐ ┌────────┐ ┌──────────────┐
│ invitations  │  │ audit_events │ │sessions│ │conversations │
│              │  │              │ │        │ │              │
│ id           │  │ id           │ │id_hash │ │ id           │
│ tenant_id    │  │ tenant_id    │ │user_id │ │ tenant_id    │
│ user_id      │  │ actor_user_id│ │tenant  │ │ user_id      │
│ token_hash   │  │ channel      │ │expires │ │ sender       │
│ invited_by   │  │ action       │ └────────┘ │ role         │
│ role         │  │ target_type  │            │ content      │
│ expires_at   │  │ metadata     │            └──────────────┘
└──────────────┘  └──────────────┘
```

---

## 5. Unified Tool Surface — Operaatiot

### 5.1 OpContext — auktorisoinnin perusta

```rust
pub enum Channel {
    Web,
    EmailAgent,
    Internal,
}

pub struct OpContext {
    pub actor_user_id: i64,
    pub tenant_id: i64,
    pub role: UserRole,
    pub channel: Channel,
}
```

Jokainen operaatio saa `OpContext`-structin ensimmäisenä parametrina. `tenant_id` ja `actor_user_id` johdetaan autentikaatiosta — eivät koskaan tule clientiltä tai tool-inputista.

### 5.2 Moduulirakenne

```
services/email/src/          (tai services/api/src/ myöhemmin)
├── ops/                     # Operaatiokerros
│   ├── mod.rs               # re-exports
│   ├── tenant.rs            # create_tenant, get_tenant, update_tenant
│   ├── user.rs              # invite_user, list_users, update_role, disable_user
│   ├── auth.rs              # login, verify_session, change_password, password_reset
│   └── error.rs             # OpError — yhtenäinen virhetyyppi
├── api/                     # REST API (axum)
│   ├── mod.rs
│   ├── admin.rs             # admin-portaalin endpointit
│   ├── auth.rs              # login/logout/register
│   └── middleware.rs         # auth middleware, CSRF
├── tools/                   # AI agent tool definitions
│   ├── mod.rs
│   └── user_management.rs   # tool_use handlerit → kutsuvat ops::*
```

### 5.3 Operaatiot ja niiden tool-vastineet

✓ = toteutettu Phase 1.5:ssä.

| Operaatio | Web API | Agent Tool | Oikeus | |
|-----------|---------|------------|--------|---|
| `ops::create_tenant` | `POST /api/register` | — | public | ✓ |
| `ops::inspect_registration_token` | `GET /set-password` (HTML) | — | public (token) | ✓ |
| `ops::complete_registration` | `POST /set-password` | — | public (token) | ✓ |
| `ops::resend_registration_verification` | `POST /api/register/resend` | — | public | ✓ |
| `ops::find_user_by_email` | (sisäinen) | (sisäinen, käytetään email-agentin tunnistuksessa) | sisäinen | ✓ |
| `ops::login` | `POST /api/auth/login` | — | public | Phase 2 |
| `ops::password_reset_request` | `POST /api/auth/password-reset/request` | — | public | Phase 2 |
| `ops::password_reset_confirm` | `POST /api/auth/password-reset/confirm` | — | public (token) | Phase 2 |
| `ops::invite_user` | `POST /api/admin/users/invite` | `invite_user` | admin | Phase 3 |
| `ops::list_users` | `GET /api/admin/users` | `list_users` | admin | Phase 3 |
| `ops::update_role` | `PATCH /api/admin/users/:id` | `update_user_role` | admin | Phase 3 |
| `ops::disable_user` | `POST /api/admin/users/:id/disable` | `disable_user` | admin | Phase 3 |
| `ops::get_tenant` | `GET /api/admin/tenant` | `get_company_info` | admin | Phase 3 |
| `ops::accept_invitation` | `POST /api/invitations/:token/accept` | — | public (token) | Phase 3 |

**Huom:** Kaikki admin-operaatiot vaativat autentikaation + admin-roolin. `tenant_id` tulee aina `OpContext`:sta.

**Huom 2:** `create_tenant` on poikkeus joka ottaa pelkän `input`:n ilman `OpContext`:a — rekisteröinti aidosti edeltää autentikaatiota. Audit-rivi kirjoitetaan `audit::record_with_email`-helperillä joka sallii NULL `actor_user_id`:n. Kaikki muut operaatiot ottavat `&OpContext` ensimmäisenä parametrina.

### 5.4 Operaation anatomia

```rust
// ops/user.rs

pub struct InviteUserInput {
    pub email: String,
    pub name: String,
    pub role: UserRole,
}

pub struct InviteUserOutput {
    pub invitation_id: i64,
    pub user_id: i64,
    pub email: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn invite_user(
    db: &PgPool,
    ctx: &OpContext,
    input: InviteUserInput,
) -> Result<InviteUserOutput, OpError> {
    // 1. Tarkista oikeudet: ctx.role == admin
    // 2. Tarkista ettei email ole jo käytössä
    // 3. Tarkista ettei aktiivista kutsua ole
    // 4. Luo users-rivi (status='invited')
    // 5. Luo user_emails-rivi (verified=false)
    // 6. Luo tenant_users-rivi (status='invited')
    // 7. Generoi turvallinen token, tallenna hash
    // 8. Luo invitation-rivi
    // 9. Kirjaa audit_event
    // 10. Palauta tulos (sähköpostin lähetys on kutsuja vastuulla)
    ...
}

pub async fn disable_user(
    db: &PgPool,
    ctx: &OpContext,
    target_user_id: i64,
) -> Result<(), OpError> {
    let mut tx = db.begin().await?;

    // Last-admin protection: varmista ettei viimeistä adminia poisteta
    let admin_count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM tenant_users
         WHERE tenant_id = $1 AND role = 'admin' AND status = 'active'
         FOR UPDATE",
        ctx.tenant_id
    ).fetch_one(&mut *tx).await?;

    // ... disable + audit
    tx.commit().await?;
}
```

**Periaatteet:**
- `OpContext` sisältää aina `tenant_id` ja `actor_user_id` — johdettu autentikaatiosta
- Operaatio ei tiedä kutsukanavasta mitään (paitsi audit-logitukseen)
- Input/Output ovat vahvasti tyypitettyjä structeja
- Virhe on `OpError` enum: `NotFound`, `AlreadyExists`, `Forbidden`, `InvalidInput(String)`, `LastAdminProtection`
- Last-admin -tarkistus: tenant_users-taulun admin-rivit lukitaan `FOR UPDATE` ennen muutosta

### 5.5 Agent Tool -määrittelyt

Agentin system promptiin lisättävät työkalut:

```json
[
  {
    "name": "invite_user",
    "description": "Kutsu uusi käyttäjä organisaatioon. Lähettää kutsusähköpostin.",
    "input_schema": {
      "type": "object",
      "properties": {
        "email": { "type": "string", "description": "Käyttäjän sähköpostiosoite" },
        "name": { "type": "string", "description": "Käyttäjän nimi" },
        "role": {
          "type": "string",
          "enum": ["user", "approver"],
          "description": "Rooli: user (tavallinen) tai approver (hyväksyjä)"
        }
      },
      "required": ["email", "name"]
    }
  },
  {
    "name": "list_users",
    "description": "Listaa organisaation käyttäjät ja heidän roolinsa.",
    "input_schema": {
      "type": "object",
      "properties": {}
    }
  },
  {
    "name": "disable_user",
    "description": "Deaktivoi käyttäjä organisaatiosta. Käyttäjän data säilytetään.",
    "input_schema": {
      "type": "object",
      "properties": {
        "email": { "type": "string", "description": "Deaktivoitavan käyttäjän sähköposti" }
      },
      "required": ["email"]
    }
  },
  {
    "name": "update_user_role",
    "description": "Vaihda käyttäjän rooli.",
    "input_schema": {
      "type": "object",
      "properties": {
        "email": { "type": "string", "description": "Käyttäjän sähköposti" },
        "role": {
          "type": "string",
          "enum": ["user", "approver"],
          "description": "Uusi rooli"
        }
      },
      "required": ["email", "role"]
    }
  },
  {
    "name": "get_company_info",
    "description": "Näytä organisaation tiedot: nimi, käyttäjämäärä, tilaus.",
    "input_schema": {
      "type": "object",
      "properties": {}
    }
  }
]
```

**Huomioita:**
- `tenant_id` injektoidaan tool-kutsuun email-luupissa (ei LLM:n inputista)
- Admin ei voi lisätä `admin`-roolia toolilla — tämä estää eskalaation
- Tool handler resolvoi email → user_id ennen ops-kutsua (adapter-kerros)
- Nimeäminen johdonmukaista ops-tason kanssa: `invite_user`, `disable_user` (ei `add_user`, `remove_user`)

---

## 6. Tekninen arkkitehtuuri

### 6.1 Palveluarkkitehtuuri — MVP

MVP:ssä email-palvelu ja web-API ovat **sama Rust-binääri**. Tämä yksinkertaistaa deploymenttia ja jakaa tietokantapooliin.

```
┌─────────────────────────────────────────────┐
│              grooveserve (binary)            │
│                                             │
│  ┌─────────────────┐  ┌──────────────────┐  │
│  │  IMAP/SMTP      │  │  HTTP Server     │  │
│  │  Email Loop     │  │  (axum)          │  │
│  │                 │  │                  │  │
│  │  agent.rs       │  │  /api/auth/*     │  │
│  │  handler.rs     │  │  /api/admin/*    │  │
│  │  spam.rs        │  │  /api/invite/*   │  │
│  └────────┬────────┘  └────────┬─────────┘  │
│           │                    │             │
│           ▼                    ▼             │
│  ┌──────────────────────────────────────┐   │
│  │         ops::* (Operations Layer)    │   │
│  └──────────────────┬───────────────────┘   │
│                     │                        │
│                     ▼                        │
│  ┌──────────────────────────────────────┐   │
│  │         PostgreSQL (sqlx pool)       │   │
│  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

**Miksi yksi binääri:**
- Ei tarvitse palvelujen välistä kommunikaatiota
- Jaettu tietokantapooli
- Yksinkertaisempi deployment (yksi kontti)
- Voidaan jakaa myöhemmin kun tarve tulee

**Huom:** Single binary -vikaantumismalli (graceful shutdown, task supervision) käsitellään issuessa #30.

### 6.2 Autentikaatio

#### Web-autentikaatio
- **Session-pohjainen**: HTTP-only secure cookie (`session_token`)
- Session-token hashataan (SHA-256) ennen tallennusta — tietokannassa vain hash
- Ei JWT:tä MVP:ssä — sessionit tietokannassa, helppo invalidoida
- Login: `POST /api/auth/login` → session cookie
- Logout: `DELETE /api/auth/session` → poistaa session-rivin
- Joka request: middleware tarkistaa session → lataa user + tenant
- **CSRF-suojaus**: Token server-rendered lomakkeissa, validointi kaikissa muuttavissa pyynnöissä
  - htmx: `<body hx-headers='{"X-CSRF-Token": "{{ csrf_token }}"}'>`
- **Login rate limiting**: Rajoita kirjautumisyritykset per IP ja per email

#### Agenttitunnistus (sähköposti)
- Lähettäjän sähköpostiosoite → `user_emails`-lookup → `user` + `tenant` (via `tenant_users`)
- Ei salasanaa — sähköpostin autenttisuus vahvistetaan SPF/DKIM/DMARC:lla (nykyinen spam-kerros)
- Tunnistuksen jälkeen agentille syötetään konteksti:

```
Käyttäjä: Matti Meikäläinen (matti@firma.fi)
Organisaatio: Firma Oy
Rooli: admin
```

#### Salasanojen tallennus
- `argon2id` (Rust: `argon2`-crate)
- Ei plaintext-salasanoja missään

#### Rekisteröinnin suojaus
- **Cloudflare Turnstile** ("I'm human" -tarkistus) rekisteröitymislomakkeessa
- Rate limiting per IP

### 6.3 Auktorisointi

Yksinkertainen roolipohjainen malli:

| Toiminto | admin | user | approver |
|----------|-------|------|----------|
| Lähettää kuitteja | x | x | x |
| Näkee omat matkalaskut | x | x | x |
| Hallitsee käyttäjiä | x | | |
| Hyväksyy matkalaskuja | x | | x |
| Organisaation asetukset | x | | |

**Tenant-eristys:** Jokainen tietokantakysely sisältää `WHERE tenant_id = $1`. Ei poikkeuksia. Operaatiokerros saa `tenant_id`:n `OpContext`:sta (autentikaatiosta), ei clientiltä.

**Last-admin -suojaus:** Adminin deaktivointi tai roolin muutos tarkistaa transaktionaalisesti, ettei se ole tenantin viimeinen aktiivinen admin.

### 6.4 API-suunnittelu

REST (JSON), axum-framework. Phase 1.5:ssä toteutetut endpointit on merkitty
✓; loput tulevat Phase 2-3:ssa.

```
✓ POST   /api/register                    # Yrityksen rekisteröityminen (Turnstile + rate limit)
✓ POST   /api/register/resend             # Uuden vahvistusmailin pyyntö (Turnstile + rate limit)
✓ GET    /set-password?token=...          # Server-rendered HTML-lomake (token NOT consumed)
✓ POST   /set-password                    # Form submit: token + password → tili aktivoidaan
✓ GET    /health                          # DB-probe (SELECT 1 → 200 / 503)

  POST   /api/auth/login                  # Kirjautuminen (rate limited) — Phase 2
  DELETE /api/auth/session                # Uloskirjautuminen — Phase 2
  GET    /api/auth/me                     # Nykyisen käyttäjän tiedot — Phase 2
  POST   /api/auth/password-reset/request # Salasanan palautuspyyntö — Phase 2
  POST   /api/auth/password-reset/confirm # Salasanan palautus tokenilla — Phase 2

  GET    /api/admin/tenant                # Organisaation tiedot — Phase 3
  PATCH  /api/admin/tenant                # Organisaation muokkaus — Phase 3

  GET    /api/admin/users                 # Käyttäjälista — Phase 3
  POST   /api/admin/users/invite          # Kutsu käyttäjä — Phase 3
  PATCH  /api/admin/users/:id             # Muokkaa käyttäjää (rooli) — Phase 3
  POST   /api/admin/users/:id/disable     # Deaktivoi käyttäjä — Phase 3
  POST   /api/admin/users/:id/enable      # Aktivoi käyttäjä uudelleen — Phase 3

  POST   /api/invitations/:token/accept   # Hyväksy kutsu (public) — Phase 3
  GET    /api/invitations/:token          # Kutsun tiedot (public) — Phase 3
```

**Huom:** rekisteröinnin vahvistus tapahtuu set-password-sivun POST:ssa, ei
erillisenä `/api/verify`-endpointtina. GET vain renderöi lomakkeen, jotta
sähköpostiskannerit eivät kuluta tokenia prefetchillä.

### 6.5 Agentin tool_use-integraatio

Nykyinen `agent.rs` käsittelee `StopReason::ToolUse` virheenä. Muutos:

```rust
const MAX_TOOL_ITERATIONS: usize = 5;

// agent.rs — muutettu tool_use-käsittely
for _iteration in 0..MAX_TOOL_ITERATIONS {
    let response = client.call(&request).await?;

    match response.stop_reason {
        StopReason::EndTurn => return Ok(render_reply(response)),
        StopReason::ToolUse => {
            let tool_uses = parse_tool_uses(&response)?;
            // Jokaiselle tool_use:lle:
            //   a) Injektoi tenant_id OpContext:sta (EI LLM:n inputista)
            //   b) Tarkista oikeudet (ctx.role vs. tool vaatimus)
            //   c) Kutsutaan ops::*-funktiota
            //   d) Muodostetaan tool_result
            let tool_results = execute_tools(&tool_uses, &ctx, &db).await?;
            // Jatka looppia: lähetä tool_results agentille
            request = build_followup(response, tool_results);
        }
        StopReason::MaxTokens => return Ok(truncation_warning(response)),
        _ => return Err(AgentError::UnsupportedStopReason),
    }
}
// Loop limit ylitetty
Err(AgentError::ToolLoopLimitExceeded)
```

**Agenttinen loop laajenee:**

```
Email saapuu
  → Tunnista käyttäjä (user_emails lookup → user → tenant_users → tenant)
  → Rakenna OpContext { actor_user_id, tenant_id, role, channel: EmailAgent }
  → Kutsu Claude API (system prompt + tools + historia)
  → Loop (max 5 iteraatiota):
      ← EndTurn → lähetä vastaus
      ← ToolUse → injektoi tenant_id, suorita ops::*, lähetä tool_result
      ← MaxTokens → truncation-varoitus
  → Audit-log tool-kutsuista
```

---

## 7. Admin-portaali (UX)

### 7.1 Näkymät

**Dashboard** (`/admin`)
- Organisaation nimi ja perustiedot
- Käyttäjämäärä (aktiiviset / kutsutut)
- Viimeaikainen toiminta (viimeiset matkalaskut)

**Käyttäjälista** (`/admin/users`)
- Taulukko: nimi, sähköposti, rooli, status, viimeisin aktiivisuus
- "Kutsu käyttäjä" -painike
- Rivin toiminnot: muokkaa roolia, deaktivoi/aktivoi

**Kutsu käyttäjä** (modal tai sivu)
- Lomake: nimi, sähköposti, rooli (dropdown)
- Lähetä kutsu → näyttää vahvistuksen

**Organisaation asetukset** (`/admin/settings`)
- MVP: organisaation nimi
- Myöhemmin: laskutusosoite, hyväksyntäkierron asetukset

### 7.2 Tech stack (frontend)

MVP:lle riittää server-rendered HTML:

- **axum** + **askama** (tai maud) — Rust-templating
- **htmx** — interaktiivisuus ilman SPA-frameworkia
- **Pico CSS** (tai Simple.css) — minimaalinen CSS-framework
- **CSRF-tokenit** kaikissa lomakkeissa
- **Cache-Control: no-store** admin-sivuille

**Miksi ei React/SPA:**
- Admin-portaali on yksinkertainen CRUD-näkymä
- Server-rendered on nopeampi toteuttaa Rust-stackilla
- Ei tarvita reaaliaikaista tilanhallintaa
- htmx antaa riittävän interaktiivisuuden (inline edit, modal)
- Voidaan siirtyä SPA:han myöhemmin jos tarve tulee

---

## 8. Migraatiosuunnitelma

### 8.1 Uudet migraatiot

**004_create_tenants.sql:**
```sql
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE tenants (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    status      TEXT NOT NULL DEFAULT 'pending_verification'
                CHECK (status IN ('pending_verification', 'active', 'suspended', 'deleted')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER trg_tenants_updated_at
    BEFORE UPDATE ON tenants
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
```

**005_create_users.sql:**
```sql
CREATE TABLE users (
    id              BIGSERIAL PRIMARY KEY,
    name            TEXT NOT NULL,
    password_hash   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE tenant_users (
    id          BIGSERIAL PRIMARY KEY,
    tenant_id   BIGINT NOT NULL REFERENCES tenants(id),
    user_id     BIGINT NOT NULL REFERENCES users(id),
    role        TEXT NOT NULL DEFAULT 'user'
                CHECK (role IN ('admin', 'user', 'approver')),
    status      TEXT NOT NULL DEFAULT 'invited'
                CHECK (status IN ('invited', 'active', 'disabled')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_tenant_users_membership ON tenant_users (tenant_id, user_id);
CREATE INDEX idx_tenant_users_tenant ON tenant_users (tenant_id);
CREATE INDEX idx_tenant_users_user ON tenant_users (user_id);
CREATE INDEX idx_tenant_users_tenant_role ON tenant_users (tenant_id, role);

CREATE TRIGGER trg_tenant_users_updated_at
    BEFORE UPDATE ON tenant_users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE user_emails (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id),
    email       TEXT NOT NULL,
    is_primary  BOOLEAN NOT NULL DEFAULT false,
    verified    BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_user_emails_email ON user_emails (LOWER(email));
CREATE INDEX idx_user_emails_user ON user_emails (user_id);
CREATE UNIQUE INDEX idx_user_emails_one_primary ON user_emails (user_id) WHERE is_primary;
```

**006_create_invitations.sql:**
```sql
CREATE TABLE invitations (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    user_id         BIGINT NOT NULL REFERENCES users(id),
    token_hash      BYTEA NOT NULL UNIQUE,
    invited_by      BIGINT NOT NULL REFERENCES users(id),
    role            TEXT NOT NULL DEFAULT 'user'
                    CHECK (role IN ('admin', 'user', 'approver')),
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'accepted', 'expired', 'cancelled')),
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at     TIMESTAMPTZ
);

CREATE INDEX idx_invitations_tenant ON invitations (tenant_id);
CREATE UNIQUE INDEX idx_invitations_pending_user ON invitations (user_id) WHERE status = 'pending';
```

**007_create_sessions.sql:**
```sql
CREATE TABLE sessions (
    id_hash     BYTEA PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id),
    tenant_id   BIGINT NOT NULL REFERENCES tenants(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_sessions_user ON sessions (user_id);
CREATE INDEX idx_sessions_expires ON sessions (expires_at);
```

**008_create_auth_tokens.sql:**
```sql
CREATE TABLE auth_tokens (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id),
    purpose     TEXT NOT NULL CHECK (purpose IN (
                    'email_verification',
                    'password_reset'
                )),
    token_hash  BYTEA NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ
);
```

**009_create_audit_events.sql:**
```sql
CREATE TABLE audit_events (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       BIGINT NOT NULL REFERENCES tenants(id),
    actor_user_id   BIGINT REFERENCES users(id),
    actor_email     TEXT,
    channel         TEXT NOT NULL CHECK (channel IN ('web', 'email_agent', 'internal')),
    action          TEXT NOT NULL,
    target_type     TEXT NOT NULL,
    target_id       BIGINT,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_events_tenant_created ON audit_events (tenant_id, created_at DESC);
CREATE INDEX idx_audit_events_actor ON audit_events (actor_user_id, created_at DESC);
```

**010_add_tenant_user_to_conversations.sql:**
```sql
ALTER TABLE conversations
    ADD COLUMN tenant_id BIGINT REFERENCES tenants(id),
    ADD COLUMN user_id BIGINT REFERENCES users(id);

CREATE INDEX idx_conversations_tenant ON conversations (tenant_id);
CREATE INDEX idx_conversations_tenant_user ON conversations (tenant_id, user_id);
```

---

## 9. Käyttäjän tunnistaminen sähköpostissa

Nykyinen flow `agent.rs`:ssä käyttää `sender`-kenttää (normalisoitu email). Uusi flow:

```rust
// Ennen agent::process_email kutsua:

// 1. Etsi käyttäjä sähköpostiosoitteen perusteella
let user_info = ops::find_user_by_email(&db, &sender_email).await?;
// Palauttaa: user + tenant_users + tenant (JOIN)

match user_info {
    Some(info) if info.membership.status == "active" => {
        // Tunnistettu käyttäjä — rakenna OpContext
        let ctx = OpContext {
            actor_user_id: info.user.id,
            tenant_id: info.tenant.id,
            role: info.membership.role,
            channel: Channel::EmailAgent,
        };
        agent::process_email_with_context(&db, &client, &email, ctx).await
    }
    Some(info) if info.membership.status == "invited" => {
        // Kutsuttu mutta ei vielä aktivoinut — muistuta aktivoinnista
        send_activation_reminder(&email, &info).await
    }
    Some(info) if info.membership.status == "disabled" => {
        // Deaktivoitu — ei vastausta
        Ok(())
    }
    None => {
        // Tuntematon — ohjaa rekisteröitymään (vain jos DMARC pass)
        send_unknown_user_reply(&email).await
    }
}
```

**System promptin laajennus:**

```
Olet Grooveserven matkalaskuassistentti.

Käyttäjätiedot:
- Nimi: {user.name}
- Sähköposti: {sender_email}
- Organisaatio: {tenant.name}
- Rooli: {membership.role}

{if membership.role == "admin"}
Sinulla on käytössäsi seuraavat hallinnointityökalut:
- invite_user: Kutsu uusi käyttäjä organisaatioon
- list_users: Listaa organisaation käyttäjät
- disable_user: Deaktivoi käyttäjä
- update_user_role: Vaihda käyttäjän rooli
{endif}
```

---

## 10. Turvallisuus

### 10.1 Tenant-eristys
- **Tietokanta**: Jokainen query sisältää `WHERE tenant_id = $1`. Ei poikkeuksia. Operaatiokerros saa `tenant_id`:n `OpContext`:sta.
- **Agentin konteksti**: `tenant_id` injektoidaan email-luupissa ennen ops-kutsua. Agentti (LLM) ei voi operoida toisen tenantin datalla.
- **Sähköposti**: Käyttäjä tunnistetaan rekisteröidyllä osoitteella.

### 10.2 Tokenien turvallisuus (invitations, sessions, auth_tokens)
- Kaikki tokenit: 32 tavua kryptografisesti turvallista satunnaisuutta (`rand::rngs::OsRng`), base64url-enkoodattu
- Tietokantaan tallennetaan vain SHA-256 hash — raakaa tokenia ei koskaan tallenneta
- Invitation-tokenit vanhenevat 7 päivässä, auth-tokenit 24 tunnissa
- Kertakäyttöiset — merkitään käytetyksi hyväksynnän jälkeen

### 10.3 Session-turvallisuus
- HTTP-only, Secure, SameSite=Lax -cookie
- Session-token hashataan ennen tallennusta
- Session vanhenee 30 päivässä (configuroitava)
- Logout invalidoi session-rivin
- Salasanan vaihdon yhteydessä kaikki sessiot invalidoidaan
- Ei client-side token storage
- CSRF-token kaikissa muuttavissa pyynnöissä

### 10.4 Sähköpostitunnistuksen rajoitukset
- SPF/DKIM/DMARC antavat kohtuullisen luottamuksen lähettäjän aitoudesta
- Sähköpostispooffaus ja sen torjunta käsitellään issuessa #15
- **Agentti-admin-operaatiot**: Jotkut admin-operaatiot voivat vaatia web-vahvistuksen sähköpostikanavasta käytettynä (pending-tila). Tämä käsitellään issuessa #22.

### 10.5 Last-admin -suojaus
- Tenant_users-taulussa pitää aina olla vähintään yksi aktiivinen admin per tenant
- Tarkistetaan transaktionaalisesti `FOR UPDATE` -lukolla ennen admin-roolin muutosta tai deaktivointia
- Koskee sekä web- että agenttikanavaa

---

## 11. MVP-rajaukset ja laajennusmahdollisuudet

### MVP:ssä (Phase 1-3)
- [x] Tenant + User + TenantUser + Email -tietomalli (many-to-many -valmius)
- [x] Kutsupohjainen käyttäjähallinta (Model B: pre-created user)
- [x] Session-autentikaatio (web) hashatuilla tokeneilla
- [x] Sähköpostitunnistus (agent)
- [x] Admin-portaali: käyttäjälista, kutsut, roolit
- [x] Agent tools: invite_user, list_users, disable_user, update_user_role
- [x] Server-rendered admin-portaali (htmx)
- [x] Audit log kaikista admin-operaatioista
- [x] CSRF-suojaus, login rate limiting, Cloudflare Turnstile
- [x] Salasanan palautus
- [x] Last-admin -suojaus

### MVP-rajoitukset (tietoiset)
- Käyttäjä voi kuulua vain yhteen tenanttiin (tietomalli tukee monta, mutta UX rajoittaa)
- Globaali uniikki email (konsultit tarvitsevat eri osoitteet eri yrityksille)
- Sähköpostin plus-osoitteet (matti+tag@firma.fi) käsitellään erillisinä osoitteina (#28)

### Myöhemmin (Phase 4+)
- [ ] Tilausten hallinta ja laskutus
- [ ] Hyväksyntäkierron konfigurointi per tenant
- [ ] OAuth/SSO (Google, Microsoft)
- [ ] Row-level security PostgreSQL:ssä
- [ ] Monisähköpostidomain per tenant
- [ ] API-avaimet ulkoisille integraatioille
- [ ] Käyttäjän self-service profiilinhallinta
- [ ] Multi-tenant jäsenyys (sama käyttäjä useassa tenantissa)

---

## 12. Avoimet kysymykset

1. **Agentti-admin-operaatioiden vahvistus**: Mitkä sähköpostiagentti-operaatiot vaativat web-vahvistuksen (pending-tila)? Käsitellään issuessa #22.

2. **Multi-domain tenantit**: Voiko yksi yritys omistaa useita sähköpostidomain-osoitteita (esim. firma.fi + firma.com)? MVP:ssä ei — käyttäjät lisätään yksitellen.

3. **Web-framework timing**: Tarvitaanko axum-HTTP-palvelin ennen käyttäjähallintaa? Kyllä, koska rekisteröityminen ja kutsun hyväksyminen vaativat web-lomakkeen.

4. **Yhteisen binäärin skaalautuvuus**: Milloin email-palvelu ja web-API pitäisi erottaa? Ei MVP:ssä — seuraa kun kuorma kasvaa. Vikaantumismalli issuessa #30.

5. **Magic link vs. salasana**: Sähköpostivetoiselle palvelulle magic link -kirjautuminen voisi olla luonteva vaihtoehto salasanalle. Eliminoisi salasanojen hallinnan kokonaan. Ratkaistaan kun web-autentikaatiota toteutetaan.

---

## Liite A: Review-huomiot

Tämä design kävi läpi nelinkertaisen LLM-review:n (Gemini, GPT-5.5, Claude Opus, DeepSeek). Täydellinen review-raportti: `history/review-design-multi-tenant.md`. Tärkeimmät löydökset on integroitu tähän dokumenttiin (v2).
