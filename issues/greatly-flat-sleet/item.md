---
created: 2026-05-09
updated: 2026-05-09
type: chore
reporter: jari
status: open
priority: normal
epic: exorbitantly-ill-apples
related: ['@ridiculously-outgoing-brass']
labels: [release-v0.5.0]
---

# Doctor: rakenteellinen apply-pipeline (Findings + Actions + ApplyOutcome)

## Description

Spin-off from /llm-review round 2 on @ridiculously-outgoing-brass (commit e81daab). Yhdistää useita rakenteellisia löydöksiä joiden korjaus on yksi yhtenäinen refaktori `crates/issuectl-core/src/doctor.rs`:ssä + pieni siivu `parser.rs`:ssä.

## Probleemi

`DoctorReport` on monoliitti johon scan + apply kirjoittavat samaan rakenteeseen. Tämä aiheuttaa ketjun konkreettisia pieniä bugeja:

1. **`apply()` palauttaa `Result<()>` ja `bail!`-keskeyttää.** `--json --fix` kun preflight estää → JSON ei tule ulos, käyttäjä saa stderriin anyhow-tekstiä. Rikkoo AGENTS.md:n "Always `--json` when scripting" -lupauksen.
2. **`fix_applied` -kenttä on epäluotettava.** `apply()`:n early-return reitti `if report.legacy_dirs.is_empty() { report.fix_applied = true; }` asettaa `true`:n vaikka mitään ei kirjoitettu.
3. **Schema bootstrap ei näy `fix_applied`:ssa.** `run()`:n manuaalinen field-splice -lista ei sisällä `schema_written`:iä, joten `--fix` joka pelkästään luo `.schema.yaml`:n raportoi `fix_applied: false`.
4. **Manuaalinen `fresh.x = std::mem::take(&mut report.x)` -lista on hauras.** Jokaisen uuden applied-action-kentän lisääminen vaatii muistin poiston tähän listaan; jos unohtaa, lopullinen JSON kertoo virheellisesti "ei tehty mitään".
5. **`preflight_apply` ja `has_critical_findings` eivät ole linjassa.** `has_critical_findings` listaa: schema-violaatiot, schema-parse-error, invalid-slugs, missing item.md, broken-refs, blocked-by-cycles, status-consistency, timestamp-issues, symlinked-dirs. `preflight_apply` estää vain: flat-layout-konfliktit, duplikaatit, both-open-closed, conflict-markerit, hard-parse-virheet. → osittaisia mutaatioita on mahdollista repolle jonka doctor itse pitää kriittisesti epäterveenä.
6. **`is_hard_parse_error` -luokitus on substring-match.** Jos parser-virheviestin sanamuoto muuttuu lokalisoinnin tai parantelun yhteydessä, virhe luokittuu "soft":ksi → `--fix` jatkaa rikkoutuneen frontmatterin yli.

## Yhtenäinen ratkaisu

Yksi rakenteellinen muutos joka kattaa kaikki yllä olevat:

```rust
// Findings: mitä scan löysi (read-only, populated by scan())
struct DoctorFindings { ... }

// Actions: mitä apply teki (yksi paikka jokaiselle mutaatiolle)
struct DoctorActions {
    legacy_dirs_renamed: Vec<...>,
    flat_layout_migrated: Vec<MigrateMove>,
    notes_renamed: Vec<...>,
    orphan_tempfiles_removed: Vec<...>,
    status_reconciled: Vec<...>,
    schema_written: bool,
    files_rewritten: u32,
}

impl DoctorActions {
    fn is_empty(&self) -> bool { ... }
}

enum ApplyOutcome {
    Ok { findings: DoctorFindings, actions: DoctorActions },
    Blocked { findings: DoctorFindings, blockers: Vec<PreflightBlocker> },
    Failed { findings: DoctorFindings, actions: DoctorActions, error: anyhow::Error },
}

fn apply(repo_root: &Path, lock: &WriteLock) -> ApplyOutcome { ... }
```

`fix_applied = matches!(outcome, ApplyOutcome::Ok { actions, .. } if !actions.is_empty())` — yhden rivin määritelmä korvaa nykyisen ad-hoc bool-laskennan + manuaaliset field-splicet.

`render_json` käsittelee jokaista `ApplyOutcome`-varianttia → JSON tulee ulos myös blocked-tilanteessa, mukana strukturoitu `blockers: [...]`-kenttä.

`preflight_apply`:n estolista yhdenmukaistuu `has_critical_findings`:in kanssa (tai jaetaan eksplisiittisesti per-vaihe; tämä on suunnittelukysymys joka ratkeaa samalla refaktorilla).

`is_hard_parse_error`:n substring-haku korvataan typed `ParseFindingKind`-enumilla `parser.rs`:ssä; doctor luokittelee enum-variantilla, ei tekstillä.

## Definition of done

- `DoctorReport` on jaettu (tai sen sisäinen layout muutettu) `Findings` + `Actions`-rakenteiksi.
- `apply()` palauttaa `ApplyOutcome`-tyyppisen rakenteen, ei `Result<()>` + sivuvaikutuksena täytetty raportti.
- `--json --fix` palauttaa rakenteellisen JSONin sekä onnistuneessa että blokatussa tapauksessa (ei stderr-tekstiä).
- `fix_applied` on yhden ilmaisun määritelmä `actions`-pohjalta; ei manuaalisia field-spliceitä.
- Schema-bootstrap on yksi `Actions::schema_written` -kenttä joka näkyy `fix_applied`:ssa.
- `preflight_apply`:n estolista joko vastaa `has_critical_findings`:iä tai per-vaihe-estolauseet ovat eksplisiittisesti dokumentoituja.
- `is_hard_parse_error` on poistettu; parser palauttaa typed enum -varianttia jonka doctor luokittelee.
- Olemassa olevat 487+ testit menevät läpi; uusia testejä lisätty preflight-blocked + JSON-tulosteen + fix_applied-tarkkuuden + schema-bootstrap-actionin osalta.

## Out of scope

- `migrate_layout`-API:n muutokset (jo tehty round 2:n yhteydessä).
- Schema-vetoiset statukset (@quite-rigid-horses).
- Lib-puolen public API hygieenisyys (R11, jo osa @ridiculously-outgoing-brass:ia).

## Origin

Surfaced by /llm-review round 2 on @ridiculously-outgoing-brass (commit e81daab). Findings OOS-1 (substring parse-error), OOS-2 (DoctorReport monolith), OOS-3 (fix_applied accuracy), OOS-4 (schema bootstrap tracking), OOS-5 (preflight vs has_critical_findings), OOS-7 (--json --fix bail discards JSON). Anthropic raised most; OpenAI confirmed; DeepSeek partially confirmed.

Yhdistetään yhdeksi issueksi koska kaikki kuusi ratkeavat samalla rakenteellisella muutoksella — ei ole järkeä kerrostaa kuutta erillistä PR:ää saman pohjavedon päälle.
