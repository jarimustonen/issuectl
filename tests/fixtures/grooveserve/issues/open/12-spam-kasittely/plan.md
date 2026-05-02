# Spam-käsittely — suunnitelma

> Päivitetty LLM-reviewin (4 mallia, 2 kierrosta) löydösten perusteella.
> Review: `history/review-spam-kasittely-plan.md`

## Nykytila

Stalwartin sisäänrakennettu spam-filtteri on disabloitu (`[spam-filter] enable = false`). Kaikki viestit tulevat läpi Stalwartista ja email-service käsittelee ne `handler.rs`-moduulissa. Nykyinen reititys:

- Reply loop -suodatus (Auto-Submitted, Precedence, List-Id, no-reply-lähettäjät)
- Reititys vastaanottajan mukaan: healthcheck → auto-reply, postmaster → deliver, assistant → auto-reply

Mitään varsinaista spam-suodatusta ei ole.

### Nykyisen koodin bugit (korjattava ennen spam-työtä)

Reviewissä löytyi useita bugeja nykyisessä email-servicessä jotka pitää korjata ennen tai osana spam-toteutusta:

1. **Vastaanottajan tunnistus `To:`-headerista** — `handler::route()` käyttää `email.to.split('@').next()` joka on hyökkääjän kontrolloitavissa. Pitää käyttää `account.name` (IMAP-tili josta viesti haettiin).

2. **Catch-all auto-reply** — `_ => Action::Reply` vastaa kenelle tahansa `*@grooveserve.com`-osoitteeseen. Tekee palvelusta backscatter-lähteen. Pitää muuttaa `_ => Action::Skip` tai `Action::Deliver`.

3. **`should_skip_sender` sekoittaa "älä vastaa" ja "älä käsittele"** — `@grooveserve.com`-lähettäjien viestit siirretään Skipped-kansioon, vaikka sisäiset viestit (hyväksyntäpyynnöt, ilmoitukset) pitäisi toimittaa. Pitää erottaa reply-eligibility ja delivery-disposition.

4. **Message-ID dedup on globaali** — `is_processed(pool, &parsed.message_id)` ei huomioi tiliä. Sama Message-ID eri tileille → toinen ohitetaan. Pitää namespacettaa `(account, message_id)`.

5. **Fallback Message-ID -kollisiot** — `generate_fallback_id()` käyttää millisekuntitimestampia. Pitää käyttää UUID:tä.

6. **`Action::Deliver` ei persistoi** — viesti jää INBOXiin, haetaan uudelleen seuraavalla IDLE-syklillä, ja dedup-logiikka siirtää sen Processed-kansioon. Pitää merkitä `\Seen` tai siirtää erilliseen kansioon.

## Vaatimukset

### Osoitetyypit ja politiikat

**Järjestelmäosoitteet** — kiinteät, tunnettu käyttäytyminen:

| Osoite | Spam-politiikka | Perustelu |
|--------|----------------|-----------|
| `healthcheck@` | Ei filtteriä | Pitää vastata aina — monitorointi rikkoutuu jos filtteri estää |
| `assistant@` | Auth-tarkistus + turvallisuusgate | Korkein riski: triggeröi automaattista käsittelyä, myöhemmin AI/tool-kutsuja |
| `postmaster@` | Kevyt filtteri, ei auto-replya | DMARC-raportit, DSN:t, abuse-ilmoitukset — eivät saa jäädä filtteriin |

**Käyttäjäosoitteet** (jari@, tero@, ...) — yrityskäyttäjien henkilökohtaiset postilaatikot:

| Osoite | Spam-politiikka | Perustelu |
|--------|----------------|-----------|
| `<käyttäjä>@` | Auth-tarkistus + AI-triage (myöhemmin) | Normaalia yrityssähköpostia + matkalaskuun liittyviä viestejä |

