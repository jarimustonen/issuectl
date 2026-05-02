# Phase 1 — Yhdistetty schema + ops-kirjasto: arkkitehtuurianalyysi

_Issue: #56 (koordinoiva epic) | Worktree: `A1-shared-schema-analysis` | Created: 2026-04-29 | Author: jari (Claude)_

Tämä dokumentti tuottaa Phase 1:n päätösehdotukset epic #56:lle. Phase 1 on Track A:n (Foundation) **kriittinen polku** — Phase 2 (identiteetti), Phase 3 (web) ja sisarepic #57:n toteutus odottavat näitä päätöksiä. Tavoite on **päätösvalmis pohja** seuraaville worktreille, ei toteutus.

Lähteet: `#26 design.md` §2/§4.2/§5/§6.1/§9, `#43` korjaussuunnitelma, `#55` IMAP-loop -tausta, `#33` lokaali ympäristö, sekä nykyiset `services/email` ja `services/api` -hakemistot.

---

## TL;DR — keskeiset suositukset

1. **Yksi binääri (vaihtoehto A) yhdellä Cargo workspacella.** IMAP-loop ja HTTP-server pyörivät samassa tokio-runtimessa eri taskeissa, jaettu `PgPool`. #55:n blokkausriski hoidetaan task-rajalla + tulevalla worker-poolilla, ei prosessijaolla.
2. **Cargo workspace `crates/`-hakemistossa**: `crates/ops` (jaettu domain-kirjasto), `crates/server` (binääri: HTTP + IMAP-loop), `crates/email-cli` (`gs-email-cli`). `services/email` ja `services/api` puretaan tässä migraatiossa.
3. **`ops`-crate sisältää sekä #26 Phase 1.5:n nykyiset moduulit että email-puolen domain-operaatiot** (`ops::receipts`, `ops::extractions`, `ops::attachments`, `ops::conversations`, `ops::agent`). `OpContext` säilyy nykyisellään mutta `Channel` laajenee (lisätään `EmailIngest`).
4. **Schema-yhdistäminen tehdään yhdellä uudella DB:llä per env**, joka on **api-puolen Phase 1.5 -skeeman superset**. Email-puolen taulut (`receipts`/`extractions`/`attachments`/`expenses`/`user_profiles`/`conversations`/`email_processing`) kopioidaan migraationa, mutta niiden `users(id)` ja `tenants(id)` -FK:t käännetään osoittamaan api-puolen tauluihin. Email-puolen legacy-`tenants` ja -`users` poistetaan.
5. **Prod-data: ei migroida.** MVP-vaiheessa prod-DB:ssä ei ole kriittistä asiakasdataa — käytännössä testidemoja ja muutama developer-tili. Cutover on **truncate + re-bootstrap**, ei datamigraatio. (Avoin kysymys jos käyttäjällä on toinen mielipide — ks. §6.)
6. **Roadmap**: kolme seuraavaa worktreeta — `A2-ops-crate-skeleton`, `A3-shared-schema-migration`, `A4-email-uses-shared-db` — yhdessä tuovat Phase 1:n maaliin.

---

## 1. Yksi vai kaksi binääriä

### Vaihtoehdot

**A. Yksi binääri** — sama prosessi ajaa HTTP-serveriä (axum) ja IMAP-loopia (per tili). Jaettu `PgPool`, jaettu `ops::*`. Päätös vastaa #26 design.md §6.1:tä (jossa MVP-perustelu jo annettu).

**B. Kaksi binääriä, jaettu `ops`-crate ja jaettu DB.** Erilliset prosessit, erilliset deploy-yksiköt. Sama domain-kirjasto kummassakin.

**C. Kaksi binääriä, ei jaettua schemaa, REST/queue niiden välillä.** Hylätty saman tien — vastaa #43:n vaihtoehtoa D ja tuo lisää failure-modeja vastineeksi marginaalisesta eristyksestä.

### Trade-offit

