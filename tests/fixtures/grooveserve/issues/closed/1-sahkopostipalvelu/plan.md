# Plan: Email Architecture v2 — Stalwart + PostgreSQL

## Tilanne 2026-04-25

### Mikä on tehty (kaikki valmiina)

#### Phase 1: Infrastruktuuri

- [x] **SOPS-salaisuudet** — `operations/secrets/stalwart.enc.yaml` luotu
  - stalwart_admin_password, postgres_password, email_service_password, mailgun_smtp_user/password
- [x] **Cloudflare DNS** — `operations/cloudflare/config.yaml` päivitetty ja applied
  - `mail.grooveserve.com` A → 204.168.196.71
  - MX → mail.grooveserve.com (priority 10)
  - SPF päivitetty: `v=spf1 include:mailgun.org a:mail.grooveserve.com ~all`
  - Vanha mxa.eu.mailgun.org MX-tietue poistettu manuaalisesti
- [x] **Ansible mail-rooli** — `roles/mail/` luotu frondeo-mallin mukaan
  - docker-compose.yml.j2 (Stalwart + PostgreSQL 17 + Roundcube)
  - nginx reverse proxy, certbot, backup, healthcheck, Roundcube-konffit
- [x] **Ansible common-rooli** — päivitetty
  - podman-compose, certbot, nginx, netcat lisätty
  - Extra-portit UFW:iin (25, 80, 443, 465, 587, 993)
  - SSH hardening laajennettu
- [x] **Ansible email-service -rooli** — `roles/email-service/` luotu
  - Rust-palvelun build + deploy Podman-konttina `--network host`
- [x] **host_vars, group_vars, email.yml** — kaikki päivitetty

#### Phase 2: Email-palvelun uudelleenkirjoitus

- [x] **Cargo.toml** — Mailgun-riippuvuudet korvattu: async-imap, lettre, sqlx, mail-parser
- [x] **main.rs** — IMAP IDLE -loop, config from env
- [x] **imap.rs** — async-imap + async-native-tls, connect/idle/fetch/move
- [x] **smtp.rs** — lettre STARTTLS, Auto-Submitted header
- [x] **handler.rs** — Reply loop -esto (Auto-Submitted, Precedence, List-Id), postmaster/dmarc/abuse → Deliver
- [x] **email.rs** — mail-parser, ParsedEmail struct
- [x] **db.rs** — sqlx migrations, email_processing -taulu, dedup message_id:llä
- [x] **mailgun.rs** — poistettu
- [x] **Dockerfile** — päivitetty migrations-hakemistolla
- [x] **Kääntyy** — `cargo build --release` OK

#### Phase 3: Deployment