Käyttäjäosoitteet eroavat järjestelmäosoitteista:
- Niitä on dynaamisesti N kappaletta (uusia käyttäjiä tulee palveluun)
- Vastaanottavat sekä palvelun sisäisiä viestejä (hyväksyntäkierrot, ilmoitukset) että ulkopuolista postia
- Tarvitsevat kunnollisen spam-suodatuksen — käyttäjä ei halua spämmiä postilaatikkoonsa
- Mutta eivät saa filtteröidä palvelun omia viestejä (grooveserve.com-domainista)

### Yleiset vaatimukset

- Matkalaskupalvelun viestit (kuitit, laskut, kulutusilmoitukset, hyväksyntäpyynnöt) eivät saa jäädä filtteriin
- Palvelun sisäiset viestit (@grooveserve.com) ohittavat spam-filtterin **vain jos autentikoitu** (DKIM/SPF aligned)
- MVP:lle riittää yksinkertainen ratkaisu
- Arkkitehtuurin pitää tukea myöhempää laajennusta (AI-luokittelu)
- Käyttäjäosoitteiden spam-politiikka ei saa vaatia per-käyttäjä konfiguraatiota — oletuspolitiikan pitää toimia

### Rajaus: post-delivery triage, ei SMTP-rejection

MVP:n spam-käsittely on **post-delivery triage**: viestit on jo vastaanotettu ja tallennettu Stalwartiin ennen luokittelua. Tämä tarkoittaa:
- Ei SMTP `550`-hylkäystä
- Stalwartin levytila kuormittuu spämistä
- Tarvitaan: max message size Stalwartissa, mailbox-quotat, Junk-retentio

Tämä on hyväksyttävä MVP-rajoitus. Myöhemmin Rspamd voi tehdä SMTP-tason suodatusta.

## Vaihtoehtojen vertailu

### 1. Stalwartin oma spam-filtteri koulutettuna

**Idea**: Enabloidaan Stalwartin spam-filtteri ja koulutetaan Bayesian-malli.

| + | - |
|---|---|
| Sisäänrakennettu, ei ulkoisia riippuvuuksia | Vaatii training-dataa jota ei ole |
| Per-käyttäjä asetukset mahdollisia | Konfiguraatio RocksDB:ssä — vaikea hallita deklaratiivisesti |
| | Ei tue per-osoite politiikkoja ilman Sieve-sääntöjä |
| | Perinteinen Bayesian ei ymmärrä matkalaskukontekstia |

**Arvio**: Ei sovellu. Training-dataa ei ole, ja perinteinen Bayesian-filtteri luokittelisi kuitit ja laskut spämiksi. Stalwartin konfiguraatio-ongelmat (DB voittaa config.toml:n) tekevät hallinnasta hankalaa.

### 2. Kolmannen osapuolen spam-palvelu (Rspamd / SpamAssassin)

**Idea**: Asennetaan Rspamd tai SpamAssassin Stalwartin eteen tai rinnalle.

| + | - |
|---|---|
| Kypsä, hyvin testattu | Uusi infra-komponentti (kontti, konfiguraatio, ylläpito) |
| Rspamd: hyvä API, Lua-laajennukset | Integraatio Stalwartiin vaatii milter-protokollaa tai pre-processing |
| Hyvä oletusdetektio (SPF/DKIM/DMARC, URL-tarkistukset, RBL) | Per-osoite politiikat vaativat erillistä konfiguraatiota |
| Ratkaisee monta ongelmaa kerralla (AR-parsinta, scoring, RBL) | Ylimitoitettu MVP-vaiheessa |

**Arvio**: Hyvä ratkaisu myöhemmälle vaiheelle. Rspamd ratkaisee monta ongelmaa (AR-parsinta, RBL, URL-tarkistukset) jotka muuten pitää toteuttaa itse. Jos custom-logiikka kasvaa liikaa, Rspamd voi olla vähemmän työtä pitkällä aikavälillä.

### 3. AI-pohjainen luokittelu (LLM)

