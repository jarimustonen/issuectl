---
created: 2026-08-12
updated: 2026-08-14
type: improvement
status: done
priority: normal
commits:
- hash: 363df7b
  summary: 'refactor(config): return schema/transitions load by value'
- hash: 737113a
  summary: 'review: soften re-parse test naming, drop dead-seam comment'
closed: 2026-08-14
---

# Return schema/transitions load by value now that the cache is gone

## Description

# Return schema/transitions `load` by value now that the cache is gone

## Tausta
`collapse-configsource-seam` poisti `ConfigSource`-traitin ja sen ainoan
toteutuksen. `schema::load` ja `transitions::load` palauttavat yhä
`Arc<Schema>` / `Arc<TransitionRules>`. Ainoa syy `Arc`-kääreelle oli
poistetun web-palvelimen `MemoizingConfig`-välimuistin ristisäikeinen
jakaminen. Nyt kutsujat käyttävät arvoa lineaarisesti — `Arc` on turha
varaus ja atomioperaatio.

## Ei suoraa käyttäjävaikutusta — perustelu
`Arc`-varaus on mitätön kustannus ja käyttäytyminen on identtinen. Tämä on
puhdas sisäinen siisteys, sama teema jonka `collapse-configsource-seam`
aloitti (issuen oma valinnainen "load voi palata arvoina" -jatko).

## Miksi tämä vaatii oman suunnittelunsa
Paluutyypin muutos `Arc<T>` → `T` heijastuu `schema::load`/`transitions::load`
-kuluttajiin `doctor.rs`, `agents.rs`, `context.rs`, `init.rs` ja `main.rs`.
`doctor.rs` ja `skill.rs` ovat tällä kierroksella rinnakkaisen worktreen
omistuksessa, joten muutosta EI voitu tehdä `collapse-configsource-seam`:issä.
Tehdään omana mekaanisena, testattavana muutoksena kun doctor/skill-worktree
on maassa. Yhdistä `load_uncached` `load`:iin samalla (nyt yhden kutsujan
kääre) — `_uncached`-nimellä ei ole enää vastaparia.
