---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#56"]
labels: [tech-debt, imap, async-runtime, build]
---

# 65. IMAP-asiakkaan migraatio tokio-natiiviksi — pudota async-std + native-tls + vendored OpenSSL

_Source: `crates/server/Cargo.toml`, `crates/server/src/ingest/{imap.rs, imap_transport.rs}`_

## Description

`grooveserve-server`-binääri sisältää **kaksi async-runtimea** ja **kaksi TLS-stackiä** rinnakkain:

```
async-std v1.13.2          ← pulled by async-imap
async-native-tls v0.5.0    ← pulled by async-imap
openssl v0.10.78           ← vendored, kompiloituu lähdekoodista
tokio v1.x + rustls        ← rest of the binary (axum, sqlx, reqwest, lettre)
```

`crates/server/Cargo.toml` joutuu eksplisiittisesti viittaamaan `openssl = { version = "0.10", features = ["vendored"] }`, jotta cross-compile macOS → Linux x86_64 kääntyy. Vendored OpenSSL kasvattaa **cold-compilea ~1 minuutilla** ja kasvattaa runtime-binäärin kokoa.

Tämä periytyy pre-A4 `services/email`-cratesta — A4 vain consolidoi binäärin paljastaen riippuvuusketjun.

## Onko tämä deadlock-riski?

**Ei demonstroitua deadlockia.** `async-std`-tyypit toteuttavat `std::future::Future`:n ja toimivat tokio:n schedulerissa ongelmitta. Gemini-reviewer kutsui tätä "executor-deadlock-riskiksi", mutta muut reviewerit (claude, gpt-5.5, deepseek) pitivät väitettä yliampuvana. Prod ei ole kärsinyt deadlockeja kuukausien aikana.

Aito kustannus on: (a) compile-time, (b) binary size, (c) async stack tracejen luettavuus tokio↔async-std-rajojen yli, (d) **kaksi TLS-stackiä yhdessä binäärissä = isompi attack surface**.

## Suunnitelma

1. **Valitse korvaava IMAP-asiakas:**
   - `imap-codec` + manuaalinen tokio-socket-loop (matalan tason mutta täysi kontrolli)
   - `imap-async-tokio` jos sopiva versio löytyy (tarkista crates.io)
   - Jokin muu tokio-natiivi vaihtoehto
2. **Toteuta IDLE uudestaan tokio-pohjaisesti.** Stalwart-prodia vasten testattuna. IDLE-tilan idle/done -semantiikka on monimutkaista, ei triviaali korvata.
3. **Vaihda `imap_transport.rs` käyttämään `tokio-rustls`:ää** native-tls:n sijaan.
4. **Pudota `Cargo.toml`:sta** `async-imap`, `async-native-tls`, `async-std`, `openssl` (vendored).
5. **Verifioi cross-compile** macOS → Linux x86_64 ilman vendored-OpenSSL:ää.

## Aikataulu

- **Ei deploy-blokkaaja** — nykykoodi toimii.
- **Motivaatiopohjainen prioriteetti:** kun compile-aika tai binary size alkaa näkyä, tai kun TLS-CVE pakottaa OpenSSL-päivityksen joka tuottaa cross-compile-kitkaa.
- Voi tehdä rinnakkain mihin tahansa muuhun työhön — ei hard-blokkaa C/D-aaltoa.

## Notes

Lähde: `/llm-review` Gemini round 1, validoitu cross-review:ssä disputed-itemiksi. Katso `history/review-A4-phases-1-5.md` finding C ja moderator-summary.
