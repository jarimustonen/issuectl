---
created: 2026-04-29
updated: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#42", "#52", "#53"]
labels: [i18n, refactor, tooling]
---

# 54. i18n: siirrä stringit TOML-tauluksi + koodigeneraatio

_Source: #42-monikielisyys review-keskustelu — pitkän aikavälin
ylläpidettävyys nykyistä funktio-per-string-tyyppistä toteutusta paremmin._

## Description

`services/api/src/i18n.rs` on tällä hetkellä ~1300 riviä käsin
kirjoitettuja funktioita muotoa:

```rust
pub fn login_title(l: Locale) -> &'static str {
    match l {
        Locale::En => "Sign in",
        Locale::Fi => "Kirjaudu",
        Locale::Sv => "Logga in",
    }
}
```

Lähestymistapa on grep-ystävällinen ja tekee sopimuksen
(funktio-signatuuri = i18n-string) eksplisiittiseksi, mutta on
ylläpidettävyydeltään raskas:

- Uuden stringin lisääminen vaatii ~5 riviä kirjoittamista
- Distinctness-testin manuaalinen lista on synkronoitava käsin
- Käännöstyö (esim. ulkoiselle kääntäjälle) ei pysty syöttämään dataa
  ilman Rust-tietoa
- Skaalautumisen kipupiste tulee vastaan ~300 stringissä
- Käännösten audit (löytyykö kaikille kolmelle kielelle versio) on
  jaettu kymmeniin match-haaroihin

## Scope

Siirrä string-data koodista yhteen TOML-tiedostoon, generoi koodi
build-scriptillä (build.rs).

```toml
# services/api/i18n/strings.toml

[login_title]
en = "Sign in"
fi = "Kirjaudu"
sv = "Logga in"

[admin_user_counts]
en = "{active} active, {invited} invited, {disabled} disabled."
fi.singular_one_form = "{active} aktiivinen, {invited} kutsuttu, {disabled} deaktivoitu."
fi = "{active} aktiivista, {invited} kutsuttua, {disabled} deaktivoitua."
# … pluralisointia kuvaava skeema
```

Build-script generoi:

- `pub fn login_title(l: Locale) -> &'static str { … }` (sama API kuin nyt)
- Distinctness-testin automaattisesti (jokainen entry tarkistetaan)
- "Käännös puuttuu"-virheet käännösaikana (esim. jos `sv` ei ole määritelty)

## Acceptance criteria

- [ ] Kaikki i18n-stringit `services/api/i18n/strings.toml`-tiedostossa.
- [ ] `build.rs` generoi `i18n_generated.rs`:n joka tarjoaa saman
      julkisen API:n kuin nykyinen `i18n.rs`.
- [ ] Yksikkötesti varmistaa että jokaiselle entry:lle on määritelty
      kaikki tuetut kielet (käännösaikaisesti tai testissä).
- [ ] Distinctness-testi generoidaan automaattisesti TOML:sta.
- [ ] Pluralisointia tukeva skeema (esim. `singular_one_form` per kieli).
- [ ] Käsin kirjoitettu `i18n.rs` poistettu / siirtyy ohueksi
      uudelleenvientikerrokseksi `i18n_generated.rs`:n päälle.

## Out of scope

- TOML-formaattia parempi formaatti (JSON, YAML, csv) — TOML on Rust-
  ekosysteemissä luonteva valinta, ei tarvita erillistä keskustelua.
- Käännösten ulkoistaminen palveluun (Lokalise, Crowdin) — eri issue.
- HTML-fragmenttien turvallisuus (#52) — pysyy oikealla mutta on
  toteutettava ennen tai samanaikaisesti.

## Notes

Kun tämä saadaan, #52 (SafeHtml-tyyppi) voidaan tehdä luonnollisesti
samaan TOML-skeemaan: lisätään `kind = "html"` -merkintä funktioille
jotka palauttavat HTML:ää, jolloin generoitu signature pakottaa
`SafeHtml`-tyypin.
