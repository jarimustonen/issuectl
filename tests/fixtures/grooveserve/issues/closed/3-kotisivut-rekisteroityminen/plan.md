# Arkkitehtuurisuunnitelma: Kotisivut ja rekisteröityminen

## Lähtötilanne

- `grooveserve.com` DNS osoittaa jo Cloudflare Pagesiin (`grooveserve-website.pages.dev`)
- Pages-projekti `grooveserve-website` on konfiguroitu `operations/cloudflare/config.yaml`:ssa
- Backend-palvelut ovat Rustia (ks. `services/email/`)
- Hetzner VPS:llä pyörii PostgreSQL 17, Stalwart, ja email-service
- Myöhemmin tarvitaan `service.grooveserve.com` — rikkaampi sovellus (matkalaskut, kuitit, profiili)

## Päätös: React Router v7 + Rust API

Vertailimme viittä polkua:

| Vaihtoehto | Arvio |
|---|---|
| Staattinen HTML/CSS | Liian rajoittunut — ei komponenttimallia, ei skaalaudu |
| Leptos (Rust full-stack WASM) | Kiinnostava mutta pre-1.0, pieni ekosysteemi, hitaat kompilointiajat |
| Next.js + Rust API | Työntää kohti omaa backendia (RSC), Vercel-kytkös, turha painolasti |
| TanStack Start + Rust API | Tuore v1.0, vahva tyyppiturvallisuus, mutta pieni yhteisö ja bus factor |
| **React Router v7 + Rust API** | **Valittu** — suurin ekosysteemi, Shopify ylläpitää, Vite-pohjainen, ei ota kantaa backendiin |

### Miksi React Router v7

1. **Remix-perintö** — Remix sulautui React Router v7:ään. Loaders, actions, nested routes, SSR — kaikki sisäänrakennettu
2. **Ei taistele Rust-backendia vastaan** — ohut frontend-kerros, ei yritä omistaa palvelinlogiikkaa
3. **Vite** — nopea kehityssykli, moderni build-tooling
4. **Suurin ekosysteemi** — React-komponenttikirjastot, UI-kitit, dokumentaatio
5. **Shopify ylläpitää** — ei yhden ihmisen projekti

### Miksi ei Leptos

Rust-WASM-frontend oli houkutteleva (yksi kieli, jaetut tyypit), mutta:
- Pre-1.0, satunnaisia breaking changeja
- Kompilointiajat 5–15s vs. <1s Vite hot reload
- UI-komponenttikirjastoja vähän — joutuisi rakentamaan perusasioita itse
- rust-analyzer ei käsittele CSR/SSR-featureita hyvin

Näillä ei ole väliä backend-Rustissa, mutta frontend-kehityksessä nopea iterointisykli on kriittinen.

---

## Arkkitehtuuri

```
                    Cloudflare Pages
                    ┌──────────────────────────────────┐
grooveserve.com ──→ │  React Router v7 (SSR/SPA)        │
                    │  - Markkinointisivu                │
                    │  - Rekisteröitymislomake           │
                    └──────────┬───────────────────────┘
                               │ fetch() → API
                               ▼
                    Hetzner VPS (204.168.196.71)
                    ┌──────────────────────────────────┐
                    │  Rust API (services/api/)          │
                    │  - POST /api/register              │
                    │  - GET  /api/verify?token=...      │
                    │  - (laajenee myöhemmin)            │
                    ├──────────────────────────────────┤
                    │  PostgreSQL 17                     │
                    │  - users-taulu                     │
                    ├──────────────────────────────────┤
                    │  Stalwart + email-service          │
                    │  - vahvistussähköposti             │
                    └──────────────────────────────────┘

                    Cloudflare Pages (myöhemmin)
                    ┌──────────────────────────────────┐
service.            │  React Router v7 (SPA/SSR)        │
grooveserve.com ──→ │  - Matkalaskut, kuitit, profiili  │
                    │  - AI-chat                        │
                    │  ↕ Rust API                       │
                    └──────────────────────────────────┘
```

### Kaksi erillistä frontend-sovellusta

