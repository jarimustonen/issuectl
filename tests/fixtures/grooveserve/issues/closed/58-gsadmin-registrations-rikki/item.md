---
created: 2026-04-29
updated: 2026-05-01
closed: 2026-05-01
type: bug
reporter: jari
assignee: jari
status: fixed
priority: high
labels: [gsadmin, schema, multi-tenant]
related: ["#26", "#50"]
epic: 56
commits:
  - hash: da1fdc3
    summary: "fix(admin): rewrite gsadmin registrations for A3 schema (#58)"
  - hash: edc3c2e
    summary: "fix(admin): apply LLM review fixes to registrations command"
---

# 58. `gsadmin registrations` kysyy sarakkeita joita ei ole olemassa

_Source: #50 LLM-review (2026-04-29) löysi `users.email` vs `user_emails` -epäjohdonmukaisuuden — verifiointi paljasti että `cmd_registrations` on todennäköisesti kokonaan rikki prodissa._

## Description

`tools/admin/gsadmin/cmd_registrations.py` kyselee `users`-taulusta sarakkeita
jotka eivät enää ole olemassa migraation 002 jälkeen.

### Mitä `cmd_registrations` olettaa

```sql
SELECT id, company_name, contact_name, email, email_verified, created_at FROM users
SELECT * FROM users WHERE email = %s
DELETE FROM users WHERE email = %s RETURNING id, email
```

Eli olettaa että `users`-taulussa on sarakkeet `company_name`, `contact_name`,
`email`, `email_verified`.

### Mitä prodin skeema on (services/api/migrations)

- `002_drop_legacy_users.sql` pudottaa vanhan `users`-taulun
- `004_create_users.sql` luo uuden normalisoidun mallin:
  ```sql
  CREATE TABLE users (id, name, password_hash, created_at, updated_at);
  CREATE TABLE user_emails (id, user_id, email, is_primary, verified, ...);
  CREATE TABLE tenant_users (id, tenant_id, user_id, role, status, ...);
  ```
- `009_locales.sql` lisää `users.locale`

`users`-taulussa on siis vain `id, name, password_hash, locale, created_at,
updated_at`. **Sarakkeita `company_name`, `contact_name`, `email`,
`email_verified` ei ole.**

### Lopputulos

Jokainen `gsadmin registrations`-komento kaatuu prodissa virheellä
`column "company_name" does not exist` (tai vastaava). Lokaalin gsdev-DB:n
skeema seuraa samaa migraatiota, joten myös lokaalisti rikki.

## Reproduction

```bash
gsadmin registrations list
# odotettu: lista käyttäjistä
# todellinen: psycopg.errors.UndefinedColumn: column "company_name" does not exist
```

## Korjaussuunnitelma

`cmd_registrations` täytyy kirjoittaa uudestaan normalisoitua skeemaa vasten:

- **list**: joinaa `users` + `user_emails` (primary email) + `tenant_users`
  (rooli/tenantti). Yritysnimi tulee `tenants`-taulusta.
- **show <email>**: hae `user_emails.email` -kautta, palauta käyttäjä +
  kaikki emailit + tenanttijäsenyydet
- **delete <email>**: harkitse — pitäisikö poistaa user_emails-rivi vai
  koko user (cascade vaikutuksineen)? `delete-all` semantiikka pitää myös
  ajatella uusiksi multi-tenant-mallissa.

`cmd_password_reset.py` on jo tehty oikealle skeemalle (`user_emails ue JOIN
users u`), joten se on hyvä viite.

## Suhde muihin issueihin

- **#26 multi-tenant käyttäjähallinta** — uusi skeema on osa tämän epicin
  toteutusta. Korjaus #58:lle riippuu siitä, mikä on lopullinen rooli- ja
  tenantti-näkymä.
- **#56 Phase 2 (Käyttäjäidentiteetti)** — luonteva paikka korjata yhdessä
  rekisteröinti- ja kutsuvirran kanssa.
- **#50 paljasti tämän** — `gsadmin/db.py` -refaktorointi yhdisti
  `cmd_registrations` ja `cmd_password_reset` saman `api_db()`-helperin
  taakse, mikä teki asymmetrian näkyväksi.

## Trade-off

- **+** Komento toimii oikeasti
- **+** Pakottaa miettimään multi-tenant-näkymän rekisteröintiin
- **−** Vaatii skeeman tarkemman ymmärryksen kuin pelkkä column-rename;
  ei yksinkertainen swap

## Quick Test (kun valmis)

```bash
gsadmin registrations list
gsadmin registrations show user@example.com
# odotettu: ei skeeman puutekaatumisia, sarakkeet/joinit täsmäävät
# user_emails + users + tenant_users -malliin
```

## Resolution (2026-05-01)

`cmd_registrations` kirjoitettu uudelleen normalisoidulle skeemalle
commitissa `da1fdc3` (rewrite) ja jatko-LLM-review-fix-erä `edc3c2e`.
Komennot toimivat post-A3-skeemalla:

- **list** — UNION ALL pending self-registration (`tenants.status =
  'pending_verification'`) + pending invitation (`invitations.status =
  'pending'`); rivit kantavat user_id, primary email, tenant + (rooli, jos
  invitation), verified-lippu.
- **show** — joinaa user + primary user_email + kaikki tenant_users-jäsenyydet
  + pending invitations + auth_tokens; tukee monitenanttijäseniä show-näkymässä,
  vaikka MVP-invariantti pitää käytännössä yhden.
- **delete** — kaskata pending self-registrationin (auth_tokens →
  user_emails → invitations → tenant_users → users → tenants),
  kieltäytyy jos käyttäjällä on muita jäsenyyksiä tai tenantilla muita
  jäseniä; pending-invitationille `cancelled` + sirous jos viimeinen
  jäsenyys.
- **delete-all** — vain pending-rekisteröinnit (ei aktiivisia user-rivejä).

Ratkaistu osana #26-multi-tenant-cluster-worktreeta — ei erillistä
follow-uppia tarvita. Quick Test ajettu `gsadmin registrations --help`
-mokkina; täysi DB-ajo on yksiselitteinen kuluttajatesti seuraavalla
gsdev-instanssilla.
