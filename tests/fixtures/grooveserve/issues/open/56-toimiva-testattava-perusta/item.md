---
created: 2026-04-29
updated: 2026-05-02
type: epic
owner: jari
status: in-progress
priority: high
related: ["#57"]
labels: [coordination, mvp, architecture]
---

# E56. Toimiva testattava perusta — yhdistetty alusta + kuittiflow

## Goal

Saada Grooveserve sellaiseen tilaan, että kuitti-flow toimii **päästä päähän** käyttäjälle ja että pääsemme **testaamaan ja kehittämään business-logiikkaa** oikealla datalla. Tavoite ei ole optimoitu tuotantojärjestelmä vaan **toimiva perustasinko**: jaettu DB ja koodi sähköposti- ja web-puolen välillä, käyttäjäidentiteetti yhtenäinen, web näyttää käyttäjän tositteet ja agentin tapahtumat, ja kuittien luku liitteistä on tiukkaa. Tämä on alusta, jonka päälle laajennetaan varsinainen matkalaskukäsittely (#5).

**Sisarepic:** [#57](../57-auditoitavuus-asiantuntijanakyma/item.md) — Auditoitavuus + asiantuntijan arviointinäkymä. Toteutetaan rinnakkain mutta koordinoidaan tästä epicistä.

---

## Koordinointi

Tämä epic on koordinaattori. Kun työ jakautuu useaan worktreen kautta tehtävään tehtävään, tämä epic pitää kirjaa jäljellä olevasta työstä ja päätöksistä jotka ohjaavat eteenpäin menemistä. Koordinaattori-agentti spawnataan aina viitteenä `#56` ja sen pitää lukea tämä tiedosto ennen kuin se jakaa työtä.

### Worktree-nimeämiskäytäntö

Worktreet saavat **track-prefiksin** + juoksevan numeron + lyhyen kuvauksen (esim. `A2-ops-crate-skeleton`, `D4-agent-trace-decisions`).

| Prefix | Track | Mihin keskittyy |
|--------|-------|------------------|
| `A` | Foundation (kriittinen polku) | Yhdistetty schema + ops, identiteetti, web-UI käyttäjälle |
| `B` | Dev/test-ympäristö | Lokaali stack, gsadmin, healthcheck |
| `C` | Tositepolun viimeistely | #15, #28, #38, #46, #49 |
| `D` | Sisarepic #57 | Asiantuntijan arviointinäkymä, agent-trace |

Numerointi on per track ja juokseva (A1, A2, A3 ovat A-trackin 1., 2. ja 3. worktree — eivät vaiheet).

### Worktreen spawn-protokolla

Kun spawnaat worktree-agentin (`/worktree`-skill), kerro sille:

1. **Worktree-nimi** track-prefiksillä
2. **Vaihe ja issue** mihin se kuuluu
3. **Velvoite raportoida valmistuessaan tähän epiciin** (`#56`):
   - Päivittää `## Vaiheet ja tila` -checkboxit ja `## Issues`-lista
   - Lisätä Decision logiin **vain ne päätökset jotka ohjaavat tulevaa työtä** (ei toteutuksen yksityiskohtia — ne jäävät commit-viesteihin)

Aiempien worktreiden tulokset löytyvät git-historiasta (`git log --first-parent main`). Tähän tiedostoon ei kerätä per-worktree-yhteenvetoja vaan vain nykytila + ohjaavat päätökset.

---

## Vaiheet ja tila

Vaiheet eivät ole sarjallisia. Neljä raitaa etenee rinnakkain, mutta osa on toistensa blokkeerattuja.

```
Track A (kriittinen polku):  Phase 1 → Phase 2 → Phase 3
Track B (dev-env):           Phase 0  (rinnakkain Track A:n alkua)
Track C (tositteet):         Phase 5  (suurelta osin riippumaton)
Track D (sisarepic #57):     Phase 4  (toteutus odotti Phase 1:tä)
```

### Phase 0 — Testattava ympäristö (Track B) — valmis

- [x] **#33** Local development environment _(B2)_
- [x] **#50** gsadmin local-dev support _(B1)_
- [x] **#74** gsdev mail send-eml/history post-A4 _(B3)_
- [x] **#75** gsdev rebuild policy _(gsdev-rebuild-policy)_
- [x] **#76** `claim_with_thread.rs` fixture-bug _(A5a:n osana)_
- [x] **#77** Hetzner hostnamectl rename _(B2 inline)_
- [x] **#13** Healthcheck-monitori (tuotanto) — lokaali smoke-testi siirretty `#99`:ään

### Phase 1 — Yhdistetty schema + ops-kirjasto (Track A) — valmis

- [x] Cargo workspace `crates/{ops,server,dev-cli}` (entiset `services/{api,email}` poistuneet) _(A2)_
- [x] Yhteinen schema: identiteetti + email-domain + agent-trace samaan DB:hen _(A3, migraatiot 001–018)_
- [x] Yhdistetty binääri `grooveserve-server` (HTTP + IMAP samasta tokio-runtimesta) _(A4a/b)_
- [x] Per-address SASL + `SMTP_<NAME>_USER`/`SMTP_<NAME>_PASSWORD` -namespace _(A4c, #64)_
- [x] `ops::ingest::*` -pinta + pipeline-decoupling + `gs-dev dev parse-eml` _(#78 A5a/b/c)_
- [x] Sulkee **#43**:n vaiheet 2–3 obsoleettina _(option C korvautui jaetulla skeemalla)_

### Phase 2 — Käyttäjäidentiteetti (Track A) — partial

- [x] **#26** Multi-tenant käyttäjähallinta — Phase 1/1.5/2 valmiit
  (rekisteröinti, kutsu, sessio, login, admin-CRUD, last-admin,
  password reset). Phase 3 valmis (a1-multi-tenant-impl-worktree):
  agenttipohjaiset hallinto-toolit + pending-vahvistuspolku +
  policy.md (#67) lukittu. Phase 4 (tilaukset/laskutus/multi-tenant-
  jäsenyys) on edelleen avoinna ja kuuluu MVP-jälkeiseen vaiheeseen.
- [x] **#6** Onboarding-flow + recovery-path _(a7-onboarding-flow)_
- [x] **#22** Käyttäjähallinta — pääkäyttäjän ylläpitonäkymä
  (käyttäjälistaus, kutsut, roolit, deaktivointi); pending-vahvistus
  email-kanavasta lisätty A1-multi-tenant-impl-worktreessa.
- [x] **#58** `gsadmin registrations` rikki nykyisellä skeemalla —
  korjattu (commits da1fdc3, edc3c2e); kytketty #26 Phase 3
  -worktreeseen.
- [x] **#67** Data visibility / access-control policy — locked v1.1
  matriisi `issues/open/67-data-visibility-access-control/policy.md`,
  pointer `crates/ops/AGENTS.md`:ssä. Follow-upit: #99/#100/#101/#102.

### Phase 3 — Web-näkymä käyttäjälle (Track A) — partial

**Estyy odottamaan Phase 2:ta (login).**

- [x] **#11** Tositelistaus + yksittäinen tosite (kuva + jäsennetyt tiedot) + haku/suodatus _(a8-receipt-list-page)_
- [x] **#114** Tapahtumaloki / agenttihistoria käyttäjälle ("mitä agentti teki viestilläsi") _(A3-user-event-log)_
- [x] **#115** Tositteen korjaus webistä _(receipt-edit-restore, 2026-05-02)_. Toteutettu: edit-lomake, revision-historia, restore. Patch<T> tri-state -semantiikka. Spin-offit `/llm-review`-katselmuksesta: **#117** (audit handler HTML-testit), **#118** (audit-trail atomic mutations), **#119** (optimistic concurrency edit-formille).

### Phase 4 — Asiantuntijan arviointinäkymä (Track D, sisarepic #57) — valmis

- [x] **#58–#62** agent-trace writer + process-with-tools-instrumentointi + error/abort-mappaus + manual-run + decisions _(D1–D4, agent-trace-manual-run)_
- [x] **#80** agent_runs cancellation safety (sweeper) _(agent-runs-cancellation-safety)_
- [x] **#82** agent_trace schema cleanup _(actor_user_id, nullable message_id)_
- [x] **#57 Phase 4** — asiantuntijan dashboard ja korjauspinta _(D1-expert-dashboard, 2026-05-01)_. Toteutettu: attention queue, trace-näkymä, mark reviewed, reprocess, revert step, receipt revision browser + restore-to-version. Phase 3 (käyttäjän tapahtumaloki) ja Phase 5 (korjauskanavat) siirretty post-PoC/pilot-vaiheeseen.

### Phase 5 — Tositepolun viimeistely (Track C) — valmis

- [x] **#46** Extract attachments before/independent of spam verdict
- [x] **#49** Agent reply truncates (MaxTokens) — verifioitu
- [x] **#38** Receipt-revision-history _(C2)_
- [x] **#28** Monivaluuttatuki — receipts/expenses/receipt_revisions + ECB-cache _(C3 + expenses-currency-block)_
- [x] **#15** Kuittien OCR — tiukennettu prompt + per-field-confidence + multi-currency-block + fixture-based testit _(C1)_

---

## Issues

**Track A (Foundation, jäljellä):**
- **#26** Multi-tenant käyttäjähallinta — Phase 1/1.5/2/3 valmis,
  Phase 4 (tilaukset/laskutus/hyväksyntäkierto/multi-tenant-jäsenyys)
  jää MVP-jälkeiseen vaiheeseen.
- **#99** ✓ `load_extraction_summaries`-skoopin tiukennus (closed
  2026-05-01) — own-skooppi `(tenant_id, user_id, message_id)`, SQL-filtteri
  + sqlx-testi cross-user-same-message-id -isolaatiolle
- **#100** ✓ `get_user_context`-toolin korjaus A3-skeemalle (fixed 2026-05-01)
- **#101** Admin-audit-log read API + UI (#67 lukitsi pinnan)
- **#103** ✓ `ops::user::*`-tx-aware-refaktorointi (closed
  2026-05-01) — apply_*_inline -duplikaatio poistettu, kaikki
  `confirm_pending`-polut käyttävät kanonisia `_tx`-primitiivejä
- **#104** ✓ `session::resolve` filteröi `tu.status='active' AND
  t.status='active'` (closed 2026-05-01)
- **#105** Pending-admin rate-limit + idempotency (open) —
  DoS-vector prompt-injection-loopissa, vaatii oman ratelimit-
  infran, jää myöhempään
- **#106** ✓ Pending-admin expiry-sweeper (closed 2026-05-01) —
  10 min cron-tikitys folded supervisoriin agent_runs-sweeperin
  rinnalle
- **#114** Tapahtumaloki — ✓ käyttäjän web-näkymä agentin toimille (closed 2026-05-01)
- **#115** ✓ Tositteen korjaus webistä (closed 2026-05-02) — edit/history/restore, Patch<T>, review-spin-offit #117/#118/#119

**Track B (Dev-env, jäljellä):**
- **#99** Healthcheck — lokaali smoke-testi -muunnelma (open, `#13` follow-up)
- **#116** E2E-perustestit lokaalisti (open) — sähköpostiputki + web + Roundcube

**Track C (Tositteet):** _(valmis)_

**Track D / sisarepic #57 (Phase 4 valmis, Phase 3/5 post-PoC):**
- **#57 Phase 4** asiantuntijan dashboard (done 2026-05-01)

**Spin-offit jotka odottavat (PoC-skoopin ulkopuolella):**
- **#85** Inbound SMTP-failure: retry-policy vai IDLE-reclaim (product-decision)
- **#89** Receipts page scaling (object storage / trigram / keyset / currency formatting, post-MVP)
- **#88–#94** D4-review SPIN-OFFit Phase 4 -dashboardia varten (`88-agent-trace-permanent-skip-atomic`, reply_sent-coverage, run_kind-discriminator, covering-index, KnownDecisionType end-to-end, audit_events.trace_id, trace_id-propagointi). Huom: tähän rangeen kuuluva `#88` on **eri issue** kuin suljettu `88-receipt-attachments-junction`; numerointitörmäys juurikaan ei ole pretty mutta korjattu yksi kerrallaan ei kannattaa. Uusien spin-offien numerointi alkaa **#113**:sta.
- **#96** user-PII-column-encryption (post-pilot)
- **#97** agent-write-policy user-data kategorisointi (post-pilot)
- **#98** validation-error-i18n (järjestelmälaajuinen refaktori)
- **#112** pending_replies pre-SMTP Message-Id marker (post-SMTP-pre-finalize crash window — #84 review spin-off)
- **#113** Receipt 1:N attachment junction-taulu (#88-receipt-attachments-junction:in option B), trigger: non-extraction-luontipolku tai monen-liitteen-tarve
- **#117** Audit handler HTML-testit (#115 review spin-off, post-#116)
- **#118** Audit trail atomic mutations (#115 review spin-off, durability hardening)
- **#119** Optimistic concurrency edit-formille (#115 review spin-off, vaatii design-keskustelun)
- **#122** Centralize revision capture (#57 review spin-off, refaktori)

**Pre-#116 worktreet (käynnissä):**
- **#120** Expert queue SQL rewrite — high priority correctness (käynnissä rinnakkain #121:n kanssa)
- **#121** Expert dashboard pagination UI — high priority visibility (käynnissä rinnakkain #120:n kanssa)

**Liittyvät, mutta eivät tämän epicin scopessa:**
- **#43** vaiheet 2–3 — obsoletoituivat Phase 1:n yhteydessä (suljettu)
- **#55** Multi-worker IMAP — ei kriittisellä polulla (CLAUDE.md: suoritusaika ei ole kriittinen MVP:ssä)
- **#34** Compound tools, **#16** Observability, **#27** Session cleanup, **#29**, **#30** -parit — myöhempiä laatuparannuksia

---

## Päätöksiä jotka ohjaavat eteenpäin menemistä

Tähän kerätään **vain ne arkkitehtuuri- ja konventiopäätökset jotka informoivat tulevaa työtä**. Implementaation yksityiskohdat valmistuneista worktreistä ovat git-historiassa, eivät täällä. CLAUDE.md:n "Päätökset ovat ohjaavia, eivät sitovia" -periaate on käytössä.

| Pvm | Päätös | Konteksti / perustelu |
|-----|--------|------------------------|
| 2026-04-29 | **Yksi binääri** (`grooveserve-server`) HTTP + IMAP samasta tokio-runtimesta | Vahvistus #26 §6.1:lle. #55 ratkaistaan task-pool-tasolla, ei prosessijaolla. Systemd `Restart=always` riittää MVP:lle. |
| 2026-04-29 | **Tyhjä DB, ei datamigraatiota** Phase 1:ssä | Ei oikeita asiakkaita, kaikki nollataan. Migraatiot renumeroidaan vapaasti. |
| 2026-04-29 | **Agent-trace erillinen `audit_events`:ista** | Eri tarkoitus: audit_events = manuaalisten muutosten audit, agent_runs = LLM-suoritusten trace. Yksisuuntainen pointer (`audit_events.metadata.agent_run_id`) kun asiantuntija peruuttaa agentin teon. |
| 2026-04-30 | **Tracing-formaatti: JSON/JSONL kaikkialla** | D-aallon agent-trace tarvitsee structured-spanit, AI-luettava on ensisijainen kuluttaja. Lokaali kehittäjä putkittaa `\| jq -r '.fields.message'` tarvittaessa. |
| 2026-04-30 | **Per-address SASL** SMTP-lähettäjäpolitiikka | Stalwart-konfig on jo per-tili (DKIM-avaimet, postilaatikot), DKIM `d=`-aligning toimii ilman lisätyötä, IMAP-puoli vaatii joka tapauksessa per-tili-credentit. Kiinteä 4-tilin joukko, ei dynaamisia per-ticket-osoitteita. Politiikka dokumentoitu `crates/server/AGENTS.md`:ssä; revisio jos joskus tarvitaan dynaamisia lähettäjäosoitteita. |
| 2026-04-30 | `ops`-crate **HTTP-/SMTP-/IMAP-vapaa** puhtaasti DB-domain | `crates/ops/CLAUDE.md` säätää: ei reqwestia, ei axumia, ei lähettävää lettreä. Ulkoiset side-effectit (esim. ECB-fetcher) injektoidaan trait + server-impl -mallilla. |
| 2026-05-01 | **MVP single-tenant -invariantti**: yksi käyttäjä = yksi tenant, multi-tenant on ERROR | Pilotti-vaiheessa jokaisella käyttäjällä on tasan yksi `tenant_users`-rivi. Multi-tenant-toiminnallisuus tulee toista reittiä: kirjanpitäjä-toimet web-liittymästä, vastaukset thread-kontekstista. Jos sender-resolution kohtaa käyttäjän jolla on >1 jäsenyysrivi, palautetaan `OpError::InvalidInput` (`LIMIT 2 + multi-row trip-wire`). `is_known_sender` on global, ei tenant-skooppinen — tenant-skoopin lisääminen on tautologinen kunnes #26/#63 muuttaa shapen. |
| 2026-05-01 | `find_user_by_email` palauttaa rivit **kaikissa tiloissa**, callerit filteröivät | Tarvitaan kahteen paikkaan: web-login-virtaan (`auth::login` haluaa erotella "ei tunneta" vs "disabled") ja sender-resolutioon (`db::resolve_user` haluaa vain `email_verified && membership_status="active" && tenant_status="active"`). Yksi kanoninen lookup + caller-puolen filterointi, ei kahta variaattia. |
| 2026-05-01 | Agent-trace **decision/effect-konventio** | `*Skip`/`*Reject` ovat decision-tapahtumia (kirjoitetaan ennen sivuvaikutusta), `*Sent`/`*Truncated` ovat effect-tapahtumia (kirjoitetaan sivuvaikutuksen jälkeen). `policy_reject` (pre-SMTP) + `policy_reply_sent` (post-SMTP) on canon-pari. SMTP-onnistumisen valehtelu ei salittu. |
| 2026-05-01 | Inline-decision-rivit luovat **uuden `agent_runs`-rivin saman message_id:n alle**, eivät kirjoita olemassa olevaan LLM-runiin | `agent_runs` on immutable finalize:n jälkeen (D2 lifecycle-invariantti). `record_inline_decision_run` käyttää atomic CTE:tä (status='completed', iterations=0). Cohort-split (`iterations >= 1` vs. `= 0`) erottaa LLM-runit decision-runeista Phase 4 -kyselyissä. |
| 2026-05-01 | Inline-decision-riveille **idempotency-key** (migraatio 023) | IMAP-reclaim voi re-enteröidä viestin → estetään duplikaatti `permanent_skip`/`policy_reject`/`reply_sent` -rivit. `(tenant_id, user_id, idempotency_key)` partial unique index, `ON CONFLICT DO NOTHING`. Avain stable-konventiolla: `permanent_skip:attachment:<id>` / `<dt>:msg:<message_id>`. |
| 2026-05-01 | `unknown_sender` ei kirjoiteta `agent_runs`:iin | Schema vaatii NOT NULL tenant_id+user_id, unknown-sender:lla ei ole kumpaakaan. `email_processing.status='unknown_sender'` riittää audittiin — tenant-vapaa transport-tason signal. |
| 2026-05-01 | A7 (#6): MVP-kentät + encryption-päätökset | Onboarding kerää `full_name`/`home_address`/`date_of_birth`/`phone_number`/`employer_name` `user_profiles`-tauluun. **IBAN ulos** (Procountor/Netvisor hoitaa). **Encryption-at-rest** Hetzner LUKS + HTTPS riittää pilotti-vaiheessa; column-encryption #96. **Kaikki agent-muokattavissa**, kategorisointi #97. **Yksi yhdistetty welcome+onboarding-mail**, ei kahta peräkkäistä. |
| 2026-05-01 | #11: vapaa haku **vendor + raw_text**, 50/sivu, attachment-route receiptin alle | Status-suodatus, thumb-sarake, yleinen `/attachments/:id`-pinta ja cursor-pagination jätettiin pois MVP:stä. Receipt ↔ attachment-linkki: alunperin extraction_id same-owner FK **tai** jaettu message_id, **mutta tämä on superseded #88 option A:lla** (strict 1:1 vain `extraction_id` → `extractions.attachment_id`). Stored-XSS-suoja: whitelist `image/jpeg\|png\|webp\|gif` (ei SVG), muut → `application/octet-stream` + `attachment`. `Cache-Control: private, no-store` (ei diskcache-vuotoa logoutin jälkeen). |
| 2026-05-01 | #88 receipt ↔ attachment-linkki **option A**: pelkkä `extraction_id` → `extractions.attachment_id` | DB-verifiointi (`grooveserve_email_main_main`: 0 riviä `extraction_id IS NULL`) ja blast-radius-diff-kysely (0 globaalisti orpoutuvaa liitettä, 6 within-email-cross-link-riviä jotka resolvoituvat oikein per-receipt-skooppiin post-fix) osoittivat että jaettu `message_id`-haara oli pelkkä per-email-grouping ilman valid use casea. PoC sallii edelleen `extraction_id IS NULL` (extraktio voi epäonnistua / MIME ei tueta) — `save_receipt` emittaa `tracing::warn!`-tripwiren tilanteen seurantaan, ei schema-CHECKiä (migraatio 013 admit:taa NULL:n web/manual-syötölle). Junction-taulu seurannassa **#113**:ssa, trigger: ensimmäinen non-extraction-pohjainen luontipolku, tripwire-varoitus tuotannossa, tai monen-liitteen-per-receipt-tarve. |
| 2026-05-01 | **Profile-mutating tools: `with_audited_profile_update` helper** | #100 review paljasti ~160 riviä duplikoitua transaktio+audit-boilerplatea `update_user_notes`- ja `update_user_preferences`-työkalujen välillä. Extractoitu `util.rs`:ään yhdeksi funktioksi, joka hoitaa koko lukitus→snapshot→päivitys→snapshot→revisio→commit -ketjun. Tulevat profile-muokkaustyökalut kutsuvat tätä — eivät kirjoita omaa lukitus/audit-logiikkaa. Closure-pohjainen API: `FnOnce(&mut PgConnection) -> Pin<Box<dyn Future<Output = Result<u64, sqlx::Error>> + Send + '_>>`. Lukitusjärjestys on kapseloitu helperiin: `FOR UPDATE tu` (membership gate) → `INSERT profile` (lazy creation) → `FOR UPDATE p` (serialisoi kirjoitukset). |
| 2026-05-01 | **Durability-konventio**: pre-mutate staging row → ulkoinen efekti → promote-or-sweep | #84:n WAL-pattern (`pending_replies` + sweeper + `promote_*_tx`-atomic) ohjaa tulevia durability-kysymyksiä. Staging-rivi kantaa kaiken recovery-polun tarvitsemaa; sweeper joinaa ulkoisen efektin (esim. SMTP, kolmannen osapuolen API) onnistumis-signaalia vasten. Ei lisätä uutta `pending_*`-taulua per kutsupinta automaattisesti — yhden generic-staging-pinnan suunnittelu tilattu vasta toiseen vastaavaan tarpeeseen. Idempotenssi: insert `ON CONFLICT … WHERE state IN (pending, failed)`, promote `SELECT … FOR UPDATE`, sisäiset INSERTit existence-probella. |

---

## Mitä EI ole scopessa

- Calendar-integraatiot (#16 Google, #17 O365) — vasta matkalasku-vaiheessa
- Netvisor (#18) ja Procountor (#19) -integraatiot — myöhempi vaihe
- Hyväksyntäkierto (#21) ja hyväksymishierarkia (#41) — vasta kun matkalaskut ovat polulla
- Suomen lainsäädännön katselmus (#36) — pohjan jälkeen
- App erillinen origin (#40) — infra-parannus, ei MVP-blokkeri
- Spam-käsittely (#12) — toimii riittävästi tällä hetkellä
- i18n laajennukset (#42, #52, #53, #54) — kosmeettista MVP-tasolle
- Single-binary supervision (#30 -pari) — Phase 1:n yhden binäärin malli ohitti tämän tarpeen

---

## Notes

### Koordinaattorin tehtävät

Kun olet koordinaattori (tämän epicin viite `#56`):

1. **Lue tämä tiedosto kokonaan** ennen kuin jaat työtä
2. **Tarkista Vaiheet ja tila -osio** — mitä on valmista, mitä blokkaa
3. **Tarkista Issues-lista** — mitkä open-issuet odottavat spawnia
4. **Spawn worktreet** track-prefiksien kanssa, kerro velvoite raportoida tähän
5. **Päivitä Decision log** kun isoja päätöksiä syntyy — vain ne jotka ohjaavat tulevaa työtä, ei toteutusyksityiskohtia
6. **Päivitä Vaiheet ja Issues** kun alaissueiden tila muuttuu
7. **Älä unohda sisarepiciä #57** — sen tila pitää myös pitää näkyvissä täällä

### Aikataulu

CLAUDE.md:n mukainen MVP-tavoite: kevät–kesä 2026 (1–3 kk). Tämä epic on **kriittinen polku siihen** — kun tämä on valmis, päästään testaamaan business-logiikkaa oikealla datalla ja laajentamaan varsinaiseen matkalaskukäsittelyyn.