1. **grooveserve.com** — markkinointisivu + rekisteröityminen (tämä issue)
2. **service.grooveserve.com** — käyttäjäsovellus (myöhemmin, issue #11)

Molemmat React Router v7, mutta erilliset deploymentit. Markkinointisivun ei pidä riippua sovelluksen deploymentista.

### Yksi jaettu Rust API

`services/api/` palvelee molempia frontendejä. Aluksi vain rekisteröityminen, myöhemmin autentikointi, matkalaskut, tositteet, jne.

---

## Monorepo-rakenne

```
sites/
  www/                          # grooveserve.com (React Router v7)
    package.json
    vite.config.ts
    app/
      root.tsx                  # Layout, head, meta
      routes/
        _index.tsx              # Laskeutumissivu
        register.tsx            # Rekisteröitymislomake (tai samalla sivulla)
        verify.tsx              # Sähköpostivahvistus
      components/
        Hero.tsx
        HowItWorks.tsx
        Pricing.tsx
        RegisterForm.tsx
        Footer.tsx

services/
  api/                          # Rust API (Axum/Actix)
    Cargo.toml
    src/
      main.rs
      routes/
        register.rs             # POST /api/register
        verify.rs               # GET /api/verify
    migrations/
      001_create_users.sql
```

---

## Toteutussuunnitelma

### 1. Markkinointisivu (`sites/www/`)

React Router v7 -projekti Vite-pohjalla. Cloudflare Pagesiin deployataan pre-rendered (SSG-mode) koska sisältö on staattista.

Sivun sisältö:
- **Hero**: palvelun ydinkuvaus ("AI hoitaa matkalaskusi")
- **Miten toimii**: 3 askelta (lähetä kuitit → AI koostaa → hyväksy ja lähetä)
- **Kenelle**: kohderyhmä (yritykset)
- **Hinnoittelu**: perusmaksu + käyttöperusteinen (tarkat hinnat TBD)
- **Rekisteröitymislomake**: yritys, yhteyshenkilö, sähköposti
- **Footer**: yhteystiedot

### 2. Rust API (`services/api/`)

Uusi Rust-palvelu (Axum) joka tarjoaa:
- `POST /api/register` — validoi, tallentaa tietokantaan, lähettää vahvistussähköpostin
- `GET /api/verify?token=...` — vahvistaa sähköpostiosoitteen

Teknologiat:
- **Axum** — HTTP-framework (tokio-pohjainen, sama kuin email-servicessä)
- **sqlx** — tietokantakirjasto (compile-time query validation)
- **lettre** tai suora SMTP — sähköpostin lähetys Stalwartin kautta

### 3. Tietokanta

Uusi `users`-taulu olemassaolevaan PostgreSQL-kantaan:

```sql
CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    company_name    TEXT NOT NULL,
    contact_name    TEXT NOT NULL,
    email           TEXT NOT NULL UNIQUE,
    email_verified  BOOLEAN NOT NULL DEFAULT FALSE,
    verify_token    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 4. Sähköpostivahvistus

Rekisteröitymisen jälkeen:
1. API generoi verify_token (random, URL-safe)
2. Lähettää sähköpostin: "Vahvista sähköpostiosoitteesi: grooveserve.com/verify?token=..."
3. Käyttäjä klikkaa linkkiä → API asettaa `email_verified = true`

Sähköposti kulkee: API → Stalwart SMTP (port 587) → Mailgun relay → käyttäjä.

### 5. Deployment

- **Frontend**: `sites/www/` → Cloudflare Pages (git push, build command: `npm run build`)
- **API**: `services/api/` → Podman-kontti Hetzner VPS:llä (uusi Ansible-rooli)
- **DNS**: lisätään `api.grooveserve.com` → Hetzner VPS (`operations/cloudflare/config.yaml`)

### 6. CORS

API:n pitää sallia pyynnöt `grooveserve.com`:sta. Axum tower-http CORS middleware:

```rust
CorsLayer::new()
    .allow_origin("https://grooveserve.com".parse())
    .allow_methods([Method::POST])
    .allow_headers([header::CONTENT_TYPE])
```

---

## Päätökset

- **Kieli**: englanti
- **CSS**: Tailwind CSS
- **CORS**: Axum CORS-headerit (ei proxya)
- **Rate limiting**: ei tässä vaiheessa
- **Lomake**: yritys + yhteyshenkilö + sähköposti