**Idea**: LLM arvioi jokaisen viestin osana email-servicen käsittelyä.

| + | - |
|---|---|
| Ymmärtää matkalaskukontekstin | Latenssi ja kustannus per viesti |
| Voi erottaa kuitin spämistä | Riippuvuus ulkoiseen API:in |
| Luonnollinen osa agentin työnkulkua | Prompt injection -riski (sähköpostin body on hostile input) |

**Arvio**: Oikea ratkaisu assistant@-osoitteelle osana AI-agentin käsittelyä. Käyttäjäosoitteille toimii "triage"-mallina: rule-based karsii ilmeisen spamin, LLM arvioi epäselvät tapaukset. Kustannus pysyy hallinnassa koska LLM:ää kutsutaan vain rajatapauksissa.

### 4. Rule-based email-service-tasolla

**Idea**: Lisätään email-serviceen rule-based spam-tarkistukset. Hyödynnetään Stalwartin jo tekemiä SPF/DKIM/DMARC-tarkistuksia Authentication-Results -headerin kautta.

| + | - |
|---|---|
| Ei ulkoisia riippuvuuksia | Ei sisältöpohjaista analyysiä — vain autentikointi + säännöt |
| Per-osoite politiikat luontevasti | Ylläpito vaatii manuaalisia sääntöpäivityksiä |
| Nopea — ei verkkolatenssia | Autentikoitu spam (oikea SPF/DKIM halvalta domainilta) menee läpi |
| Helppo toteuttaa ja testata | |

**Arvio**: Paras MVP-ratkaisu. Rehellisesti rajattuna: tämä on **autentikointipohjainen triage**, ei täysi spam-filtteri. Estää spoofatun ja väärennetyn postin, mutta ei sisältöpohjaista spämmiä.

## Suositus: Kerroksittainen malli (auth-triage + AI-triage)

Yhdistelmä joka skaalautuu järjestelmäosoitteista käyttäjäosoitteisiin:

```
Kerros 1: Authentication-tarkistus (nopea, aina, ensin)
    ↓ Parsitaan Stalwartin Authentication-Results (authserv-id validoitu)
    ↓ SPF/DKIM/DMARC-tulokset → strukturoitu AuthResult

Kerros 2: Ohitussäännöt (nopea, vaatii kerros 1:n tulokset)
    ↓ grooveserve.com + DKIM/SPF aligned → luotettu sisäinen
    ↓ healthcheck@ → ei filtteriä
    ↓ allow-lista + auth aligned → läpi

Kerros 3: Politiikkapäätös (per-osoite)
    ↓ DMARC p=reject + fail → Spam
    ↓ DMARC p=quarantine + fail → Junk (käyttäjäosoitteet)
    ↓ Autentikoitu mutta tuntematon → Clean (MVP), AI-triage (myöhemmin)
    ↓ Ei autentikointia → Suspicious

Kerros 4: AI-triage (myöhemmin, vain käyttäjäosoitteet + Suspicious)
    ↓ LLM arvioi: spam / ham / epävarma
    ↓ Epävarma → Junk-kansio, käyttäjä päättää
```

### Miksi autentikointi ensin, bypass-säännöt sen jälkeen

Alkuperäinen suunnitelma teki Layer 1:n (bypass) ennen Layer 2:ta (auth). Tämä on turvallisuusaukko: `@grooveserve.com`-bypass `From:`-headerista on triviaalisti spoofable. **Autentikointi pitää tehdä ensin**, ja bypass-säännöt voivat luottaa vain autentikoituihin identiteetteihin.

### Per-osoite politiikat

| Vastaanottaja | Auth | Bypass | DMARC p=reject | Suspicious | Auto-reply |
|---------------|------|--------|----------------|------------|------------|
| `healthcheck@` | - | ohitus | - | - | kyllä (allowlist/token) |
| `postmaster@` | kyllä | DMARC-raportit | Spam | deliver | ei koskaan |
| `assistant@` | kyllä | sisäinen auth | Spam | ei auto-replya | vain Clean |
| `<käyttäjä>@` | kyllä | sisäinen auth | Spam | Junk (MVP) / AI (myöhemmin) | ei |
| tuntematon | kyllä | - | Spam | Skip | ei koskaan |

