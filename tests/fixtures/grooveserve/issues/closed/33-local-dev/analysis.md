# Local Development Environment — Analysis

_Issue: #33 (`33-local-dev`) · alkuperäinen 2026-04-27 · päivitetty 2026-04-30 (B2-worktree, post-A4)_

> **Tämä päivitys** korvaa pre-A4-version. Aiempi analyysi (yli 900 riviä Hybrid-arkkitehtuurin design-dokumenttia) on sama kuin git-historiassa — Phase 1 -refaktori toteutti suurimman osan siitä A2/A3/A4a/A4b/A4c/B1-worktreessä. Tämä päivitys inventoi mitä on jo tehty ja mitä jää.

---

## TL;DR

`#56` Phase 1 ja Phase 0 (A1–A4c + B1) toteuttivat alkuperäisen `analysis.md`:n ydinkohteet — yhdistetty `grooveserve-server`-binääri, unified DB per-instance, gsdev-CLI, workmux-paneelit, gsadmin lokaalimoodi. **#33:n alkuperäinen scope ("tuota analysis-dokumentti + jatkoissueet") on toteutunut.**

Jäljelle jäävät asiat ovat pieniä jälkikorjauksia, dokumentoitu tämän dokumentin §3:ssa ja eriytetty omiksi alaissueikseen (#74–#77). #33 voidaan sulkea `done`-statuksella tämän worktreen lopuksi.

---

## 1. Inventointi — mitä Phase 0/1 toi

Lähteet: `#56` Worktree-loki (B1, A1, A2, A3, A4a, A4b, A4c), `#56` Decision log, `tools/dev/AGENTS.md`, `operations/dev/`, `crates/server/AGENTS.md`, `tools/admin/AGENTS.md`.

| Alkuperäinen vaatimus | Tila | Toteutuspaikka |
|---|---|---|
| Yksi jaettu Postgres-kontti | ✅ | `operations/dev/compose.dev.yml`, `gsdev init` käynnistää |
| Per-instance Mailpit | ✅ | `gsdev` containers.py, `axllent/mailpit:v1.20.0` |
| Per-instance GreenMail (opt-in) | ✅ | `gsdev imap up` provisioi käyttäjät `IMAP_ACCOUNTS`+`SMTP_<NAME>_PASSWORD`-pareista |
| Per-instance Roundcube (opt-in) | ✅ | `gsdev roundcube up`/`open`/`down` |
| Per-instance unified DB | ✅ A3 | `grooveserve_dev_<safe_id>`. Aiempi kahden DB:n malli (`grooveserve_api_*` + `grooveserve_email_*`) poistui. `instance.dbs`-mappi pitää sekä `"api"` että `"email"` -avaimet samaan databaseen yhteensopivuuden vuoksi. |
| Env-templatet | ✅ A4b | Yksi `operations/dev/env.server.template` korvaa entiset `env.{api,email}.template`. www: `operations/dev/env.www.template`. |
| Per-account SMTP-konfiguraatio | ✅ A4c | `SMTP_ACCOUNTS=noreply,assistant,healthcheck` + `SMTP_<NAME>_USER/_PASSWORD` per-tili. `IMAP_ACCOUNTS` on validoitu osajoukko. Vanha `ACCOUNTS`/`ACCOUNT_PASSWORD` poistettu. |
| `SMTP_TRANSPORT=plain` Mailpit-ajossa | ✅ A4a/A4c | `crates/server/src/http/smtp_transport.rs`. Mailpit-AUTH ohitetaan `SMTP_SKIP_STARTUP_CHECK=1`-flagilla dev-templatessa. |
| `INGEST_ENABLED=0` oletuksena | ✅ A4b | Server on HTTP-only kunnes manuaalinen flippaus `crates/server/.env.local`:iin → cargo-watch restartti |
| `.workmux.yaml` per-worktree-paneelit | ✅ A4b | Yksi paneeli `cargo watch crates/server`:lle, toinen `pnpm dev`:lle, kolmas `<agent>`:lle |
| Repo-paikallinen sccache `.cargo/config.toml` | ✅ A2/A4 | committattu repon juureen |
| Per-service `.envrc` (committable) | ✅ | `crates/server/.envrc`, `sites/www/.envrc` |
| Slug → resurssinimi (törmäysvarma) | ✅ | `tools/dev/gsdev/slug.py::safe_resource_id` (sanitointi + `sha1[:8]`-hash) |
| Atomic registry + filelock | ✅ | `tools/dev/gsdev/registry.py` |
| Pre-buildattu CLI-binääri (ei target-lock-kontentointia) | ✅ | `gsdev init` rakentaa `gs-dev`:n `~/.cache/gsdev/cli/release/gs-dev`:hen. (Ks. §3.2 cache-rebuild-aukko.) |
| `grooveserve-server` -binäärin lokaali ajo | ✅ A4 | Yksi binääri (HTTP + IMAP samasta tokio-runtimesta), workspace-jäsen `crates/server` |
| `gs-dev` dev-subkomennot | ✅ A4a/b | `setup-tenant`, `dev send` (sender-resolution), `dev tool <name> --input <json>`, `dev trace <run_uuid>` |
| gsadmin lokaalimoodi | ✅ B1 (#50) | `GSADMIN_API_DB_URL`/`GSADMIN_EMAIL_DB_URL`/`GSADMIN_LOG_FILE`, `prod_only`-decorator |
| Hetzner host-rename `grooveserve-email` → `grooveserve-server` | ✅ Ansible-puoli | A4b: `host_vars/grooveserve-server.yml`, `roles/server/`, `server.yml`. **Käyttäjän vastuulla:** `hostnamectl`-ajo palvelimella. (Ks. §3.4.) |

---

## 2. Mitä alkuperäinen `#33`-scope vaati ja missä se on nyt

Alkuperäinen item.md (2026-04-27) pyysi:

1. **"Inventories what each service needs to run locally"** — toteutui pre-A4 `analysis.md`:ssa (§1) ja jälleen post-A4 tässä §1:ssä.
2. **"Compares options for local infra (Postgres, mail server, frontend)"** — toteutui pre-A4 `analysis.md` §2:ssa; **päätökset eivät ole muuttuneet** (Hybrid: Postgres jaetaan, sovellukset host-prosesseina, Mailpit per-instance, opt-in GreenMail/Roundcube).
3. **"Recommends a pragmatic approach with concrete next steps"** — toteutui pre-A4 §5–§7:ssa.
4. **"Sketches the files needed (compose, env templates, scripts)"** — toteutui pre-A4 §6:ssa, ja **kaikki tiedostot ovat nyt olemassa** (ks. §1:n taulukko).

Lopputulos: alkuperäinen issue oli **analysis-deliverable** ja se on tuotettu. Toteutus jakautui A-trackin Phase 1 -worktreille, ei #33:lle itselleen.

---

## 3. Aukot — mitä jää post-A4

A4b:n Worktree-loki ja A4c:n Decision log mainitsevat jälkikorjauksia. Listataan ne tähän, prioriteettijärjestyksessä.

### 3.1 `gsdev mail send-eml` ja `gsdev mail history` rikki post-A4

A4b totesi (Worktree-loki):
> `gsdev mail send-eml` ja `gsdev mail history` printtaavat ohjeen `gsadmin email` / `gsdev imap up` -reiteille (gs-email-cli:n vanha pinta ei mappaudu gs-dev:hen).

Konkreettisesti `tools/dev/gsdev/mail.py::cmd_send_eml`/`cmd_history` palaa exit-koodilla 2 ja kirjoittaa stderriin viittauksen vaihtoehtoisiin reitteihin (`gsdev imap up` → GreenMail / `gsadmin email list --from <addr>` → DB).

Tämä on korjattava puutos: pre-A4 `gs-email-cli send-eml` ajoi raakaa `.eml`-fixturea läpi `email::parse → spam → handler → agent` -putkeen, ja `gs-email-cli history` listasi `conversations`-rivit. Molempia tarvitaan kun touchataan `crates/server/src/ingest/`-koodia ilman täyden GreenMail-stack:n pystyttämistä.

**Vaihtoehdot:**
- **A. Rakenna `gs-dev dev parse-eml` ja `gs-dev dev history`** — uudet subkomennot `crates/dev-cli`:hen, `ops::ingest::process_message`-pinta laajenee tukemaan `.eml`-syötettä (claim/spam_verdict siirtyvät A4b:n narrow-surface-ulkopuolelta ops:iin). Linjassa A4b:n havainnon kanssa: D-aalto laajentaa `ops::ingest::process_message`-pintaa kun trace-kirjoitukset menevät paikoilleen.
- **B. Poista wrapperit kokonaan** — Pythonin stub-stderr-viestit eivät ole arvokkaampia kuin niiden poisto, jos tiedämme että vaihtoehtoiset reitit (gsadmin/GreenMail) riittävät MVP:lle.

**Suositus:** vaihtoehto **A**, mutta **odota D-aaltoa**. D2 (#58) toi `agent_trace`-writerin; D3 (#59) instrumentoi `process_with_tools`. Kun siirto on tehty ja `ops::ingest::process_message` on laajempi, `gs-dev dev parse-eml` on triviaali toteuttaa. Wrapperit voivat säilyä exit-2-stubeina kunnes silloin.

→ **Lapsi-issue #74** (luotu).

### 3.2 gsdev pre-build cache rebuild policy

Decision log 2026-04-29 (C1-worktreestä):
> `gsdev mail send` käytti vanhentunutta cachea (`~/.cache/gsdev/cli/release/gs-email-cli`), vaati manuaalisen rm+rebuildin — kannattaa filed:nä jos toistuu.

A4b vaihtoi cache-binäärin nimen `gs-email-cli` → `gs-dev`, mutta perusongelma on sama: **`gsdev` ei rebuildaa cachea automaattisesti kun `crates/dev-cli/src/main.rs` (tai `crates/ops/`-riippuvuus) muuttuu.** `tools/dev/gsdev/mail.py::gs_dev_cli_path()` rakentaa binäärin vain jos tiedosto **puuttuu** — mtime-tarkistusta ei ole.

Käyttötapaus jossa rikkoutuu: kehittäjä muokkaa `crates/dev-cli/src/main.rs`:ää (esim. lisää uuden `dev tool` -aliaksen) ja ajaa `gsdev mail send`. Cache palauttaa vanhan binäärin → uutta logiikkaa ei aja → debugaaminen vie tunnin.

**Vaihtoehdot:**
- **A. mtime-pohjainen rebuild-tarkistus** — tarkista `crates/dev-cli/src/**`, `crates/ops/src/**`, `crates/dev-cli/Cargo.toml` mtime vs. binäärin mtime ennen ajoa.
- **B. `cargo build`-poll joka kerta** — Cargo on jo idempotentti; lämpimällä cachella se ei tee mitään 0,3 s:ssa. Mutta kontentoi mahdollisesti `cargo watch`:n target-kansion kanssa, vaikka `CARGO_TARGET_DIR` on eri (sccache jaa kompilaattorivälimuistia).
- **C. `gsdev doctor` raportoi stale-cachen** — käyttäjä huomaa itse, ei automaatiota.

**Suositus:** vaihtoehto **A** — kevyt mtime-tarkistus, ei riippuvuutta cargo-runaamisesta joka tapauksessa. Hyvä leveling kustomoinnin ja suorituskyvyn kanssa.

→ **Lapsi-issue #75** (luotu).

### 3.3 `crates/server/tests/claim_with_thread.rs` — pre-existing failure

C2:n landing-notes (`#56` Worktree-loki):
> Pre-existing failure: `crates/server/tests/claim_with_thread.rs` 11 testiä punaisina mainissa (tenants.slug NOT NULL ei honoroitu fixturessa).

**Tämä ei ole local-dev-aukko per se** — testit menevät rikki kaikkialla, ei vain lokaalisti. Mutta **lokaali kehittäjä törmää tähän jatkuvasti**: `cargo test --workspace` näyttää 11 punaista riviä jokaisessa worktreessa joka on perustettu C2:n landauksen jälkeen. Tämä piilottaa todelliset regressiot.

→ **Lapsi-issue #76** (luotu). Ei A4b:n synnyttämä, mutta paikallinen kehittäjäkokemus paranee selvästi kun testit ovat vihreitä.

### 3.4 Hetzner-host-rename — `hostnamectl`-ajo

A4b:n Worktree-loki:
> Hetzner-rename: ohjeet annettu Phase 10+11 -committin viestissä; käyttäjä ajaa `hostnamectl`-komennot palvelimella.

Tämä **ei ole local-dev-asia**, vaan tuotanto-ops-tehtävä joka jäi yksittäisestä virstapylväästä. Mutta jätettynä huomaamatta se aiheuttaa Hetznerin paneelin ja palvelimen `hostname`:n drift:n.

→ **Lapsi-issue #77** (luotu). Ei B-trackin scopessa, mutta filed B2:n löydöksenä jotta ei unohdu.

### 3.5 `#13` Healthcheck-monitori — re-evaluointi uutta rakennetta vasten

`#13`:n alkuperäinen scope (2026-04-25, pre-A4) suunnitteli **tuotantopalvelimelle deployatun cron-skriptin** joka pingaa `healthcheck@grooveserve.com`-tiliä 5 min välein. Implementation lukko `operations/infra/ansible/roles/healthcheck-monitor/`:iin.

Phase 1:n päätökset (yksi binääri, unified DB, `INGEST_ENABLED=0` lokaalisti) eivät rikko `#13`:a — Ansible-rooli ja cron-skripti toimivat edelleen tuotannossa. **Mutta:** A4b totesi "Päästä päähän round-trip-testaus jää käyttäjälle" — eli **automaattinen lokaali smoke-test** (joka oli alkuperäisen `#33`:n viimeisellä rivillä mainittu mutta ei #13:n scopessa) on edelleen avoinna.

**Vaihtoehdot:**
- **A. Laajenna #13 lokaaliin smoke-testiin** — sama Ansible-rooli, mutta `gsdev`-pohjainen lokaali variantti (`gsdev healthcheck round-trip`) joka ajaa `roundcube up` + sähköpostin lähetyksen + odottaa vastauksen Mailpitistä.
- **B. Jätä #13 prod-only:ksi, file uusi issue lokaalille smoke-testille** — pidetään skoopit erillään.

**Suositus:** ei B-trackin worktreelle vielä. **#13 on jo `in-progress` Phase 0:ssa** (epic #56 Issues-lista) — kun sitä työnnetään seuraavan kerran, vaihtoehto A on triviaali liitäntä. Ei tarvitse uutta alaissue-numeroa B2:n alle.

### 3.6 Pre-A4 -listalta enää avoinna

Tarkasti pre-A4 `analysis.md`:n §6.8 (Optional polish, defer):
- ⏸ `gsdev seed` — setup-tenant + näytekuitti. Toteutuneen `gs-dev setup-tenant`:n ja `dev tool save_receipt`:n yhdistelmä; voi kirjoittaa skriptin tai bash-aliaksen ilman omaa issuetä. **Ei child-issue**.
- ⏸ `gsdev shell --instance X` — subshell instanssin env:llä. Quality-of-life, ei MVP-blokkeeri. **Ei child-issue**.
- ⏸ Agentti-shorthand `gsdev test "..."` — hyvä idea myöhemmin, mutta nykyinen `gs-dev dev send` + `gsadmin email list` riittää. **Ei child-issue**.

§7 Päätökset / §8 Open questions: kaikki päätökset toteutuneet tai dokumentoidut ratkaisuilla nykyisessä koodissa. Avoimet kysymykset (gsadmin lokaali stub, healthcheck cron, Mattermost webhook, CI-strategia, Cargo workspace, Postgres major version drift, GreenMail self-signed cert, SQLx offline-mode) ovat joko ratkaistu (gsadmin = #50, workspace = A2, GreenMail TLS = `IMAP_TRANSPORT=tls-insecure`) tai eivät ole MVP-blokkereita.

---

## 4. Suositukset — missä järjestyksessä

Kaikki §3:n alaissueet ovat **rinnakkain ajettavissa** keskenään (eri tiedostot). MVP-vaikutuksen mukaan:

1. **#74 (gsdev mail commands post-A4)** — odota D-aaltoa (#58–#62). Kun D-aalto laajentaa `ops::ingest`:ia, `gs-dev dev parse-eml` on triviaali. Ei kiireellinen koska wrapperit antavat exit-2-virheen — ei ole hiljaista failurea.
2. **#75 (gsdev rebuild policy)** — pieni, voi tehdä koska tahansa. Estää tunnin debug-jaksot kun cache vanhenee. **Suositus: aja seuraavalla B-aallolla.**
3. **#76 (claim_with_thread tests)** — pieni mutta toistuva DX-ärsytys. **Suositus: aja heti — vie tunnin, vapauttaa kaikki seuraavat worktreet näkemään `cargo test --workspace`-tuloksen rehellisesti.**
4. **#77 (Hetzner hostname rename)** — ei B-trackin asia, ei MVP-blokkeri. **Suositus: file ja delegoi tuotanto-ops-vuorolle**, ei spawnata B-worktreetä.

---

## 5. Lapsi-issuet (luodut)

| # | Slug | Tyyppi | Lyhyt scope |
|---|------|--------|-------------|
| 71 | `gsdev-mail-commands-post-a4` | task | Toteuta `gs-dev dev parse-eml` ja `gs-dev dev history` (tai siivoa stub-wrapperit pois) D-aallon laajennuksen jälkeen |
| 72 | `gsdev-rebuild-policy` | improvement | mtime-pohjainen rebuild-tarkistus `gsdev mail`-komennoille — estä vanhentuneen cache-binäärin käyttö |
| 73 | `claim-with-thread-tests-fixture` | bug | `crates/server/tests/claim_with_thread.rs` 11 testiä punaisina (`tenants.slug NOT NULL`-fixture-fix) |
| 74 | `hetzner-hostname-rename-verify` | chore | Verifioi/aja `hostnamectl set-hostname grooveserve-server` Hetzner-palvelimella + Hetzner Cloud Console -kohdistus |

Kaikilla `epic: 56` ja `labels: [devex]` (paitsi #77 jolla `[ops]`).

---

## 6. #33:n lopputila

`#33` (`33-local-dev`) suljetaan `done`-statuksella tämän worktreen lopuksi:

- Analysis-dokumentti päivitetty (tämä tiedosto).
- Lapsi-issuet luotu (§5).
- Alkuperäinen scope ("analysis + concrete next steps") suoritettu.

Jatkotyöt:
- #74/#75/#76/#77 spawnataan erillisinä worktreina kun aikataulu sallii (B-aalto tai ad hoc).
- `#13` healthcheck-monitori jatkaa pre-existing-issuena Phase 0:ssa (epic #56:n koordinointi).
- `#56` Phase 0 -checkbox `#33 Local development environment` voidaan rastittaa tämän worktreen päättyessä.
