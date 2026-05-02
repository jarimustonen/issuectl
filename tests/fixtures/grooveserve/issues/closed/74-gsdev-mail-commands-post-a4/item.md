---
created: 2026-04-30
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#33", "#59", "#78", "#95"]
labels: [devex, gsdev]
commits:
  - hash: 4366896
    summary: "feat(dev-cli): add `gs-dev dev parse-eml` CLI (#78 A5c) — send-eml osa"
  - hash: f821889
    summary: "feat(dev-cli): add `gs-dev dev history` + wire `gsdev mail history` (#74 B3)"
  - hash: b1e20bd
    summary: "fix(dev-cli): apply review findings on `dev history` (#74 B3)"
  - hash: 201f7da
    summary: "feat(gsdev): wire `mail send --body` via parse-eml synthesis (#74)"
  - hash: c164694
    summary: "fix(gsdev): harden `mail send --body` per round-2 review (#74)"
  - hash: 6f871e7
    summary: "docs(dev-cli): doc sweep + control-char sanitization on history preview (#74)"
---

# 74. gsdev mail send-eml ja history rikki post-A4 cutover

_Source: B2-local-dev-analysis worktree, 2026-04-30. Lähde: `tools/dev/gsdev/mail.py::cmd_send_eml`/`cmd_history`._

> **Status 2026-05-01:** **Suljettu.** Kaikki kolme post-A4-aukkoa
> palautettu unified-pipelinen päälle:
> - `send-eml` (A5c, #78): `gs-dev dev parse-eml` + `mail.py::cmd_send_eml`.
> - `history` (B3): `gs-dev dev history` + `mail.py::cmd_history`
>   (read-only `conversations`-luenta `(tenant_id, user_id)`-avaimella).
>   `/llm-review` (Gemini, OpenAI, Claude, DeepSeek) + `/assess-findings`:
>   8 FIX:t toteutettu (limit-validointi, stdout-rivit, char-aware
>   truncation, Z-aikaleima, lisäkolumneja human-modeen, sender_local-
>   uudelleennimeäminen, envelope-JSON, doc-korjauksia, PII-varoitus).
>   3 DROP-tuomiota perustellusti (inline-SQL→ops, --tenant-lippu,
>   tests). 1 INCORRECT-finding ("legacy NULL rows invisible"):
>   skema kieltää `tenant_id`/`user_id` NULL:n migraatio 010:ssä,
>   joten löydös ei pidä paikkaansa.
> - `send --body` (B3): `gsdev mail send --body X [-a path]` palautettu
>   pre-A4-UX synteesiamalla `.eml`-tiedoston Pythonissa ja
>   syöttämällä se `parse-eml`-pinnan läpi.
>
> SPIN-OFF:
> - **#95** (`gs-dev` ajaa migraatiot read-only-komennoillekin) —
>   numero alunperin #88, renumeroitu mainin D4-spin-offien (#88-#94)
>   landauksen jälkeen.
> - Pre-existing `gsdev mail send --body` typer-required-bugi
>   ratkesi yhteydessä (cmd_send hyväksyy nyt body:n).

## Description

A4b:n cutoverissa (`#56` Phase 1) `gs-email-cli` korvautui `gs-dev`:llä, mutta `gsdev mail send-eml` ja `gsdev mail history` jäivät ilman vastinetta uudella pinnalla. Wrapperit (`tools/dev/gsdev/mail.py`) printtaavat nyt stderriin ohjeen vaihtoehtoisille reiteille (`gsdev imap up` GreenMail-reitille, `gsadmin email list --from <addr>` DB-luvulle) ja palaavat exit-koodilla 2.

Pre-A4 nämä komennot ajoivat:
- `send-eml`: raaka `.eml`-fixture → `email::parse → spam → handler → agent` -putki ilman IMAP/SMTP-infraa
- `history`: `conversations`-rivit annetulle email-osoitteelle

## Scope

**Vaihtoehto A — toteuta uudet `gs-dev` -subkomennot:**
- `gs-dev dev parse-eml --file FIXTURE.eml` ajaa `.eml`-fixturen `ops::ingest::process_message`-pinnan läpi (vaatii että D-aalto on laajentanut process_messagen sisältämään claim/spam_verdict/handler-vaiheet)
- `gs-dev dev history --user EMAIL` lukee `conversations`-rivit DB:stä read-only
- Päivitä `tools/dev/gsdev/mail.py::cmd_send_eml`/`cmd_history` kutsumaan uutta CLI:tä

**Vaihtoehto B — siivoa stub-wrapperit pois:**
- Poista `cmd_send_eml`/`cmd_history` `mail.py`:stä jos arvioidaan että `gsadmin email list --from <addr>` + `gsdev imap up` ovat riittäviä reittejä
- Päivitä `tools/dev/AGENTS.md` ja `CLAUDE.md` poistamalla viittaukset

## Suositus

**Vaihtoehto A**, mutta **odota D-aaltoa**. D2 (#58) toi `agent_trace`-writerin, D3 (#59) instrumentoi `process_with_tools`. Kun A4b:n "narrow surface" -tulkinta `ops::ingest::process_message`:lle laajenee D-aallon mukana (claim/spam_verdict/handler ops:iin), `gs-dev dev parse-eml` on triviaali toteuttaa.

## Acceptance

- `gsdev mail send-eml --file <fixture>` ajaa parse → handler -putken ilman GreenMail-stack:ia (tai poistuu pinnalta)
- `gsdev mail history --user <email>` listaa keskustelurivit (tai poistuu pinnalta)
- `tools/dev/AGENTS.md` ajan tasalla

## Related

- A4b decision log: "gsdev mail send-eml + history rikki post-cutover, dokumentoitu" (2026-04-30)
- D-aalto (#58–#62): laajentaa `ops::ingest`-pintaa