### MVP (nyt): Kerrokset 1 + 2 + 3

#### Arkkitehtuuri

```
Stalwart vastaanottaa viestin
    ↓ SPF/DKIM/DMARC-tarkistus (Stalwart tekee tämän)
    ↓ tallentaa mailboxiin (RocksDB)
email-service lukee IMAP:lla (account.name = vastaanottaja)
    ↓ parsii viestin + AR-headerit (email.rs)
    ↓ 1. tunnistaa vastaanottajan: account.name (EI email.to)
    ↓ 2. parsii AR: authserv-id validointi (spam.rs)
    ↓ 3. bypass-säännöt: sisäinen auth, allow-lista (spam.rs)
    ↓ 4. politiikkapäätös per-osoite (spam.rs)
    ↓ 5. reititys + reply-eligibility (handler.rs)
    ↓ 6. toiminto: deliver/junk/skip + reply/no-reply (main.rs)
```

#### Komponentit

**1. `ParsedEmail` laajennus** (`email.rs`)

```rust
pub struct ParsedEmail {
    // ... nykyiset kentät ...
    pub authentication_results: Vec<String>,  // KAIKKI AR-headerit (Vec, ei Option)
    pub return_path: Option<String>,          // Envelope sender
}
```

**2. Strukturoitu `AuthResult`** (`spam.rs`)

AR-headerit parsitaan strukturoituun muotoon. Vain Stalwartin oma header (authserv-id = hostname) luotetaan.

```rust
pub struct TrustedAuthResult {
    pub spf: AuthStatus,
    pub dkim: AuthStatus,
    pub dkim_domain: Option<String>,  // DKIM d= domain (alignment check)
    pub dmarc: AuthStatus,
    pub dmarc_policy: DmarcPolicy,    // p=none/quarantine/reject
}

pub enum AuthStatus {
    Pass,
    Fail,
    SoftFail,
    Neutral,
    None,       // Ei tarkistusta (header puuttuu)
    TempError,
    PermError,
}

pub enum DmarcPolicy {
    None,        // p=none — domain ei pyydä enforcementia
    Quarantine,  // p=quarantine
    Reject,      // p=reject
    Unknown,     // Ei saatu selville AR-headerista
}

/// Parsii VAIN luotetun AR-headerin (authserv-id match).
pub fn parse_trusted_ar(
    headers: &[String],
    trusted_authserv_id: &str,
) -> Option<TrustedAuthResult> { ... }
```

**Huom:** Harkittava `mail-auth`-cratea AR-parsintaan — Stalwart itse käyttää sitä.

**3. Spam-arviointi** (`spam.rs`)

```rust
pub enum SpamVerdict {
    Clean,
    Suspicious { reason: &'static str },
    Spam { reason: &'static str },
}

/// Sisäisen lähettäjän tunnistus — vaatii autentikoinnin.
fn is_trusted_internal(email: &ParsedEmail, auth: &TrustedAuthResult) -> bool {
    email.from_domain() == Some("grooveserve.com")
        && (auth.dkim_aligned_pass("grooveserve.com")
            || auth.spf == AuthStatus::Pass)  // SPF aligned
}

/// Allow-lista — vaatii autentikoinnin.
fn is_allowlisted(email: &ParsedEmail, auth: &TrustedAuthResult) -> bool {
    let domain = email.from_domain();
    ALLOWLIST.contains(domain)
        && (auth.dmarc == AuthStatus::Pass
            || auth.dkim_aligned_pass(domain))
}

pub fn evaluate(
    email: &ParsedEmail,
    auth: &TrustedAuthResult,
    recipient: &str,           // account.name, EI email.to
) -> SpamVerdict { ... }
```

