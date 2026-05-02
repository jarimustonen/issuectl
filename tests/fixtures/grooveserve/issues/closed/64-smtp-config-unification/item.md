---
created: 2026-04-30
updated: 2026-04-30
closed: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#43", "#56"]
labels: [foundation, smtp, config, A-track]
commits:
  - hash: 07dd147
    summary: "refactor(server): unify SmtpConfig with accounts map (#64)"
  - hash: 3d3a525
    summary: "chore(deploy): migrate env templates to per-account SMTP creds (#64)"
  - hash: 125856a
    summary: "docs(server): document unified SmtpConfig + per-address SASL (#64)"
---

# 64. SMTP-konfiguraation yhtenäistäminen — yksi `SmtpConfig` HTTP- ja ingest-puolelle

_Source: `crates/server/src/{http,ingest}/smtp_transport.rs`, `crates/server/src/{main.rs, ingest/config.rs}`_

## Description

`grooveserve-server` lukee SMTP-asetukset **kahdesta erillisestä konfiguraatiopolusta**, jotka eivät puhu keskenään:

- **HTTP-puoli** (`crates/server/src/http/smtp_transport.rs`): `SmtpConfig::from_env()` lukee `SMTP_USER`, `SMTP_PASSWORD`, `SMTP_ASSISTANT_USER`, `SMTP_ASSISTANT_PASSWORD`. Käytetään verifikaatio-, kutsu-, salasanan-resetointi- ja set-password-mailien lähetykseen.
- **Ingest-puoli** (`crates/server/src/ingest/config.rs`): `Config::from_env()` lukee `ACCOUNTS=assistant,healthcheck,…`-listan ja per-account `<NAME>_PASSWORD`-arvot (tai jaettu `ACCOUNT_PASSWORD`). Käytetään agentin vastausten ja healthcheck-vastausten lähetykseen.

Käynnistys-aikainen `verify_smtp`-probe (`crates/server/src/main.rs:117-126`) kokeilee **vain HTTP-puolen** credentit. Se EI varmista että `ASSISTANT_PASSWORD` (jota ingest-puolen `assistant`-tili käyttää oikeasti) toimii.

**Konkreettinen riski:** production voi läpäistä startup-probet ja silti `assistant@`-tilin agentti-vastaukset epäonnistuvat IMAP-loopissa, jos `SMTP_ASSISTANT_PASSWORD` ja `ASSISTANT_PASSWORD` ovat eri arvoja `.env`:ssä / SOPS-secreteissä.

Tämä periytyy pre-A4 `services/api` ja `services/email` -binäärien erillisistä env-tiedostoista. A4 vain yhdisti binäärin paljastaen kahdennuksen.

## Suunnitelma

Aito korjaus vaatii oman suunnitelman:

1. **Päätä SPF/DKIM-aligned senderin politiikka:** lähettääkö agentti vastauksensa per-osoite-credentiaaleilla (kuten nyt: `assistant@grooveserve.com` autentikoituu Stalwartiin `assistant`-tilinä) vai yhteisellä relay-credentiaalilla? Politiikkavaikutukset prod-DKIMiin.
2. **Yhtenäistä SMTP-konfiguraatio:** yksi `SmtpConfig` joka kantaa sekä HTTP-puolen "noreply"-credentiaalit että ingest-puolen account-listan. Tai kaksi struct:ia mutta yhteinen env-namespace ilman duplikaatteja.
3. **Laajenna `verify_smtp` kattamaan kaikki ACCOUNTS-listan tilit** plus HTTP-puolen primary user. Boot epäonnistuu varhaisessa vaiheessa jos jokin tilin creds on rikki.
4. **Päivitä Ansible/secrets:** yksi env-tiedosto (vai kaksi roolia)? Salaisuuksien hallinta SOPS:ssa.

## Aikataulu

- **EI deploy-blokkaaja** — nykykoodi toimii niin kauan kuin `.env`-tiedostoissa `SMTP_ASSISTANT_PASSWORD == ASSISTANT_PASSWORD`.
- **EI A4b:n osa** — A4b:llä on jo valtava scope (Phases 6–15). SMTP-konsolidointi on luonteva pari `ops::ingest`-irrotuksen jälkeen, mutta voidaan tehdä rinnakkain.
- **A4c-worktree** spawn:ataan A4b:n landauksen jälkeen, **rinnakkain** C-aallon (#28, #38, #15) ja D-aallon (#58–#62) kanssa.

## Quick Test

```
# Aseta eri arvoiksi:
export SMTP_ASSISTANT_PASSWORD="api-side-creds"
export ASSISTANT_PASSWORD="ingest-side-creds-different"
# Käynnistä grooveserve-server:
# Boot probe läpäisee (HTTP-puolen credentiaalit ok), mutta agentti-vastaukset failaavat IMAP-loopissa
```

## Notes

Lähde: `/llm-review` round 2 -löydös GPT-5.5:lta, vahvistettu kaikkien neljän reviewer:n cross-review:ssä. Katso `history/review-A4-phases-1-5.md` finding #8.
