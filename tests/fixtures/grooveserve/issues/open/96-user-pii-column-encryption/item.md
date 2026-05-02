---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#6", "#26"]
labels: [security, db, post-pilot]
---

# 88. Käyttäjien PII-tietojen kolumni-encryption

_Source: spin-off #6:n onboarding-toteutuksesta_

## Tausta

#6:n onboarding-virta tallentaa `user_profiles`-tauluun arkaluontoista
identiteettidataa: `home_address`, `date_of_birth`, `phone_number`,
`employer_name`, sekä jatkossa muita kenttiä jotka agentti voi kirjoittaa.

Pilotti-vaiheessa data on tallennettu **plain Postgres-sarakkeisiin**:
- Encryption-at-rest hoidetaan levypohjaisesti (LUKS Hetzneriä).
- Transit-puoli HTTPS:llä (web) ja TLS:llä (DB-yhteydet).

Päätös lykätä kolumni-encryption pilotti-vaiheen yli on kirjattu #56:n
decision-logiin (2026-05-01) — perustelu: pgcrypto / KMS-pinta vaatii
operationaalista avainhallintaa (rotaatio, dump/restore-ergonomia,
sqlx:n custom-decode-pinta) joka ei tuo pilotti-asiakkaille
suhteellista turvaa kun operatiivinen tiimi on yhden hengen kokoinen.

## Scope (post-pilotti)

- [ ] Päätös: **pgcrypto** (`pgp_sym_encrypt`/`pgp_sym_decrypt` master-keyllä)
      vs. **app-tason crypto** (esim. `aes-gcm`-rust-crate, avain
      SOPS:issa) vs. **KMS** (Hetznerin Vault tai vastaava).
      Kirjaa decision-logiin.
- [ ] Avain-konfiguraatio operationaalisesti — SOPS-secret palvelimelle,
      rotaatio-suunnitelma, restore-flow.
- [ ] Migraatio: encrypted-sarakkeet rinnakkain, write to both,
      lue encryptedista, drop plain. Rolling-deploy yhteensopiva.
- [ ] Sqlx-pinta: encrypt/decrypt-helperit `crates/ops/src/crypto.rs`:ssä
      tai vastaavassa; `ops::onboarding`-funktiot kuljettavat plaintext-
      stringit nykyiseen tapaan, alustakerros encryptaa.
- [ ] Käyttöönotto: covered fields = onboarding-PII (osoite, syntymäaika,
      puhelin, työnantajan nimi); muut kentät (notes_md, preferences,
      transport-prefs) jätetään plain:iksi koska ne ovat agent-luettuja
      jokaisessa LLM-kierroksessa ja decrypt-cost olisi raskas.
- [ ] Audit-tableiden (`user_profile_revisions.previous` / `current`)
      JSONB-snapshotin käsittely: encryptataanko myös audit-rivit?

## Pre-vaatimukset

- Ei asiakkaita produktiossa _(MVP-pilotti, post-pilot kun
  asiakaskunta on > 0 ja compliance-vaatimuksia syntyy)_
- Avainhallinta-stack päätetty (SOPS riittää, vai tarvitaanko KMS?)

## Pois scopesta

- Kuittien / liitteiden encryption (eri datamalli, eri lifecycle)
- LLM-prompt-tallenteiden encryption (#57 trace-puoli, eri scope)

## Miksi ei nyt

- Pilotti-asiakkaat hyväksyvät levypohjaisen encryption-at-restin
- Avainhallinta on ops-taakka jonka kustannus näkyy
  vasta restore-tilanteessa (rotation pain, dump/restore-friction)
- Compliance-vaatimukset (GDPR, ISO 27001) tarvitsevat kolumnitason
  cryption vasta kun asiakaskunta vaatii sitä; pilotti-asiakkailla
  yksittäinen NDA + DPA riittää
