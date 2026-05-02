---
created: 2026-04-29
updated: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
labels: [auth, approval, multi-tenant]
related: ["#21", "#26"]
---

# 41. Hyväksymishierarkia: admin / hyväksyjä / käyttäjä

_Source: matkalaskujen hyväksyntäkierto_

## Description

Nykyinen rooli-malli on liian yksinkertainen. Tällä hetkellä `users`
-taulussa on kaksi roolia (`user`, `admin`) eikä mallia siitä **kuka
hyväksyy kenenkin matkalaskut**. Käytännössä yrityksissä:

- Jokaisella käyttäjällä on **oma hyväksyjä** (esim. esihenkilö).
- Hyväksyjä voi olla itsekin tavallinen käyttäjä jolla on
  hyväksymisoikeus tiettyihin alaisiinsa — ei välttämättä admin.
- Admin (tenantin pääkäyttäjä) hallinnoi käyttäjiä ja
  järjestelmäasetuksia, mutta ei välttämättä ole kenenkään hyväksyjä.
- Joskus admin **on** myös hyväksyjä tai jopa **viimekätinen
  hyväksyjä** kun ketjussa ei ole muuta vaihtoehtoa.

Tämä koskee suoraan issue #21:n hyväksyntäkiertoa: ennen kuin
hyväksyntäpyyntö voi mennä eteenpäin, järjestelmän on tiedettävä kuka
on lähettäjän hyväksyjä.

## Scope (alustava)

- [ ] Tietomalli: `users`-tauluun (tai erilliseen
      `user_approvers`-tauluun) **approver_id**-relaatio. Yksi
      approver per käyttäjä riittää MVP:ssä — myöhemmin voi tukea
      ketjua / vaihtoehtoja.
- [ ] Roolit: `admin`, `approver`, `user`. Päätä:
      - Voiko käyttäjä olla samanaikaisesti useampaa roolia?
      - Riittääkö `is_admin: bool` + `approver_id: Option<i64>`
        booleanina + relaationa, vai tarvitaanko erillinen
        `roles`-taulu?
- [ ] Admin-UI: käyttäjälistalla pitää näkyä per käyttäjä (a) rooli, (b)
      kenen hyväksyjä on. Asetus muutettavissa adminilla.
- [ ] Edge cases:
      - Hyväksyjä deaktivoidaan → kenelle hyväksyttäväksi tulee menee?
        (Fallback admin?)
      - Käyttäjä lähettää itselleen → blokataan.
      - Hyväksyjä ei vastaa N päivässä → muistutus / eskalointi
        adminille.
- [ ] Migraatio nykyisistä rooleista (`user` → `user`, `admin` → `admin`,
      ei `approver` -käyttäjiä alussa).
- [ ] `ops::auth`-tason apurit:
      - `ops::auth::is_admin(user_id)`
      - `ops::auth::approver_for(user_id) -> Option<i64>`
      - `ops::auth::can_approve(actor, target_expense)`

## Out of scope

- Hyväksyntäketjun toteutus (issue #21).
- Eskalointiautomaatio.
- Multiple-approver-rinnakkaiset hyväksynnät.

## Quick Test

- Admin avaa `/admin/users` ja asettaa Paavolle hyväksyjäksi Tiinan.
- DB:ssä `users.approver_id = tiinan_id` Paavon rivillä.
- `ops::auth::approver_for(paavo)` palauttaa Tiinan id:n.
- Tiina deaktivoidaan → Paavon `approver_for` palauttaa fallback-arvon
  (tenantin ensimmäinen aktiivinen admin tai None — päätettävä).