**4. Reply-eligibility erillinen päätös** (`handler.rs`)

Reply-eligibility ja spam-verdict ovat erillisiä päätöksiä:

```rust
pub enum ReplyEligibility {
    CanReply,
    NoReply { reason: &'static str },
}

pub struct Decision {
    pub mailbox: MailboxDisposition,
    pub reply: ReplyEligibility,
    pub status: &'static str,
}

pub enum MailboxDisposition {
    LeaveInInbox,       // postmaster, user: normaali toimitus
    MoveToProcessed,    // healthcheck, assistant: käsitelty
    MoveToSkipped,      // reply-loop, no-reply sender
    MoveToJunk,         // spam/suspicious
}
```

**5. Per-osoite politiikat** (`handler.rs`)

```rust
pub fn decide(
    email: &ParsedEmail,
    spam: &SpamVerdict,
    recipient: &str,  // account.name
) -> Decision {
    // 1. Spam → Junk kaikille (paitsi healthcheck)
    if recipient != "healthcheck" {
        if let SpamVerdict::Spam { .. } = spam {
            return Decision {
                mailbox: MailboxDisposition::MoveToJunk,
                reply: ReplyEligibility::NoReply { reason: "spam" },
                status: "spam",
            };
        }
    }

    // 2. Suspicious → per-osoite politiikka
    if let SpamVerdict::Suspicious { .. } = spam {
        match recipient {
            "healthcheck" => { /* ohitus */ }
            "postmaster" | "dmarc" | "abuse" => { /* deliver normaalisti */ }
            "assistant" => {
                return Decision {
                    mailbox: MailboxDisposition::LeaveInInbox,
                    reply: ReplyEligibility::NoReply { reason: "suspicious auth" },
                    status: "suspicious",
                };
            }
            _ => {
                // Käyttäjäosoitteet: Junk (MVP), AI-triage (myöhemmin)
                return Decision {
                    mailbox: MailboxDisposition::MoveToJunk,
                    reply: ReplyEligibility::NoReply { reason: "suspicious" },
                    status: "suspicious",
                };
            }
        }
    }

    // 3. Clean → normaali reititys per-osoite
    match recipient {
        "postmaster" | "dmarc" | "abuse" => Decision {
            mailbox: MailboxDisposition::LeaveInInbox,
            reply: ReplyEligibility::NoReply { reason: "postmaster" },
            status: "delivered",
        },
        "healthcheck" => Decision {
            mailbox: MailboxDisposition::MoveToProcessed,
            reply: ReplyEligibility::CanReply,
            status: "reply_sent",
        },
        "assistant" => Decision {
            mailbox: MailboxDisposition::MoveToProcessed,
            reply: ReplyEligibility::CanReply,  // MVP: ping-pong
            status: "reply_sent",
        },
        _ => Decision {
            // Käyttäjäosoitteet: toimita, ei auto-replya
            mailbox: MailboxDisposition::LeaveInInbox,
            reply: ReplyEligibility::NoReply { reason: "user mailbox" },
            status: "delivered",
        },
    }
}
```

#### DMARC-politiikan mukainen käsittely

| DMARC tulos | Sender policy | Verdict |
|-------------|--------------|---------|
| pass | mikä tahansa | auth ok (jatkaa bypass/clean-polkuun) |
| fail | p=reject | `Spam` — domain-omistaja kieltää |
| fail | p=quarantine | `Suspicious` → Junk käyttäjille |
| fail | p=none | `Suspicious` → lokitetaan, ei Junk (domain ei pyydä enforcementia) |
| none/temperror | - | `Suspicious` → lokitetaan |

**Huom:** Jos Stalwartin AR-header ei sisällä DMARC-politiikkaa, tarvitaan joko DNS-lookup tai konservatiivinen oletuskäsittely (`Suspicious`, ei `Spam`).