| Kriteeri | A — yksi binääri | B — kaksi binääriä |
|---|---|---|
| Deploy-monimutkaisuus | 1 kontti, 1 systemd-unit, 1 healthcheck | 2 konttia, 2 unitia, kaksi liikkuvaa osaa Ansiblessa |
| Käyttöoikeudet/eristys | Sama prosessi → IMAP-creds ja web-creds samassa muistiavaruudessa | Voi rajata ympäristömuuttujia (esim. IMAP-palvelu ei tarvitse Turnstile-secrettiä) |
| **#55 IMAP-blokkaus** | IMAP-prosessoinnin pitkä task **ei blokkaa HTTP-handlereita** kunhan ne ajetaan eri tokio-taskeissa — `process_message` on `async` ja antaa pisteissä rungon takaisin runtimeen. Riski on jos joku tekee `block_on` -kutsun, mutta sitä vältetään muutenkin. | Täysi prosessieristys — HTTP ei voi blokkautua IMAP:n takia missään tapauksessa |
| **Vision-kutsujen (#55) kuormavaikutus** | Anthropic-kutsut ovat `await`-pisteitä → muut taskit skeduloituvat. Riittävä CPU-paine vain jos kuvat ladataan synkronisesti — nykyinen `extraction.rs` käyttää `reqwest::async`-clientiä → ei ongelmaa | Sama, mutta eri prosessissa |
| **DB-pool** | 1 pool, helppoa rajoittaa kustannuksia (CX23 = 4GB RAM) | 2 poolia, max-yhteyksien sumi pitää säätää |
| Kehittäjän DX (lokaali) | `cargo run -p server` käynnistää koko stackin | `cargo run -p server` + `cargo run -p email-worker` rinnakkain → vaatii orkestrointia (#33 gsdev hoitaisi tämän) |
| Migraatioiden ajo | Yksi paikka käynnistyksen yhteydessä | Kaksi paikkaa, riski ajaa tuplana → migraatiot pitää ajaa yhdestä paikasta |
| Vikaantumismalli (#26 §12 q4 + #30) | Yksi prosessi → jos kaatuu, koko palvelu kaatuu. systemd `Restart=always` riittää MVP:lle. | IMAP-puoli voi kaatua ilman web-katkoa. Mutta MVP:ssä kummatkin pitää joka tapauksessa olla pystyssä |
| Aikataulu Phase 1:lle | Yksi binääri rakennetaan kerralla, 1 worktree riittää toteutukseen | Vaatii enemmän jakotyötä: yhteinen crate, kaksi binääriä, deploy-paketointi → 2 worktree-pottia minimissään |
| Konvertoituvuus | Helppo jakaa myöhemmin: ota `email-worker`-binääri Cargo workspaceen, käytä samaa `ops`-cratea | — |

### Suositus

**Vaihtoehto A — yksi binääri.**

**Perustelut:**

- **MVP-painotus.** CLAUDE.md sanoo: "toiminnallisuus ennen optimointeja" ja "suoritusaika ei ole kriittinen". Prosessi-eristys on optimointi, ei toiminnallisuus. Yksi binääri on yksinkertaisempi käynnistää, debugata ja deployata, eikä se sulje pois mitään tulevaa.
- **#55 IMAP-blokkaus on ensisijaisesti rinnakkaisuuskysymys, ei prosessi-kysymys.** Yksi prosessi voi ajaa monta IMAP-tiliä rinnakkaisina taskeina (nykyinen `tokio::task::JoinSet` `services/email/src/main.rs:85` tekee jo niin) ja worker-poolin yhden tilin sisällä — sama ratkaisu toimii kahdessa binäärissä yhtä lailla. Jaolla ei voiteta tässä mitään olennaista.
- **#26 design.md §6.1 päätti jo "yksi binääri MVP:ssä"** ja tämä analyysi vahvistaa sen — ei syytä vaihtaa kantaa, kun kantava perustelu (jaettu DB-pool, yksi deploy) on edelleen voimassa.
- **Konvertoituvuus**: jos kuorma kasvaa, jako kahteen binääriin on triviaali Cargo workspace -muutos kun `ops`-crate on jaettu. Eli emme menetä mitään lykkäämällä päätöstä.
- **#33 lokaali stack (gsdev) on rakennettu olettaen kaksi prosessia tällä hetkellä** — yhden binäärin malli **yksinkertaistaa tätäkin** (vain yksi `cargo watch`-paneeli .workmux.yaml:ssa).

**Riski jonka hyväksymme:**

Yhden binäärin koko on suurempi (axum + sqlx + lettre + IMAP + LLM-kirjastot). Compile-aika nousee jonkin verran. Tämä on hyväksyttävää kustannus, etenkin kun `crates/ops`-jako mahdollistaa inkrementaalisen rebuildin paremmin kuin nykyinen monoliittinen `services/email` ja `services/api`.

**Mitä tämä EI sulje pois:**

- Worker-poolimallin (#55 vaihtoehto A) — sen voi rakentaa yhden binäärin sisään yhtä helposti
- Background-jonon (#55 vaihtoehto B) — DB-pohjainen jono toimii prosessien sisällä yhtä hyvin
- Tulevan jaon kahteen binääriin — `ops`-crate on jo jaettu, jako on minuuttien työ

---

## 2. Cargo workspace -rakenne

### Hakemistorakenne (ehdotus)

```
grooveserve-monorepo/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── ops/                    # jaettu domain-kirjasto (kaikki bisneslogiikka)
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── context.rs      # OpContext, Channel, UserRole
│   │   │   ├── error.rs        # OpError
│   │   │   ├── audit.rs
│   │   │   ├── auth.rs         # login, sessions, password
│   │   │   ├── tenants.rs
│   │   │   ├── users.rs
│   │   │   ├── invitations.rs
│   │   │   ├── tokens.rs
│   │   │   ├── receipts.rs     # nyk. services/email saver
│   │   │   ├── extractions.rs  # nyk. services/email extractor
│   │   │   ├── attachments.rs
│   │   │   ├── expenses.rs
│   │   │   ├── conversations.rs
│   │   │   ├── user_profile.rs
│   │   │   ├── email_processing.rs  # IMAP-claim/retry-jonon ops
│   │   │   ├── agent_runs.rs   # #57:n agent-trace (Phase 1.5+)
│   │   │   └── validate.rs
│   │   └── migrations/         # KAIKKI migraatiot ovat täällä
│   ├── server/                 # päätuotantobinääri
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs         # axum + IMAP-loop spawnataan samasta runtimesta
│   │   │   ├── http/           # vastaa nykyistä services/api/src/routes/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── admin.rs
│   │   │   │   ├── auth.rs
│   │   │   │   ├── invite.rs
│   │   │   │   ├── register.rs
│   │   │   │   ├── set_password.rs
│   │   │   │   ├── reset_password.rs
│   │   │   │   ├── me.rs
│   │   │   │   ├── settings.rs
│   │   │   │   └── user_actions.rs
│   │   │   ├── ingest/         # vastaa nykyistä services/email/src/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── imap.rs
│   │   │   │   ├── smtp.rs
│   │   │   │   ├── spam.rs
│   │   │   │   ├── handler.rs
│   │   │   │   ├── extraction.rs
│   │   │   │   └── agent/
│   │   │   ├── tools/          # LLM tool dispatch (siirretään email/src/tools/)
│   │   │   ├── middleware.rs
│   │   │   ├── web.rs          # HTML-shell, CSRF, askama
│   │   │   ├── i18n.rs
│   │   │   ├── notify.rs
│   │   │   └── config.rs       # env → Config (yksi paikka)
│   │   └── tests/
│   ├── email-cli/              # gs-email-cli (kehitystyökalu)
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   └── infra-types/            # (myöhemmin?) jaetut serde-tyypit, jos tarvitaan
├── services/                   # POISTUU Phase 1:n migraation jälkeen
│   ├── email/                  # → crates/server/src/ingest + crates/email-cli
│   └── api/                    # → crates/server/src/http
├── tools/, operations/, sites/, issues/   # ennallaan
```

### Riippuvuussuunta

```
crates/server     ──depends──▶  crates/ops
crates/email-cli  ──depends──▶  crates/ops
                                   │
                                   ▼
                            sqlx, chrono, serde, anyhow
                            (ei axum, ei reqwest, ei lettre)
```

**`crates/ops` ei riipu axumista, lettrestä, reqwestistä eikä mistään HTTP/SMTP-runkokirjastosta.** Se on puhtaasti DB-domain-kirjasto. Tämä rajaus on tärkeä:

- Ops-funktiot ovat testattavissa puhtaasti `#[sqlx::test]`-makrolla ilman HTTP-kontekstia
- Server-crate kantaa sivuvaikutus-pinnat (sähköpostin lähetys, HTTP, IMAP, Anthropic API) ja kutsuu ops-funktioita
- `crates/email-cli` voi käyttää `ops::receipts::*` ilman koko serverin riippuvuuksia

**Poikkeus**: `ops::email::send_*` -tyyppiset funktiot, joiden on lähetettävä sähköpostia, ottavat **mailer-trait-objektin** parametrina (`&dyn Mailer`). Server kuljettaa konkreettisen `lettre`-pohjaisen toteutuksen, testit kuljettavat `Vec<SentMail>`-keräävän mockin. Tämä pitää `ops`-craten `lettre`-vapaana.

### Migraatiopolku nykyisestä rakenteesta

| Vaihe | Toiminto | Worktree |
|---|---|---|
| 1 | Luo workspace-juuri-`Cargo.toml`, `crates/ops`-skeleton, siirrä `services/api/src/ops/` → `crates/ops/src/` ja sovita import-polut | A2 |
| 2 | Siirrä migraatiot `services/api/migrations/` ja `services/email/migrations/` → `crates/ops/migrations/` (renumeroidaan + de-duplikoidaan) | A3 |
| 3 | Luo `crates/server` jossa `http/` = `services/api/src/routes/` ja `ingest/` = `services/email/src/`, server kutsuu yhdestä `ops`-cratesta | A4 |
| 4 | Poista `services/email` ja `services/api` git-historian kanssa, päivitä `Dockerfile`, Ansible, gsdev-templatet | A4 / B-track update |

Tarkka renumerointi/deduplikointi on osa A3-worktreen omaa scope-suunnittelua.

---

## 3. `ops`-crate

### `OpContext` — laajennus

Nykyinen (`services/api/src/ops/context.rs`):

```rust
pub struct OpContext {
    pub actor_user_id: i64,
    pub tenant_id: i64,
    pub role: UserRole,
    pub channel: Channel,   // Web | EmailAgent | Internal
}
```

Ehdotus uudeksi muodoksi:

```rust
pub struct OpContext {
    pub actor_user_id: i64,
    pub tenant_id: i64,
    pub role: UserRole,
    pub channel: Channel,
    /// Jäljitysavain auditia ja agent_runs-taulua varten.
    /// - `EmailIngest`: viestin message-id
    /// - `Web`: request-id
    /// - `Internal`: työn nimi (e.g. "imap_idle_loop")
    pub trace_id: Option<String>,
}

pub enum Channel {
    Web,           // HTTP-pyyntö, autentikoitu sessio
    EmailAgent,    // tunnistettu sähköpostilähettäjä, agentin tool_use
    EmailIngest,   // pelkkä vastaanotto/extraction (ei vielä agentti) — uusi
    Internal,      // taustatyöt, CLI, healthcheck
}
```

**Miksi `EmailIngest` erilleen `EmailAgent`ista:**

- `EmailIngest` on **vastaanoton** vaihe ennen kuin agenttinen looppi käynnistyy: spam-tarkistus, extraction, attachment-tallennus. Tässä vaiheessa "actor" on käyttäjä (lähettäjä), mutta **kanavakohtainen politiikka** on tiukempi: ei admin-operaatioita, ei muita tooleja kuin sisäinen tallennus. Tämä erottuu selkeästi auditissa.
- `EmailAgent` on **LLM-agenttinen** vaihe: tool_use-kutsuja, jotka voivat olla mitä vain `ops::*`-funktioita. Vahvistuspolitiikka voi olla erilainen (esim. admin-operaatiot vaativat web-vahvistuksen — #22).

**`trace_id`** on Phase 1:ssä pelkkä `Option<String>`. #57:n agent-trace-suunnittelu (Phase 4) saa sen ottaa käyttöön johdonmukaisesti — mutta kenttä lisätään nyt, jotta seuraavat worktreet eivät joudu menemään takaisin lisäämään sitä jokaiseen ops-funktion signatuuriin.

### Moduulijako

```
crates/ops/src/
├── lib.rs
├── context.rs       # OpContext, Channel, UserRole
├── error.rs         # OpError
├── validate.rs      # input validation helpers (email, slug)
├── audit.rs         # ops::audit::record / record_with_email
├── auth.rs          # login, password verify, session create/resolve/destroy
├── password.rs      # argon2 hash/verify
├── token.rs         # token generate, hash_raw
├── tenants.rs       # create_tenant, get_tenant, update_tenant
├── users.rs         # find_user_by_email, list_users, update_role,
│                    #   disable_user, enable_user
├── invitations.rs   # invite_user, inspect_invitation, accept_invitation
├── password_reset.rs
├── email.rs         # email-normalisointi (lettre::Address)
├── attachments.rs   # store_attachment (sha256 dedup), get, list
├── extractions.rs   # save_extraction, get_for_attachment
├── receipts.rs      # save_receipt, list_receipts, update_receipt,
│                    #   set_status, get_revision_history (#38 kytkentä)
├── expenses.rs      # add_expense, list, update, set_status
├── conversations.rs # append_turn, list_for_thread, get_thread
├── user_profile.rs  # update_user_preferences, update_notes,
│                    #   get_user_context
├── email_processing.rs  # try_claim_message, mark_processed, retry-queue
└── agent_runs.rs    # #57:n perusta: start_run, append_step, finish_run
                     #   (skeleton Phase 1:ssä; täysi schema #57 Phase 1:n yhteydessä)
```

**Mitä siirretään:**

- `services/api/src/ops/` siirtyy lähes 1:1
- `services/email/src/db.rs`:n DB-funktiot puretaan ops-moduuleihin: `try_claim_message` → `ops::email_processing`, `is_known_sender` → `ops::users::find_user_by_email`, conversations-funktiot → `ops::conversations`
- `services/email/src/tools/` **ohuteneat** wrapper-funktiot pysyvät server-cratessa (LLM-tool-dispatch ja JSON-schema), mutta itse domain-toteutus siirtyy ops-moduuleihin. Esim. `tools::receipts::save_receipt::Tool::execute` muuttuu wrapperiksi joka kutsuu `ops::receipts::save_receipt(ctx, req)`.

### Funktiosignatuurien malli

Yhtenäinen malli kaikille operaatioille:

```rust
// crates/ops/src/receipts.rs

pub struct CreateReceiptReq {
    pub extraction_id: Option<i64>,
    pub vendor: Option<String>,
    pub receipt_date: Option<NaiveDate>,
    pub total_amount: Option<Decimal>,
    pub currency: String,            // "EUR" by default
    pub items: Option<serde_json::Value>,
    pub payment_method: Option<PaymentMethod>,
    pub category: Option<String>,
    pub source: ReceiptSource,
}

pub struct CreateReceiptOutput {
    pub id: i64,
    pub created_at: DateTime<Utc>,
}

pub async fn create_receipt(
    db: impl PgExecutor<'_>,    // Pool tai &mut Tx — kutsuja päättää
    ctx: &OpContext,
    req: CreateReceiptReq,
) -> Result<CreateReceiptOutput, OpError> {
    // 1. Authz: ctx.tenant_id antaa tenant-eristyksen
    //    (ei require_admin — käyttäjä saa luoda omia kuitteja)
    // 2. Validate (validate.rs)
    // 3. INSERT ... RETURNING id
    // 4. ops::audit::record(...)
    // 5. Palauta tulos
}
```

**Yhdenmukaiset säännöt:**

- **Ensimmäinen parametri** on aina executor (`PgPool` tai `&mut PgConnection` tai `&mut Transaction`) — `PgExecutor`-bound mahdollistaa molemmat ilman kutsujakohtaista koodia. Nykyinen api/ops noudattaa tätä — säilytetään.
- **Toinen parametri** on aina `&OpContext`. Poikkeus on `create_tenant` (rekisteröinti edeltää autentikaatiota) — se ottaa optional `Channel` -arvon.
- **Kolmas parametri** on aina `Req`-struct — ei pitkiä parametrilistoja, jotta kanavat (HTTP-formi, JSON-API, tool_use JSON) voivat deserialisoida samalle tyypille.
- **Output** on aina `Result<Output, OpError>` jossa `Output` on struct.
- **Input-validointi** (sähköpostit, slugit, valuuttakoodit) tehdään `ops::validate`:ssa — yksi paikka kaikille kanaville.
- **Audit-kirjaus** kutsutaan funktion sisällä, ei kutsujassa. Operaatio päättää itse onko se audit-kelpoinen.

### Kanavakohtaiset politiikat

`ops::*`-funktiot tarkistavat **roolin** ja **tenant-eristyksen** itse, mutta **kanavakohtainen vahvistus** on kutsujakerroksessa (#26 §2 alkuperäinen periaate). Yhteenveto:

| Politiikka | Toteutuspaikka | Esimerkki |
|---|---|---|
| Tenant-eristys (`tenant_id` aina query:ssä) | Ops-funktion sisällä | `WHERE tenant_id = $1 AND id = $2` |
| Roolitarkistus (admin-only) | Ops-funktion sisällä, `ctx.require_admin()` | `disable_user`, `update_role` |
| **Kanavakohtainen pending-tila** | Kutsujakerros (HTTP-handler tai tool-dispatcher) | EmailAgent-kanavasta tullut `disable_user` → kirjoitetaan `pending_actions`-tauluun ja vaaditaan web-vahvistus (#22) |
| CSRF-tarkistus | HTTP-middleware | Web-pyynnöissä |
| Rate-limit | HTTP-middleware | Per-IP, per-email |
| Sähköpostin SPF/DKIM-vahvistus | Ingest-handler ennen ops-kutsua | `EmailAgent`-kontekstia ei rakenneta jos auth fail |

**Miksi kanavavahvistus on kutsujassa:** sama operaatio (`disable_user`) on itse oikea ja sallittu, mutta sen turvallisuusmalli on erilainen webissä (live-sessio + CSRF) vs. sähköpostissa (DMARC + mahdollinen pending-vahvistus). Erottelu pitää `ops`-craten yksiselitteisenä ja siirtää politiikan kanavavalintaan.

---

## 4. Schema-yhdistäminen

### Nykytila

| | email-DB (`grooveserve_email_*`) | api-DB (`grooveserve_api_*`) |
|---|---|---|
| Identiteetti | `tenants(id, name, domain, settings)` <br> `users(id, tenant_id, email, name, role)` | `tenants(id, name, slug, status)` <br> `users(id, name, password_hash)` <br> `tenant_users(tenant_id, user_id, role, status)` <br> `user_emails(user_id, email, verified, ...)` |
| Sessiot | — | `sessions`, `auth_tokens`, `invitations`, `audit_events` |
| Domain (matkalasku) | `attachments`, `extractions`, `receipts`, `expenses`, `user_profiles`, `tax_rates`, `email_processing`, `conversations`, `conversation_threads` | — |
| Ristiriidat | Sama nimi `tenants`, `users` mutta eri rakenne | (vrt. vasen) |

### Tavoiteskeema

**Yksi DB per env, api-DB:n Phase 1.5 -skeema kanonisena identiteettinä.** Email-DB:n domain-taulut tuodaan mukaan, mutta `users(id)` ja `tenants(id)` -viittaukset osoittavat api-puolen tauluihin.

Konkreettisesti:

1. `tenants` ja `users` säilyvät Phase 1.5:n muodossa (api-DB).
2. `tenant_users`, `user_emails`, `invitations`, `sessions`, `auth_tokens`, `audit_events` säilyvät.
3. Email-puolen `attachments`, `extractions`, `receipts`, `expenses`, `user_profiles`, `tax_rates`, `email_processing`, `conversations`, `conversation_threads`, `user_profile_revisions`, `receipts_idempotency`, `extractions_idempotency` siirretään yhteiseen DB:hen — niiden `tenant_id BIGINT REFERENCES tenants(id)` ja `user_id BIGINT REFERENCES users(id)` jäävät, koska ne osuvat nyt **api-puolen** tauluihin.
4. Email-puolen **legacy** `tenants` ja `users` poistetaan kokonaan (ovat olemassa email-DB:ssä rinnakkain api-DB:n kanssa).
5. **Email-puolen `users.email`** korvataan `user_emails.email`-haulla (`ops::users::find_user_by_email` tekee tämän jo).
6. **Email-puolen `users.role`** korvataan `tenant_users.role`-haulla.

**Domain-erikoisuus:**

- Email-puolen `tenants(domain)`-kenttä on käytetty `services/email/migrations/007_tenants_domain_unique.sql`:ssa. Tämä sarake **siirretään `tenants`-tauluun** (`ALTER TABLE tenants ADD COLUMN domain TEXT UNIQUE`). Käytetään SPF-aligned senderin tunnistuksessa ja #43 vaihe 3:n verified-domain-flagissa.
- Email-puolen `tenants(settings JSONB)` siirretään niin ikään.

### Migraatiopolku

**Vaihe a — uusi yhteinen DB (per env)**

- Luodaan uudet DB:t: `grooveserve_main` (prod) ja per-instance `grooveserve_dev_<slug>` (gsdev).
- `gsdev` (#33) päivitetään luomaan **yksi** DB, ei kahta.
- Vanhat DB:t (`grooveserve_api_*`, `grooveserve_email_*`) säilyvät rinnakkain Phase 1:n migraation aikana ja poistetaan vasta cutoverin jälkeen.

**Vaihe b — yhdistetty migraatio-skripti**

- Kaikki nykyiset migraatiot siirretään `crates/ops/migrations/`-hakemistoon. Renumeroidaan järjestyksessä:
  - 001–008 api-puolelta (`tenants`, `users`, `tenant_users`, `user_emails`, `invitations`, `sessions`, `auth_tokens`, `audit_events`)
  - 009 `tenants.domain TEXT UNIQUE` (legacy email-tarvis)
  - 010 `tenants.settings JSONB` (legacy email-tarvis)
  - 011 `email_processing` (entinen email/001)
  - 012 `conversations` + `conversations.tenant_id/user_id` (yhdistetään email/003 + email/009)
  - 013 `attachments`, `extractions`, `receipts`, `expenses`, `user_profiles`, `tax_rates` (email/005, korjattu FK:ineen)
  - 014 `conversation_threads` (email/010)
  - 015 `user_profile_revisions` (email/011)
  - 016 `receipts_idempotency_key` (email/012, suora ALTER)
  - 017 `extractions_idempotency` (email/013)
  - 018 `agent_runs` / `agent_steps` skeleton (#57 Phase 1)
- **Kanoninen järjestys: identiteetti ennen domainia.** FK-rajat osoittavat alusta loppuun yhteen suuntaan.
- **Locales-taulu** (api/009) kulkee mukana mutta on irrallinen.

**Vaihe c — data-migraatio (jos tarpeen)**

Tämä on **avoin kysymys käyttäjälle** (ks. §6, kysymys 1):

- **Suositus: ei migroida prod-dataa.** MVP-vaiheessa prod-DB:ssä on käytännössä testidemoja ja muutama developer-tili. Cutover on TRUNCATE + re-bootstrap (rekisteröidään admin-tili uudestaan).
- Jos käyttäjä haluaa säilyttää, tarvitaan kahden DB:n yhdistämisskripti, joka:
  1. Lukee api-DB:n `tenants`/`users`/`tenant_users`/`user_emails` ja kopioi ne uuteen DB:hen säilyttäen `id`:t (`OVERRIDING SYSTEM VALUE` BIGSERIAL-sarakkeisiin).
  2. Ratkaisee email-DB:n `tenants(name)` ↔ api-DB:n `tenants(name)` mappauksen — joka voi olla 1-N (eri rivit, sama nimi). Käyttäjälle näytettävä mappausehdotus.
  3. Siirtää email-DB:n `attachments`/`extractions`/`receipts`/... rivit, **muuntaen** `user_id` ja `tenant_id` mappauksen mukaisesti.
  4. Käsittelee `BIGSERIAL`-sekvenssien resetin yhdistämisen jälkeen (`SELECT setval(...)`).

**Vaihe d — cutover**

- Stop service.
- Aja yhdistetty migraatio-skripti uutta DB:tä vasten.
- (Jos data-migraatio: aja se nyt.)
- Päivitä `DATABASE_URL` osoittamaan uutta DB:tä (yksi env-muuttuja).
- Käynnistä yksi binääri.
- Verifioi #46 round-trip + #43-skenaariot.
- Vanhat DB:t pidetään lukutilassa 7 päivää, sitten poistetaan.

### Riskit

| Riski | Vaikutus | Mitigointi |
|---|---|---|
| **Sama `id` kahdessa DB:ssä** (`users.id=1` molemmissa) | FK-eheys särkyy datamigraatiossa | Datamigraatio-skripti tarvitsee mappaustaulun + sekvenssin reset (jos säilytetään dataa). Jos truncate-cutover, ei ongelmaa |
| **Email-DB:n `users.role`-arvo** ei ole `tenant_users.role`-arvojen kanssa identtinen ('user'/'admin'/'approver') | Roolin siirto suoraan toimii, ei muunnosta | Validoi datamigraatio-vaiheessa CHECK-rikkomukset |
| **`conversations.tenant_id/user_id` NULL** vanhoissa riveissä | Kysely-koodi tarvitsee NULL-tarkistuksen | #43 vaihe 1 jo käsittelee — vanhat conversaatiot on jo NULL-skenaariossa |
| **Rinnakkaiset write-polut migraation aikana** | Email-puoli kirjoittaa email-DB:hen, api-puoli api-DB:hen → cutoverin aikana data divergoi | Maintenance window: stop service → migrate → start. MVP:ssä alle 5 minuutin downtime hyväksyttävä |
| **Migration-makro `sqlx::migrate!()`** ajaa nyt kahdesta paikasta | Yksi binääri → yksi migrate-kutsu, yksi `_sqlx_migrations`-taulu | Server-cratessa `crates/ops::migrate(&pool).await?` kutsutaan kerran startupissa |
| **`tenants(domain)` -uniikki vs api-puolen `slug`-uniikki** törmäys | Jos tenant rekisteröidään webistä ilman domainia, ja sähköpostista yritetään verifioida | Tee `domain` `NULL`-sallivaksi, uniikki vain ei-NULL-arvoille (`UNIQUE` partial index) |
| **`gsadmin password-reset` tekee suoran INSERTin auth_tokens-tauluun** | Toimii sellaisenaan kun DB on yksi, mutta env-muuttuja `GSADMIN_DIRECT_DB_URL` muuttuu (yksi sijaan kaksi) | Päivitä `tools/admin` -skripteissä DB-tunnistus |
| **Migraatioiden numerointi (sqlx)** ei salli aukkoja eikä uudelleenjärjestämistä jo ajetuissa migraatioissa | Kaikki olemassa olevat envs joutuvat menemään truncate+rebuild-polulle, tai migraatiot toteutetaan no-op:eina jos jo ajettu vanhassa DB:ssä | MVP-suositus: truncate. Tee migraatio-skripti olettaen tyhjä uusi DB |
| **Identiteetin yhdistäminen email + api**: sama käyttäjä molemmissa DB:issä eri `id`:llä | Datamigraatiossa pitää valita yksi kanoninen | Kanoniseksi tulee api-DB:n `users.id`. Email-DB:n `users` ei migroida (tunnistus tehdään `user_emails`-haulla) |

---

## 5. Roadmap — seuraavat Phase 1 -worktreet

Suositus: **kolme peräkkäistä worktreeta**, joista jokainen tuottaa testattavan välituloksen. Sarjallinen siksi, että ne koskevat samaa koodipohjaa eri kerroksilla — rinnakkaisuus johtaisi pahoihin merge-konflikteihin.

### `A2-ops-crate-skeleton`

**Tavoite:** Cargo workspace-juuri pystyssä, `crates/ops`-crate olemassa, `services/api/src/ops/` siirretty `crates/ops/src/`:hen ja `services/api`-binääri kääntyy ops-cratea vasten.

**Tuotos:**

- `Cargo.toml` (workspace root) ja `crates/ops/Cargo.toml`
- `crates/ops/src/` sisältää nykyisen `services/api/src/ops/`:n moduulit
- `crates/ops/migrations/` sisältää nykyiset api-migraatiot (renumerointi tehdään A3:ssa)
- `services/api`:n koodi importoi `grooveserve_ops::*` `crate::ops::*`-tilalta
- Nykyiset 74/74 testit vihreinä (`cargo test -p grooveserve-api`)
- **EI vielä koske email-palveluun** — sen kääntyvyys turvataan, mutta sitä ei muokata

**Onnistumisen kriteerit:** workspace kääntyy, kaikki olemassa olevat testit menevät läpi, deploy-pipeline (Ansible) ei rikkoudu (kuljettaa edelleen `services/api`-binäärin).

**Estoja:** ei.

### `A3-shared-schema-migration`

**Tavoite:** Yhden DB:n migraatio-paketti olemassa, gsdev luo yhden DB:n, **ja** api-binääri ajaa migraatiot tästä paketista yhteen DB:hen.

**Tuotos:**

- Migraatiot `crates/ops/migrations/` renumeroitu (001–017+), sisältäen email-puolen kaikki schemat **mutta** kanonisen identiteetin (api-puolen) kanssa.
- `gsdev` (#33) muutos: yksi DB per instance (`grooveserve_dev_<slug>`), poistaa `_api_` ja `_email_` -slugit
- Migraation lokaali-testi: tyhjä DB → kaikki migraatiot ajavat → kaikki taulut olemassa, FK:t johdonmukaiset
- API-binääri ajaa nyt yhdistetyt migraatiot omaan poolinsa
- Email-binääri **ei vielä käytä uutta DB:tä** — sitä ei kosketa tässä worktreessä
- Päätös prod-data-migraatiosta dokumentoituna (jos käyttäjä on vastannut §6:n kysymyksiin)

**Onnistumisen kriteerit:** uusi gsdev-instanssi käynnistyy yhdellä DB:llä, api-puoli toimii ennallaan sitä vasten, migraatiot eivät hajoa idempotency-testissä (`migrate` ajaa kaksi kertaa peräkkäin → ei muutoksia).

**Estoja:** A2 valmis.

### `A4-server-crate-and-cutover`

**Tavoite:** `crates/server` olemassa, sisältää sekä HTTP-routet että IMAP-loopin, kaikki ops-kutsut menevät `crates/ops`:n läpi, vanhat `services/api` ja `services/email` poistettu, deploy-pipeline päivitetty.

**Tuotos:**

- `crates/server/src/main.rs` käynnistää **sekä axum-serverin että IMAP-loopit** samasta tokio-runtimesta
- `crates/server/src/http/`, `crates/server/src/ingest/`, `crates/server/src/tools/`
- Email-puolen domain-toiminnot (receipts/extractions/...) refactoroitu ops-funktioihin, server-side wrappit ohuita
- `services/email` ja `services/api` -hakemistot poistettu git-historiasta säilyttäen
- `crates/email-cli` syntynyt (gs-email-cli)
- Dockerfile yhdistetty (yksi runtime-kontti)
- Ansible-roolit yhdistetty (`grooveserve` korvaa `grooveserve-email` ja `grooveserve-api`)
- gsdev .workmux.yaml -templatet päivitetty (yksi `cargo watch`-paneeli)
- Round-trip -testi (registration → email round-trip Roundcubessa) toimii
- Päätös data-migraatiosta toteutettu (joko truncate-cutover tai migraatio-skripti)
- `#43` vaiheet 2–3 voidaan sulkea `obsolete`-tilassa (option C ei enää tarvita; legitiimi käyttäjäresoluutio toimii suoraan `ops::users::find_user_by_email`-kautta)

**Onnistumisen kriteerit:** lokaali round-trip toimii, Ansible deploy onnistuu, `#43`:n quick-test ei luo haamutilejä, `#46`:n kuittiekstrahointi toimii ennallaan.

**Estoja:** A2, A3 valmiit.

### Rinnakkaisuus muiden trackien kanssa

- **Track B (#33 lokaali stack)**: A3 muuttaa gsdev-templatea — koordinoi B-trackin kanssa, ettei kaksi worktreeta editoi samaa templatea yhtä aikaa.
- **Track C (kuittipolku)**: voi edetä rinnakkain niin kauan kuin **ei** kosketa schemaan tai ops-rajapintaan. #28 monivaluutta on schema-muutos → odotetaan A3:n jälkeen tai lähetetään pre-A3 erillisenä migraationa joka ajetaan myös A3:n migraatiopaketissa. **Suositus: pidä #28 odottamassa A3:n valmistumista.**
- **Track D (#57 sisarepic)**: D1-design voi käydä rinnakkain — `agent_runs`/`agent_steps`-skeleton lisätään A3:n migraatioihin (rivi 018) niin että D2-toteutuksen schema on jo paikoillaan kun A4 valmistuu.

---

## 6. Päätökset (käyttäjän vastaukset 2026-04-29)

Alla kysymykset suosituksineen ja käyttäjän tekemät päätökset. Nämä lukitsevat A2/A3/A4-worktreiden scopen.

### 1. Prod-datan migrointi

**Päätös: ei oikeita asiakkaita, kaikki nollataan. Tyhjä DB.**

- A3-worktree olettaa tyhjän DB:n: ei datamigraatio-skriptiä, ei id-mappausta, ei säilytysjaksoja
- A4-cutover on suoraviivainen: stop service → drop old DB:t → create unified DB → run migrations → start service
- Vanhat `grooveserve_api_*` ja `grooveserve_email_*` -DB:t voidaan dropata heti A4:n jälkeen (ei 7 päivän odotusjaksoa)

### 2. Email-DB:n `users.id` ↔ api-DB:n `users.id` -yhteensovittaminen

**Päätös: ei merkitystä (kysymys 1 → tyhjä DB).** Ei mappaustaulua, ei `OVERRIDING SYSTEM VALUE` -viritystä — uudet rivit syntyvät kanonisesta api-skeemasta.

### 3. `Channel::EmailIngest` lisääminen + per-viesti-trace

**Päätös: lisätään `EmailIngest`. Lisäksi kunnianhimoinen tavoite:** agenttisen loopin per-viesti-tason analyysi pitää olla mahdollista — tämä korreloi suoraan #57:n `agent_runs`/`agent_steps`-suunnittelun kanssa.

**Vaikutukset Phase 1:n scopeen:**

- `audit_events.channel CHECK`-constraint laajenee: `('web', 'email_agent', 'email_ingest', 'internal')`
- A3:n migraatioissa rivi 018 ei ole pelkkä "skeleton vaan **täysmuotoinen `agent_runs`+`agent_steps`-skeema**, jonka #57:n design-worktree (`D1`) suunnittelee ennen A3:n valmistumista. Eli D1 ei voi enää venyä — sen tuotos (schema-luonnos) on **input A3:lle**.
- `OpContext.trace_id` on `Option<String>` Phase 1:ssä mutta sen täyttäminen on **velvollisuus**, ei hyvä tapa: jokainen `EmailIngest`/`EmailAgent`-kutsu kantaa `agent_run_id`-pohjaisen jäljitysavaimen. Web-kutsut saavat HTTP-request-id:n.
- A4-server-cratessa: kun viesti vastaanotetaan, luodaan `agent_run`-rivi heti `email_ingest`-kanavalla; agentin tool_use-loop kasvattaa `agent_steps`-rivejä `email_agent`-kanavalla saman `run_id`:n alle.

**Koordinointi #57:n kanssa:** `D1-agent-trace-design` -worktree pitää käynnistää **ennen A3:a** tai rinnakkain niin, että D1:n schema-päätökset ovat A3:n migraatiopaketissa.

### 4. `gs-email-cli`:n kohtalo + agentin testaustyökalut

**Päätös: a) — säilytetään ja LAAJENNETAAN.** Kevyt CLI on tarpeen sekä:

1. **Sähköpostin simulointi:** syöttää viesti agenttiselle loopille IMAP:n ohi (`gs dev send --from jari@x.fi --subject ... --attachment receipt.pdf`)
2. **Agentin testaaminen:** kutsua tooleja suoraan ilman LLM:ää (`gs dev tool save_receipt --json ...`), tarkastella `agent_runs`-trace yksittäisestä viestistä (`gs dev trace <run_id>`)
3. **Tenant/user setup:** nykyinen `setup-tenant` säilyy mutta uudelleenkirjoitetaan kutsumaan `ops::tenants::create_tenant` + `ops::invitations::*` (ohittaa Turnstilen, vain dev-käyttöön)

**Scope-laajennus Phase 1:lle:**

- `crates/email-cli` saa subkomennot `dev send`, `dev tool`, `dev trace` näiden lisäksi
- `ops::ingest::process_message(ctx, parsed_email)` -funktio extraktoidaan ops-crateen, jotta sekä IMAP-loop että CLI voivat kutsua sitä
- A4-worktree sisältää tämän laajennuksen — ei erillistä worktreetä

**Tämä laajennus on tärkeä #56:n omalle tavoitteelle ("päästä testaamaan business-logiikkaa") ja #57:n trace-näkymälle.**

**Avoin kysymys käyttäjälle:** Sopiiko että nimi muuttuu `gs-email-cli` → `gs` (yleisempi devaajan CLI) vai pidetäänkö nykyinen nimi? Suositukseni: nimi `gs-dev` selkeyttää että se on dev-käyttöön, ei prodiin.

### 5. Migraatioiden renumerointi

**Päätös: oletetaan tyhjä DB.** A3 renumeroi migraatiot uusiksi 001–N. Ei `_sqlx_migrations`-yhteensovittamista, ei no-op-migraatioita.

### 6. `#26 §6.1 q4` ja `#30` single-binary supervision

**Päätös: systemd `Restart=always` riittää Phase 1:ssä.** #30 (graceful shutdown + task-supervision) jää myöhempään, ei kuulu A2/A3/A4-scopeen.

### 7. Roundcube-, Mailpit- ja GreenMail-kytkentä `#33`-trackissä

**Päätös: env-template-muutos sisältyy A4:ään.** Ei erillistä B-worktreetä env-yhdistämistä varten — pidetään cutover yhdessä paikassa, vältetään puolitiehen-tila gsdev-skripteissä.

---

## Liite — Lähdetiedostoon viitanneet kohdat

Tarkistuksen helpoksi viitelista lähdetiedostoissa:

- `services/email/src/main.rs:85` — `tokio::task::JoinSet`-spawn per IMAP-tili (#55-tausta, vaikuttaa §1:n suositukseen)
- `services/email/src/db.rs:34` — `is_known_sender` (siirtyy `ops::users::find_user_by_email`:n alle)
- `services/api/src/ops/context.rs:11` — `Channel`-enum (laajennetaan §3:n mukaan)
- `services/email/migrations/005_expense_report_phase1.sql` — domain-skeema joka siirretään yhteiseen DB:hen
- `services/email/migrations/009_add_tenant_user_to_conversations.sql` — kommenttirivi §6.1-konsolidointiin viitaten (vahvistaa että tämä on jo suunniteltu)
- `services/api/src/ops/mod.rs` — nykyinen ops-rakenne
- `issues/open/26-multi-tenant-kayttajahallinta/design.md` §6.1 — alkuperäinen "yksi binääri MVP:ssä" -päätös