- [x] **Ansible-playbook ajettu** — `ok=50 changed=6 failed=0`
  - Stalwart, PostgreSQL, Roundcube kontit pyörivät
  - TLS-sertifikaatti hankittu (Let's Encrypt)
  - Nginx pyörii, Roundcube saavutettavissa
  - grooveserve-email kontti pyörii, PostgreSQL yhdistetty, migraatiot ajettu
- [x] **Stalwart-tilit luotu** API:n kautta
  - Domain: grooveserve.com
  - Tilit: noreply, healthcheck, postmaster
- [x] **Stalwart-konfiguraatio** — hostname, TLS-sertifikaatti, Mailgun relay lisätty config.toml:iin

### IMAP-autentikointi — RATKAISTU 2026-04-25

**Ongelma:** grooveserve-email -palvelu ei pysty kirjautumaan Stalwart IMAP:iin.

**Juurisyyt (3 kpl):**

1. **Podman-DNS ei toimi** → Stalwartin webadmin-bundle ei lataudu GitHubista ensimmäisellä käynnistyksellä → oletusasetukset (~3500 kpl) eivät asennu → IMAP/SMTP-autentikointi ei toimi normaalitileille (vain fallback-admin toimii). Frondeo käyttää Dockeria jossa DNS toimii automaattisesti.

2. **IMAP_USER väärässä muodossa** → Stalwart IMAP LOGIN vaatii pelkän käyttäjänimen (`noreply`), EI email-osoitetta (`noreply@grooveserve.com`). Sama koskee SMTP AUTH:ia.

3. **Autoban-kierre** → Epäonnistuneet auth-yritykset kerryttivät banni-laskuria RocksDB:ssä. Bannattu IP `10.89.0.1` (Podman bridge gateway) esti KAIKEN sisäisen liikenteen (IMAP, SMTP, API). Banni persistoi uudelleenkäynnistysten yli.

**Korjaukset:**

- Stalwart käynnistetty `--network host` -flagilla (DNS toimii)
- Stalwart-data nollattu (puhdas alustus webadminilla)
- `IMAP_USER` ja `SMTP_USER` muutettu: `noreply@grooveserve.com` → `noreply`
- Domaini ja tilit luotu uudelleen API:n kautta

**Opit dokumentoitu:** `operations/AGENTS.md` (Podman vs Docker, Stalwart-operointi)

### End-to-end testi — ONNISTUI 2026-04-25

Koko sähköpostiketju testattu onnistuneesti:

1. `jari.mustonen@iki.fi` → `noreply@grooveserve.com` (SMTP via mail.maalla.dev)
2. Stalwart vastaanotti (port 25, SPF pass)
3. grooveserve-email otti vastaan (IMAP IDLE)
4. Vastaus lähetetty (SMTP submission, port 587)
5. Stalwart reititti Mailgunin kautta (`smtp.eu.mailgun.org`, 250 OK)

**Lisälöydökset:**
- Spam-filtteri luokitteli kaiken spämiksi (ei koulutettua mallia) → disabloitu
- Routing strategy vaatii heittomerkit expression-arvoissa: `"'mailgun-relay'"`
- SMTP_HOST pitää olla `mail.grooveserve.com` (ei `127.0.0.1`) koska TLS-sertifikaatin CN täsmätään
- Mailgun relay -konfiguraatio on config.toml:ssa (Settings API ei tue luontia)

### Palvelimen nykytila 2026-04-25

- Stalwart pyörii `--network host` -tilassa, webadmin asennettu
- Domaini `grooveserve.com` + tilit `noreply`, `healthcheck`, `postmaster`
- TLS-sertifikaatti, hostname, Mailgun relay, routing konfiguroitu
- Spam-filtteri disabloitu
- Loglevel: trace (vaihda info:ksi kun valmis)
- grooveserve-email pyörii, IMAP + SMTP toimii
- Roundcube EI vielä käynnissä

### Seuraavat askeleet (päivitetty 2026-04-25)

1. ~~**Luo `assistant@`-tili** Stalwartiin~~ — TEHTY, kaikki 4 tiliä (noreply, healthcheck, assistant, postmaster) olemassa, sama salasana
2. ~~**Muuta arkkitehtuuri** noreply → healthcheck + assistant~~ — TEHTY, email-service refaktoroitu multi-account IMAP-monitoroinniksi
3. ~~**Ansible-roolien päivitys**~~ — TEHTY, `--network host`, SMTP_HOST=mail.grooveserve.com, stalwart config.toml template, multi-account env
4. ~~**Healthcheck-monitori**~~ — siirretty omaan issueen (#13)
5. ~~**Roundcube**~~ — TEHTY, käynnistetty `host.containers.internal`-konfiguraatiolla, HTTPS toimii
6. ~~**Deploy & end-to-end testit**~~ — TEHTY, email-service deployattu, healthcheck@ ja assistant@ testattu end-to-end
7. ~~**Loglevel**~~ — TEHTY, Stalwart trace → info
8. **Ansible-roolien täysi deploy** — `ansible-playbook` ei vielä ajettu koska Stalwart/Roundcube käynnistettiin manuaalisesti. Compose-tiedostot päivitetty, mutta ensimmäinen `podman-compose up -d` vaatii vanhojen manuaalisten konttien siivoamisen.
9. **Spam-käsittely** — oma issue (#12)

### Palvelintiedot

| | Grooveserve | Frondeo (referenssi) |
|---|---|---|
| IP | 204.168.196.71 | 89.167.99.120 |
| SSH | `~/.ssh/grooveserve-hetzner` | `~/.ssh/frondeo-hetzner-root` |
| Stalwart | v0.15.5 | v0.15.5 |
| Domain | grooveserve.com | frondeo.ai |
| Admin UI | SSH tunnel :8080 | SSH tunnel :8080 |

### Testaustiedot

- **Lähettäjä:** jari.mustonen@iki.fi
- **IMAP/SMTP:** mail.maalla.dev (Stalwart), käyttäjä `jari`
- **Salasana:** `infra/secrets/stalwart.yaml` → `stalwart.accounts.jari.password` (homebase SOPS)
- **Ohjeet:** `/Users/jari/Sources/homebase/AGENTS-EMAIL-API.md`
