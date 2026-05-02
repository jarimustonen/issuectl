---
created: 2026-04-28
updated: 2026-04-28
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#33", "#31"]
labels: [ai, conversation, storage]
---

# 37. Cap tool_result content size when persisting conversation history

_Source: agentin keskusteluhistorian tallennus, `services/email/src/db.rs`_
_Continues: #33 (LLM-review löydös C9)_

## Description

`save_conversation_messages_tx` (ja vanha `save_conversation_messages`)
serialisoi koko `Vec<ContentBlock>` JSONB-saraakkeeseen
`conversations.content_json` ilman kokorajoitusta:

```rust
let content_json = serde_json::to_value(&msg.content).unwrap_or_default();
```

`ContentBlock::ToolResult { content: String, .. }` on rajoittamaton
merkkijono. Sisarsarake `content` (TEXT) katkaistaan 100 merkkiin
("`[tool_result]`"-placeholder), mutta `content_json` saa koko sisällön.

### Mihin tämä johtaa

1. **Kannan kasvu**: jos tool palauttaa esim. 30 KB JSON-listan kuluja,
   se tallentuu sellaisenaan jokaiseen iteraatioriviin.
2. **Token-bloat historian uudelleenladauksessa**: seuraava viesti samaan
   threadiin lukee `content_json`:n takaisin ja syöttää LLM:lle. Vanhat
   isot tool_result-sisällöt maksavat tokenit uudestaan joka turn.
3. **Kerrannaisvaikutus**: yhden turnin 5–10 riviä kerää sisällön
   moneen kertaan.

### Miksi nyt RARE

Nykyiset toolit (`save_receipt`, `add_expense`, `update_*`) palauttavat
pieniä `ToolOutput`-rakenteita (alle 1 KB). Extraction-tulokset (`PDF`/
`image OCR`) lisätään käyttäjän viestiin, ei tool_resultina. Mikään tool
ei palauta base64:a tai isoja blobs:ja.

### Miksi ennen agent-cutoveria

Designissa #33 on listattu uusia tooleja (`list_expenses`, `list_receipts`,
`get_draft_summary`), jotka voivat palauttaa pitkiä listoja. Yhdistettynä
`load_conversation_by_thread`:in 200 rivin rajaan ja prompt-cache-strategiaan
(#35), kanta- ja token-budjetit voivat paisua nopeasti.

## Scope

- [ ] Lisää koonsuojaus `save_conversation_messages_tx`:iin: jos
  `ToolResult.content` ylittää inline-rajan (esim. 4 KB), korvataan se
  placeholderilla `"[tool_result elided: <bytes> bytes]"` ennen
  serialisointia.
- [ ] Päätä strategia isoille payloadeille:
  - **A**: pelkkä elide (helppo, häviää tieto historiasta)
  - **B**: erillinen `tool_result_blobs(id, content)` -taulu, viittaus
    `content_json`:ssa (säilyttää ladattavissa, mutta kasvattaa skeemaa)
- [ ] Telemetria: lokita kun elide tapahtuu, koodataan iteraatioiden
  tasolla bytes saved ja keskimääräinen content size.
- [ ] Sama suoja `truncate_pair_aware`-jälkeisessä `load_conversation_*`
  -polussa: vaikka tallennus on jo siivottu, vanhat rivit ennen tätä
  korjausta voivat olla isoja → load-time suoja varmistaa että emme
  syötä paisuneita ToolResulteja LLM:lle.

## Acceptance Criteria

- [ ] Tool_result content yli 4 KB tallentuu placeholderiksi.
- [ ] Yksikkötesti vahvistaa että iso input lyhennetään ennen JSON-
  serialisointia.
- [ ] Loadissa vanha bloated content_json ei pääse LLM-kutsuun
  sellaisenaan (sama elide-logiikka tai erillinen sanitointi).
- [ ] Telemetriasta nähdään elide-tapahtumat (bytes saved, tool_use_id).

## Toteutusvinkki

```rust
const TOOL_RESULT_INLINE_LIMIT: usize = 4096;

fn elide_large_tool_results(blocks: &[ContentBlock]) -> Vec<ContentBlock> {
    blocks.iter().map(|b| match b {
        ContentBlock::ToolResult { tool_use_id, content, is_error }
            if content.len() > TOOL_RESULT_INLINE_LIMIT =>
        {
            tracing::info!(
                tool_use_id, original_bytes = content.len(),
                "tool_result elided in conversation history"
            );
            ContentBlock::ToolResult {
                tool_use_id: tool_use_id.clone(),
                content: format!(
                    "[tool_result elided: {} bytes; original tool_use_id={}]",
                    content.len(), tool_use_id
                ),
                is_error: *is_error,
            }
        }
        other => other.clone(),
    }).collect()
}
```

Käytetään sekä tallennuksessa että ladauksessa.

## Riskit ja huomiot

- Eliderajan valinta: 4 KB on alustava. Token-budjetointi (#31) voi
  vaikuttaa rajaan; kytkentä #31:n kanssa kannattaa tarkistaa toteutuksen
  yhteydessä.
- Vaihtoehto **B** (blob-taulu) on parempi jos historian tarkat sisällöt
  pitää säilyttää debug/audit-tarkoitukseen, mutta lisää skeemamonimutkaisuutta.
  Päätös toteutuksen aikana.
