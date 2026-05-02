---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#28"]
labels: [tech-debt, agent-tools, testing]
---

# 72. Korvaa ECB-fetcherin OnceCell-singleton ToolRuntime-injektiolla

_Source: C3 (#28) post-implementation /llm-review (kaikki neljä reviewer:ä lippuilivat)_

## Description

C3 lisäsi `crates/server/src/exchange_rates.rs`:ään
`OnceCell<Arc<dyn ExchangeRateFetcher>>`-process-level-singletonin, jonka `main.rs` asettaa booting yhteydessä. `save_receipt`-tool lukee `global_fetcher()`-helperin kautta. Tämä on tunnustettu MVP-shortcut, dokumentoitu `crates/server/AGENTS.md`:n "Exchange rate plumbing" -osiossa.

`/llm-review`-kierros (Gemini, GPT-5.5, Claude, DeepSeek) lippuili tämän anti-patternina:

- **Test-eristys**: `OnceCell::set` panic-aa toistuvasti — testi-prosessissa toinen `set_global_fetcher`-kutsu kaataa ajon. Rinnakkais-testit jotka tarvitsevat eri mock-fetchereitä eivät toimi.
- **Boot-order-coupling**: dev-cli-polut jotka eivät kutsu `set_global_fetcher`:iä saavat foreign-currency-flow degradoidun "exchange_rate is required" -virheeseen ilman selvää syytä.
- **Multi-tenant escape hatch**: jos joskus tarvitaan per-tenant-fetcher-overrides (sandbox-tenant käyttää fixture-rate:ja, prod ECB), singleton estää tämän.
- **Hidden dependency**: `SaveReceipt`-tool-koodi näyttää itsenäiseltä mutta kantaa kätketyn riippuvuuden.

## Suunniteltu ratkaisu

Lisää `Option<Arc<dyn ExchangeRateFetcher>>`-kenttä `ToolRuntime`-structiin (`crates/server/src/tools/context.rs`). Plumbaa läpi:

1. `ToolRuntime { db, exchange_rate_fetcher }` (struct-kenttä)
2. `dispatch::execute(pool, fetcher, ctx, name, input)` (parametri)
3. `process_with_tools(client, pool, fetcher, model, tool_ctx, input, history)` (parametri)
4. Ingest-puolen `runner.rs` × 2 callsite:a (rakennetaan AppState:sta)
5. AppState saa `Arc<dyn ExchangeRateFetcher>` -kentän (bootaan `main.rs`:ssä)
6. `dev-cli`:n `tool save_receipt` -polku — joko No-op-fetcher tai null-init joka palauttaa selvän virheen non-EUR-tapauksessa

Poista `static FETCHER: OnceCell<...>` ja `set_global_fetcher`/`global_fetcher` -helperit. Päivitä `crates/server/AGENTS.md`:n "Exchange rate plumbing" -osio.

Testit: lisää testi-helpper joka rakentaa `ToolRuntime`:n mock-fetcherillä, kattaa `save_receipt`-tool foreign-currency + missing rate -polun (joka tällä hetkellä ei ole testattavissa singletonin takia).

## Miksi ei kuulu #56:n scopeen

- #56 Phase 1 (foundation) on valmis. OnceCell ei estä Phase 2:n (#26 identiteetti), Phase 3:n (web-näkymä) eikä Phase 5:n loppujen (#15 OCR) etenemistä — singleton on lokaali yhdellä tool-polulla.
- Track A/B/C/D-jaolle ei luonteva paikka: tämä on agent-loopin DI-infraa, ei receipts/identity/dev-env -aluetta.
- Filataan itsenäiseksi ilman epiciä; aikataulutetaan kun joku muu refaktoroi `ToolRuntime`:a samalla, tai kun OnceCell:in haitat (testit, monitenant) konkretisoituvat.

## Liittyvät

- `#28` — C3 jossa OnceCell otettiin käyttöön
- `crates/server/src/exchange_rates.rs` — OnceCell-singleton ja set_global_fetcher
- `crates/server/src/tools/context.rs` — ToolRuntime struct
- `crates/server/src/tools/dispatch.rs` — execute-callsite
- `crates/server/src/ingest/agent/mod.rs::process_with_tools` — agent-loop
- `crates/server/AGENTS.md` "Exchange rate plumbing" — nykyinen dokumentaatio
