# Kuittien OCR — LLM-pohjainen lähestymistapa

## Lähtökohta

Kuitit tulevat sähköpostin liitteinä (kuva/PDF). Lähetetään suoraan multimodaaliselle LLM:lle, joka palauttaa strukturoidun JSON:n. Ei erillistä OCR-pipelinea.

## Mallien vertailu

### Tarkkuus

| Malli | Tekstipohjaiset PDF:t | Skannatut kuitit | Vahvuus |
|-------|----------------------|-------------------|---------|
| GPT-4o | 98% | 91% (+ ulkoinen OCR) | Monikielisyys, yleinen tarkkuus |
| Claude Sonnet 4.6 | 97% | 90% (+ ulkoinen OCR) | Monimutkainen layout, matala hallusinaatioaste (0.09% CC-OCR) |
| Gemini 2.5 Pro | 96% | **94%** (natiivi visio) | Paras skannatuille dokumenteille, ei tarvitse erillistä OCR:ää |
| Gemini 2.5 Flash | ~95% | ~92% (natiivi visio) | Hinta/laatu-suhde |
| Claude Haiku 4.5 | ~95% | ~88% | Halpa, nopea |

Lähde: [Koncile benchmark](https://www.koncile.ai/en/ressources/claude-gpt-or-gemini-which-is-the-best-llm-for-invoice-extraction), [OmniAI benchmark](https://getomni.ai/blog/ocr-benchmark), [Vellum 2026](https://www.vellum.ai/blog/document-data-extraction-llms-vs-ocrs)

**Huomio**: Gemini ei tarvitse erillistä OCR-vaihetta — se käsittelee kuvan natiivisti. Claude ja GPT hyötyvät ulkoisesta OCR:stä skannattujen dokumenttien kanssa, mutta toimivat myös suoraan kuvasta.

### Hinnoittelu (per kuitti)

Kuittikuva ≈ 258–1600 tokenia (riippuu resoluutiosta). Vastaus (JSON) ≈ 500–1000 tokenia.

#### Edulliset mallit (MVP-kandidaatit)

| Malli | Input $/MTok | Output $/MTok | ~Hinta/kuitti | Batch -50% |
|-------|-------------|--------------|---------------|------------|
| **Gemini 2.5 Flash** | $0.30 | $2.50 | **~$0.003** | ~$0.0015 |
| GPT-4.1 nano | $0.10 | $0.40 | ~$0.001 | ~$0.0005 |
| GPT-4.1 mini | $0.40 | $1.60 | ~$0.002 | ~$0.001 |
| **Claude Haiku 4.5** | $1.00 | $5.00 | **~$0.005** | ~$0.003 |
| Gemini 3.1 Flash-Lite | $0.25 | $1.50 | ~$0.002 | - |

#### Frontier-mallit (vertailun vuoksi)

| Malli | Input $/MTok | Output $/MTok | ~Hinta/kuitti | Batch -50% |
|-------|-------------|--------------|---------------|------------|
| Claude Opus 4.6 | $15.00 | $75.00 | ~$0.08 | ~$0.04 |
| Claude Sonnet 4.6 | $3.00 | $15.00 | ~$0.02 | ~$0.01 |
| GPT-5.4 | $2.50 | $10.00 | ~$0.015 | ~$0.008 |
| GPT-4.1 | $2.00 | $8.00 | ~$0.01 | ~$0.005 |
| Gemini 3.1 Pro | $2.00 | $12.00 | ~$0.015 | - |
| Gemini 2.5 Pro | $1.25 | $10.00 | ~$0.01 | ~$0.005 |
| o3 (reasoning) | $2.00 | $8.00 | ~$0.01 | ~$0.005 |
| o4-mini (reasoning) | $1.10 | $4.40 | ~$0.006 | ~$0.003 |

Frontier-mallit ovat 3–20x kalliimpia, mutta absoluuttisesti ero on pieni: kalleimmallakin mallilla 1000 kuittia/kk maksaa ~$80. MVP-volyymilla (100–1000 kuittia/kk) edulliset mallit maksavat alle $5/kk ja frontier-mallitkin alle $20/kk.

**Huomio**: Reasoning-mallit (o3, o4-mini) eivät todennäköisesti tuo lisäarvoa kuitti-OCR:ään — ne on suunniteltu monimutkaiseen päättelyyn, ei tiedon poimintaan.

### Rust-integraatio

| Malli | SDK | Kypsyys |
|-------|-----|---------|
| Claude (Anthropic) | `anthropic-sdk-rust` (epävirallinen, kattava) | Hyvä — aktiivisesti ylläpidetty, TypeScript SDK -pariteetti |
| Gemini (Google) | `google-cloud-rust` (virallinen) tai REST/reqwest | OK — virallinen SDK olemassa mutta nuori |
| GPT (OpenAI) | `async-openai` (epävirallinen) | Hyvä — laajasti käytetty |

Kaikki toimivat REST API:n kautta `reqwest`-kirjastolla, jos SDK ei riitä.

## Suomalaisten kuittien erityispiirteet

LLM:t hallitsevat nämä hyvin:
- **ALV-kannat**: 25.5% (yleinen), 14% (ruoka), 10% (alennettu)
- **EUR-summat**: pilkku desimaalierottimena (12,50 €)
- **Skandit**: ä, ö toimivat kaikilla malleilla
- **Y-tunnus**: 1234567-8 -muoto

LLM:n etu perinteiseen OCR:ään: ymmärtää kontekstin ja pystyy päättelemään kentät ilman erillistä sääntöpohjaista parseria.

## Ehdotettu JSON-skeema

```json
{
  "merchant": "K-Market Kallio",
  "business_id": "1234567-8",
  "date": "2026-04-15",
  "currency": "EUR",
  "total": 45.90,
  "items": [
    { "description": "Kahvi", "quantity": 1, "unit_price": 4.50, "total": 4.50, "vat_rate": 14.0 }
  ],
  "vat_breakdown": [
    { "rate": 14.0, "base": 39.39, "amount": 6.51 }
  ],
  "payment_method": "Visa",
  "raw_text": "koko kuitin teksti tähän"
}
```

## Suositus MVP:lle

### Ensisijainen: Gemini 2.5 Flash

- **Paras hinta/laatu** kuitti-OCR:ään (~$0.003/kuitti)
- **Natiivi vision** — ei tarvita erillistä OCR-vaihetta, paras skannatuille kuiteille
- Ilmainen taso (15 RPM, 1M tokenia/min) riittää kehitykseen
- Google Cloud -tili tarvitaan

### Vaihtoehto: Claude Haiku 4.5

- **Tuttu ekosysteemi** — käytämme jo Claudea muualla
- Hyvä tarkkuus, matala hallusinaatioaste
- Hieman kalliimpi (~$0.005/kuitti) mutta silti halpa
- Parempi monimutkaisissa layouteissa

### Toteutusstrategia

1. **Abstraktio**: Tee trait `ReceiptOcr` joka piilottaa mallin valinnan
2. **Aloita yhdellä**: Gemini Flash tai Claude Haiku — valinta ei ole kriittinen MVP:ssä
3. **Fallback myöhemmin**: Jos yksi malli ei tunnista kuittia, kokeile toista
4. **Prompt**: Yksi selkeä system prompt joka pyytää JSON-vastauksen yllä olevalla skeemalla
5. **Validointi**: Validoi JSON-vastaus Rust-structiin (serde)

### Miksi EI erillistä OCR-pipelinea

- LLM tekee OCR:n ja strukturoinnin yhdessä kutsussa
- Ei tarvita Tesseractia, Google Vision API:a tai muuta erillistä palvelua
- Vähemmän liikkuvia osia = vähemmän ylläpitoa
- LLM ymmärtää suomen kielen ja ALV-säännöt kontekstista
- Hintaero perinteiseen OCR API:iin on minimaalinen MVP-volyymilla

## Avoimet kysymykset

- Pitääkö tukea PDF-kuitteja (ei vain kuvia)? → Kaikki mallit tukevat PDF-inputtia
- Miten käsitellään huonolaatuiset kuvat? → LLM palauttaa confidence-kentän, voidaan pyytää käyttäjältä uusi kuva
- Tarvitaanko kuvan esikäsittelyä (resize)? → Todennäköisesti ei, mutta kuvakoko vaikuttaa tokenimäärään

## Lähteet

- [Koncile: Claude vs GPT vs Gemini laskujen käsittelyssä](https://www.koncile.ai/en/ressources/claude-gpt-or-gemini-which-is-the-best-llm-for-invoice-extraction)
- [Vellum: Document Data Extraction 2026](https://www.vellum.ai/blog/document-data-extraction-llms-vs-ocrs)
- [Parsli: LLM OCR vs Traditional OCR 2026](https://parsli.co/blog/llm-ocr-vs-traditional-ocr)
- [OmniAI OCR Benchmark](https://getomni.ai/blog/ocr-benchmark)
- [CodeSOTA: Claude vs GPT-4o OCR](https://www.codesota.com/ocr/claude-vs-gpt4o-ocr)
- [Gemini API Pricing](https://ai.google.dev/gemini-api/docs/pricing)
- [Claude API Pricing](https://platform.claude.com/docs/en/about-claude/pricing)
