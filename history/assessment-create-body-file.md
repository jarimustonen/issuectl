# Assessment: create body-file review

Source: [`history/review-create-body-file.md`](review-create-body-file.md)

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Structured-body schema completion lacked a positive regression[^f1] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F2 | Agent guidance overstated default epic rendering[^f2] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F3 | Issuectl JSON export-to-import duplicates structured body headings[^f3] | CONFIRMED | OCCASIONAL | NEUTRAL | MODERATE | HIGH | SPIN-OFF |
| F4 | Changelog entry should move from Fixed to Changed[^f4] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F5 | Heading-less structured bodies must warn or error[^f5] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F6 | Boolean body mode permits contradictory core states[^f6] | CONFIRMED | RARE | NEUTRAL | MODERATE | HIGH | DROP (Rule 1b: RARE, no readability gain) |
| F7 | Trailing Markdown whitespace is lost by the new structured mode[^f7] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F8 | Schema stubs lack a separating blank line[^f8] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F9 | Canonical frontmatter splitting is a no-op[^f9] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F10 | Templates drifted and a constructor field was missing[^f10] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F11 | Compatibility paths should adopt structured mode[^f11] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |
| F12 | Structured body files need an H1 prohibition[^f12] | CONFIRMED | RARE | NEUTRAL | MINOR | MED | DROP (Rule 1b: RARE, no readability gain) |
| F13 | Create should reject unclosed Markdown fences[^f13] | CONFIRMED | RARE | NEUTRAL | MINOR | MED | DROP (Rule 1b: RARE, no readability gain) |
| F14 | Required headings should support broader CommonMark forms[^f14] | CONFIRMED | RARE | WORSENS | MODERATE | MED | DROP (Rule 1b: RARE, no readability gain) |
| F15 | Duplicate precheck must score the rendered body[^f15] | UNABLE_TO_VERIFY | — | — | — | MED | DROP (Rule 1a: incorrect or unable to verify) |
| F16 | Body-file integration test needs renaming[^f16] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect or unable to verify) |

**FIX: 2   FIX (with care): 0   SPIN-OFF: 1   DISCUSS: 0   DROP: 13**

## F3 — Issuectl JSON export-to-import duplicates structured body headings

- **Milloin tämä näkyy käyttäjälle** — kun käyttäjä vie issuectl-issuet JSON-muotoon ja tuo saman JSON-aineiston takaisin `issuectl import` -komennolla.
- **Miten se näkyy** — tuotu body käsitellään vapaana kuvauksena, joten jo rakenteisessa bodyssa oleva `## Description` saa eteensä uuden samannimisen otsikon; myös vanha H1 voi jäädä uuden otsikon alle.
- **Miksi sillä on väliä** — työkalun oma JSON-vienti on dokumentoitu importin syötteeksi, mutta uudelleentuonti voi rikkoa bodyn rakenteen jokaisessa tietueessa.
- **Miksi tämä vaatii oman suunnittelunsa** — parserin pitää erottaa issuectl-exportin rakenteinen `body` ulkoisten lähteiden vapaasta `description`-tekstistä rikkomatta GitHub- ja käsin kirjoitettujen importtien nykyisiä semantiikkoja.
- Korjaus tarvitsee erilliset yhteensopivuustestit koko export → parse → create -ketjulle; tämän create-only-korjauksen vaatimus nimenomaisesti säilyttää importin nykyisen käytöksen.

[^f1]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:13`](review-create-body-file.md#L13); `gpt-5.6-sol`: [`history/review-create-body-file.md:13`](review-create-body-file.md#L13); `claude-fable-5`: [`history/review-create-body-file.md:13`](review-create-body-file.md#L13); `deepseek-v4-pro`: [`history/review-create-body-file.md:13`](review-create-body-file.md#L13)
[^f2]: `gpt-5.6-sol`: [`history/review-create-body-file.md:20`](review-create-body-file.md#L20)
[^f3]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:29`](review-create-body-file.md#L29); `gpt-5.6-sol`: [`history/review-create-body-file.md:29`](review-create-body-file.md#L29); `claude-fable-5`: [`history/review-create-body-file.md:29`](review-create-body-file.md#L29); `deepseek-v4-pro`: [`history/review-create-body-file.md:29`](review-create-body-file.md#L29)
[^f4]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:38`](review-create-body-file.md#L38); `gpt-5.6-sol`: [`history/review-create-body-file.md:38`](review-create-body-file.md#L38); `claude-fable-5`: [`history/review-create-body-file.md:38`](review-create-body-file.md#L38); `deepseek-v4-pro`: [`history/review-create-body-file.md:38`](review-create-body-file.md#L38)
[^f5]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:43`](review-create-body-file.md#L43); `gpt-5.6-sol`: [`history/review-create-body-file.md:43`](review-create-body-file.md#L43); `claude-fable-5`: [`history/review-create-body-file.md:43`](review-create-body-file.md#L43); `deepseek-v4-pro`: [`history/review-create-body-file.md:43`](review-create-body-file.md#L43)
[^f6]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:48`](review-create-body-file.md#L48); `gpt-5.6-sol`: [`history/review-create-body-file.md:48`](review-create-body-file.md#L48); `claude-fable-5`: [`history/review-create-body-file.md:48`](review-create-body-file.md#L48); `deepseek-v4-pro`: [`history/review-create-body-file.md:48`](review-create-body-file.md#L48)
[^f7]: `gpt-5.6-sol`: [`history/review-create-body-file.md:55`](review-create-body-file.md#L55)
[^f8]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:57`](review-create-body-file.md#L57)
[^f9]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:58`](review-create-body-file.md#L58)
[^f10]: `gpt-5.6-sol`: [`history/review-create-body-file.md:59`](review-create-body-file.md#L59)
[^f11]: `gemini-3.1-pro-preview`: [`history/review-create-body-file.md:60`](review-create-body-file.md#L60); `gpt-5.6-sol`: [`history/review-create-body-file.md:60`](review-create-body-file.md#L60); `claude-fable-5`: [`history/review-create-body-file.md:60`](review-create-body-file.md#L60); `deepseek-v4-pro`: [`history/review-create-body-file.md:60`](review-create-body-file.md#L60)
[^f12]: `gpt-5.6-sol`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61); `deepseek-v4-pro`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61)
[^f13]: `deepseek-v4-pro`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61)
[^f14]: `gpt-5.6-sol`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61); `deepseek-v4-pro`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61)
[^f15]: `deepseek-v4-pro`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61)
[^f16]: `deepseek-v4-pro`: [`history/review-create-body-file.md:61`](review-create-body-file.md#L61)