#### Null reverse-path (DSN/bounce) -käsittely

```rust
if email.return_path == Some("<>".to_string()) || email.return_path == Some("".to_string()) {
    // DSN/bounce — ei koskaan auto-replya
    return Decision {
        mailbox: MailboxDisposition::LeaveInInbox,
        reply: ReplyEligibility::NoReply { reason: "DSN/bounce" },
        status: "delivered",
    };
}
```

#### Toiminto spam-viestille

- `Spam` → siirretään Junk-kansioon, DB `status = 'spam'`, reason tallennetaan
- `Suspicious` → käyttäjäosoitteille Junk (helppo palautus), muille deliver + lokitus
- `Clean` → normaali käsittely

#### Junk-kansion lifecycle

- **Kansio**: `Junk` (Stalwartin `\Junk` special-use)
- **Retentio**: 30 päivää, siivotaan cron-työnä
- **False positive -palautus**: käyttäjä siirtää viestin Junk → INBOX (Roundcubessa). Myöhemmin: email-service havaitsee siirron ja tallentaa korjauksen DB:hen.
- **Quota**: Junk lasketaan mailbox-quotaan

#### Muutokset tiedostoittain

| Tiedosto | Muutos |
|----------|--------|
| `src/email.rs` | AR-headerit `Vec<String>`, Return-Path, from_domain() helper |
| `src/spam.rs` | Uusi: `TrustedAuthResult`, `parse_trusted_ar()`, `evaluate()`, `is_trusted_internal()` |
| `src/handler.rs` | Refaktoroi: `route(email, recipient)`, `Decision`-tyyppi, reply-eligibility erillinen, poista catch-all auto-reply |
| `src/main.rs` | `account.name` reititysparametriksi, Junk-kansion luonti, dedup `(account, message_id)` |
| `src/db.rs` | spam-status, spam_reason, recipient-sarake, dedup-key laajennus |
| `migrations/` | Uusi migraatio: `spam_verdict`, `spam_reason`, `recipient`, unique constraint |

#### Esivalmistelu: Stalwart AR-header -verifiointi

> **Toteuttava agentti**: Tarkista tämä toteutuksen alussa (vaihe 1). Stalwartin
> AR-header -käyttäytyminen `spam-filter = false` -tilassa ei ole verifioitu.

Varmistettava ennen spam.rs-parserin kirjoittamista:

```
1. Lähetetään testiviesit Stalwartin läpi:
   - SPF pass, DKIM pass, DMARC pass
   - SPF fail, DKIM fail
   - DMARC fail (p=reject domain)
   - Viesti ilman autentikointia
   - Viesti injektoidulla fake AR-headerilla
2. Tallennetaan Stalwartin generoimat AR-headerit test-fixtureiksi
3. Varmistetaan authserv-id (Stalwartin hostname)
4. Varmistetaan AR-headerin sijainti (prepend vs append)
5. Jos Stalwart EI tuota AR-headeria spam-filter=false -tilassa,
   tutkitaan voidaanko SPF/DKIM/DMARC-tarkistukset enabloida
   ilman varsinaista spam-filtteriä
```

### Vaihe 2: Kerros 4 — AI-triage käyttäjäosoitteille

Kun LLM-integraatio on paikallaan (assistant@-agentin yhteydessä):

1. AI-triage kutsutaan **vain** kun kerros 3 palauttaa `Suspicious` käyttäjäosoitteelle
2. LLM saa **vain metadatan** (ei koko viestiä — tietosuoja):
   - From domain
   - Subject
   - Body snippet (ensimmäiset N merkkiä)
   - Liitteiden metatiedot (tyyppi, koko — ei sisältöä)
   - Auth-yhteenveto
3. LLM-vastaus pakotettu JSON-schemaan (ei vapaamuotoista):
   ```json
   { "classification": "ham|spam|uncertain", "reason_code": "..." }
   ```
