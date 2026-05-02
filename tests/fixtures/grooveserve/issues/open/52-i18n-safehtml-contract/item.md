---
created: 2026-04-29
updated: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#42"]
labels: [i18n, security, refactor]
---

# 52. i18n: pakota HTML-escape tyyppitasolla (SafeHtml-newtype)

_Source: #42-monikielisyys review (round 2 cross-review, kaikki neljä mallia
flagasivat)_

## Description

`services/api/src/i18n.rs` -funktiot palauttavat tällä hetkellä joko pelkkää
tekstiä tai HTML-fragmentteja samalla `String`/`&'static str`-tyypillä.
Esimerkkejä funktioista jotka palauttavat HTML:ää interpoloimalla:

- `accept_invited_to(locale, tenant)` → `"You have been invited to <strong>{tenant}</strong>."`
- `email_invitation_intro(locale, inviter, tenant)` → sisältää `<strong>`-tagin
- `me_signed_in_to(locale, tenant)` → sisältää `<strong>`-tagin

Sopimus on: kutsuja escappaa interpoloitavat arvot (`tenant`, `inviter`)
**ennen** kutsumista. Tämä toimii nyt koska huolelliset kutsupaikat
(`routes/invite.rs`, `routes/me.rs`, jne.) muistavat käyttää `escape()`a.
Mutta:

- Kääntäjä ei valvo. Tuleva uusi kutsupaikka unohtaa.
- Sama funktiosignatuuri sallii sekä turvallisen että turvattoman käytön.
- `render_action_email`:n `intro: &str` -parametri ottaa joskus
  englanninkielisen lauseen ja joskus pre-escapatun HTML-fragmentin.
  Saman parametrin kaksi eri sopimusta.

#42:ssa otettu käyttöön input-validointi (`ops::validate::name`) joka
hylkää `<>\r\n\0` nimissä — käytännössä XSS-pinta on lähes nolla, mutta
arkkitehtuuri jää hauraaksi.

## Scope

Vaihtoehto A: `SafeHtml(String)` -newtype

```rust
pub struct SafeHtml(String);

impl SafeHtml {
    pub fn from_text(s: &str) -> Self {
        Self(crate::web::escape(s))
    }
    pub fn raw(s: String) -> Self { Self(s) } // used by trusted markup
}

pub fn accept_invited_to(l: Locale, tenant: &SafeHtml) -> SafeHtml { ... }
```

Pakottaa kutsujan rakentamaan `SafeHtml`-arvon ennen kuin se päätyy i18n-funktioon.

Vaihtoehto B: i18n palauttaa pelkkää tekstiä, renderöijä lisää HTML:n

```rust
// i18n
pub fn accept_invited_to_prefix(l: Locale) -> &'static str {
    match l { ... }
}

// route
format!(
    "<p>{} <strong>{}</strong>.</p>",
    i18n::accept_invited_to_prefix(locale),
    escape(tenant_name),
)
```

Yksinkertaisempi, sopii MVP "no template engine" -valintaan paremmin.
Mutta kääntää käännöstaakkaa: jokainen lause joka sisältää interpolaation
pitää jakaa kahdeksi (alku/loppu) tai uudelleenrakenneltava niin että
muuttuja on lopussa.

## Acceptance criteria

- [ ] Kaikki i18n-funktiot jotka aiemmin palauttivat raakaa HTML:ää on
      muunnettu joko (a) pelkän tekstin palauttaviksi, tai (b)
      `SafeHtml`-newtype-tyyppisiksi.
- [ ] `routes/{invite,me,...}.rs`-kutsupaikat eivät enää tee
      `&html_escape(tenant_name)` -ennakkokäsittelyä — eskape on i18n:ssä
      tai renderöijässä.
- [ ] `web::escape`:n duplikaatit on poistettu (jo tehty #42:ssa).
- [ ] Yksikkötesti varmistaa että `SafeHtml`:n raakakonstruktorin
      ulkopuolella ei voi rakentaa escapaamatonta HTML:ää (jos vaihtoehto
      A valitaan).

## Out of scope

- Marketing-sivun (`sites/www`) i18n.
- Email body locale -valinta (jo tehty #42:ssa).
