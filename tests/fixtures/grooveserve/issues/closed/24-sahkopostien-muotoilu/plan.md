# Sähköpostien muotoilu — toteutussuunnitelma

## Yhteenveto

Muutetaan email-palvelun lähetys plaintext → multipart/alternative (HTML + plaintext). AI-agentin markdown-vastaukset konvertoidaan HTML:ksi `comrak`-kirjastolla ja kääritään brändin mukaiseen HTML-templateen.

## 1. Riippuvuudet

Lisätään `Cargo.toml`:iin:
- `comrak` — markdown → HTML -konversio (GFM-yhteensopiva, turvallinen)

## 2. Templates-moduuli (`src/templates.rs`)

Uusi moduuli joka vastaa kaikesta sähköpostien muotoilusta.

### 2.1 HTML-pohja

Taulukkolayout (sähköpostien HTML-rajoitukset: ei flexbox/grid). Inline CSS.

```
┌─────────────────────────────┐
│  HEADER: Grooveserve-logo   │  bg: #4f46e5 (brand-600)
│  (teksti, ei kuvaa)         │  text: white
├─────────────────────────────┤
│                             │
│  SISÄLTÖ                    │  bg: white
│  (markdown → HTML)          │  text: #1f2937 (gray-800)
│                             │
├─────────────────────────────┤
│  FOOTER                     │  bg: #f9fafb (gray-50)
│  Grooveserve                │  text: #6b7280 (gray-500)
│  grooveserve.com            │
└─────────────────────────────┘
```

Brändivärit (sites/www/app/app.css):
- Primary: `#4f46e5` (brand-600)
- Dark: `#312e81` (brand-900)
- Light bg: `#eef2ff` (brand-50)
- Fontti: system sans-serif (Inter ei toimi sähköposteissa ilman web font -tukea)

### 2.2 Markdown → HTML pipeline

```
AI-vastaus (markdown string)
    ↓ comrak::markdown_to_html() 
    ↓ GFM extensions (taulukot, strikethrough)
    ↓ Turvallinen (no raw HTML passthrough)
    ↓ HTML-fragmentti
    ↓ wrap_html_email(fragment, message_type)
    ↓ Täysi HTML-sähköposti
```

### 2.3 Viestityyppikohtaiset templateit

Funktiot jotka tuottavat `EmailBody { html: String, plain: String }`:

- `format_ai_reply(markdown: &str) -> EmailBody` — AI-vastaus, markdown → HTML
- `format_healthcheck(from: &str, subject: &str) -> EmailBody` — healthcheck-vastaus
- `format_error() -> EmailBody` — virheilmoitus

Kaikki käyttävät samaa HTML-pohjaa (header + footer), mutta sisältö vaihtelee.

### 2.4 Footer/allekirjoitus

Kaikissa viesteissä yhtenäinen footer:
```
---
Grooveserve
grooveserve.com
```

HTML-versiossa: harmaa tausta, pieni fontti, linkki sivustolle.

## 3. SMTP-muutokset (`src/smtp.rs`)

Muutetaan `send_reply` hyväksymään `EmailBody` (HTML + plain) ja käyttämään lettren `MultiPart::alternative()`:

```rust
pub async fn send_reply(
    config: &Config,
    account: &AccountConfig,
    original: &ParsedEmail,
    subject: &str,
    body: &EmailBody,  // oli: &str
) -> Result<String>
```

Lettre multipart:
```rust
MultiPart::alternative()
    .singlepart(SinglePart::plain(body.plain))
    .singlepart(SinglePart::html(body.html))
```

## 4. Handler-muutokset (`src/handler.rs`)

`ReplyContent.body` → poistetaan. Handler ei enää tuota valmista bodya, vaan `main.rs` käyttää templates-moduulia muotoiluun. Handler tuottaa vain rakenteellisen datan (subject, viestityyppi).

## 5. Agent-muutokset (`src/agent.rs`)

- Poistetaan `ensure_signature()` — footer tulee templatesta
- `SIGNATURE`-vakio poistetaan
- `error_reply()` palauttaa pelkän tekstin ilman allekirjoitusta (template lisää)
- AI-vastaus palautetaan raakana markdownina (template konvertoi)

## 6. Main.rs -integraatio

`process_message` ja `process_assistant_reply` käyttävät templates-moduulia:
- AI-vastaus: `templates::format_ai_reply(&reply.text)`
- Healthcheck: `templates::format_healthcheck(&from, &subject)`
- Error: `templates::format_error()`

## 7. Testit

- `templates.rs`: yksikkötestit jokaiselle format-funktiolle (tarkistaa HTML + plain sisällön)
- `agent.rs`: päivitetään testit (ei enää signature-tarkistuksia)
- `handler.rs`: testit pysyvät lähes ennallaan (reply_content muuttuu)
- `smtp.rs`: ei yksikkötestejä (vaatisi SMTP-palvelimen)