4. Prompt sisältää matkalaskukontekstin ja on eristetty prompt injectionilta:
   - Ei tool-kutsuja spam-luokittelun aikana
   - Sähköpostin body on "hostile input" -kontekstissa
   - Matala lämpötila, strukturoitu vastaus

**Kustannus:** Haiku-tason malli. Todellinen Suspicious-osuus pitää mitata MVP:ssä ennen kustannusarviota (alkuperäinen "90% ratkeaa kerroksissa 1-3" on verifioitava).

**Feedback-mekanismi:** DB:hen `user_corrected_verdict`, `corrected_at` -sarakkeet. Myöhemmin: per-lähettäjä reputaatio-cache.

### Vaihe 3 (tarvittaessa): Rspamd

Jos sähköpostiliikenne kasvaa merkittävästi:

1. Rspamd-kontti Podman-stackiin
2. Rspamd hoitaa raskaan analyysin (URL-tarkistukset, RBL, Bayesian) kerroksena 2.5
3. AI-triage vain Rspamdin "epävarmat"
4. email-service tekee lopullisen päätöksen per-osoite politiikan mukaan

## Toteutusjärjestys

### Vaihe 0: Nykyisen koodin bugfix (ennen spam-työtä)

0. **Stalwart AR-header -verifiointi**: testaa ja tallenna fixturet
1. **`handler.rs`**: Muuta `route()` ottamaan `recipient: &str` parametri (`account.name`)
2. **`handler.rs`**: Poista `_ => Action::Reply` catch-all, korvaa `_ => Action::Skip`
3. **`handler.rs`**: Erota `should_skip_sender` reply-eligibilityksi (ei estä deliveryä)
4. **`main.rs` + `db.rs`**: Dedup-key `(account, message_id)`, UUID fallback-ID
5. **`main.rs`**: `Action::Deliver` → merkitse `\Seen` tai siirrä

### Vaihe 1: Auth-triage (kerrokset 1-3)

6. **`email.rs`**: AR-headerit `Vec<String>`, Return-Path, from_domain()
7. **`spam.rs`**: `TrustedAuthResult`, `parse_trusted_ar()`, `is_trusted_internal()`, `evaluate()`
8. **`handler.rs`**: Refaktoroi `Decision`-tyypillä, integroi spam-arviointi
9. **`main.rs`**: Junk-kansion luonti, spam-käsittely `Decision`-tyypin mukaan
10. **`db.rs` + migraatio**: spam_verdict, spam_reason, recipient
11. **Testit**: Yksikkötestit spam.rs:lle oikeilla Stalwart AR-header -fixtureilla

### Vaihe 2: AI-triage

12. **`spam.rs`**: `evaluate_with_ai()` — async, kutsutaan vain Suspicious + käyttäjäosoite
13. **LLM-integraatio**: Haiku, strukturoitu JSON-vastaus, prompt injection -suojaus
14. **`db.rs`**: user_corrected_verdict, corrected_at
15. **Testit**: AI-triage mockattuna

### Avoimet arkkitehtuurikysymykset

Nämä pitää ratkaista ennen käyttäjäosoitteiden tuotantokäyttöä:

1. **Dynaamiset käyttäjätilit**: Nykyinen `ACCOUNTS`-ympäristömuuttuja on staattinen. Vaihtoehdot: (A) catch-all postilaatikko + envelope-recipient -metadata, (B) dynaaminen tilien haku DB:stä + task-lifecycle. Päätös tarvitaan.
2. **ARC-tuki**: Edelleenlähetetty posti rikkoo SPF/DMARC. ARC (RFC 8617) säilyttää autentikoinnin. Stalwart tukee ARC:ia. Tarvitaan ennen kuin DMARC-enforcement on tiukka.
3. **Allow-listan hallinta**: DB-taulu vs config-tiedosto vs hardcoded. DB on joustavin.
4. **Rate limiting**: Per-sender/domain rate limit auto-replyille ja AI-kutsuille.
