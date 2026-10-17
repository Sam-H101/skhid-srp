# `server-game-data.db`: the game data of tests and tools

The server's game data of 0.26g's `resources.db` with the test patches
`resources.db.patch_14`, `_15`, `_16` and `_17` over it, exactly as `srp-game-data
build` makes it (`crates/resources-db`, `server`): every file under
`gameplay/` the game mounts, the English and Ukrainian localizations, each
project's `client_project` cut to its `server_objects` (the respawn points and
battery containers srp reads), and `srp/provenance`, what it was made of.
6 525 112 bytes, sha256
`358cb6c580cb53cbe2093e438be3b1a20fc62172b76b3e56e5ff6446c8b3b36b`. Tests and
tools read it (`crates/survarium/src/v026g/game_data.rs`); `srp` itself reads
`SRP_GAME_DATA`.

| Archive | Size | sha256 | Gives |
|---|---|---|---|
| `resources.db` (0.26g, build 2777) | 6 285 533 906 | `c74078be…d875492c` | 0.26g's 508 `gameplay/` files, its localizations and 73 projects |
| `resources.db.patch_14` (menus) | 43 691 157 | `7535d0d0…05d4c6c3` | no file srp reads |
| `resources.db.patch_15` (the Bandit balaclava 1810, item level 8) | 46 429 158 | `191b24fb…7b993ea0` | a new `gameplay/db_static_dictionaries`, `gameplay/items/armour/mask/bandit_026.options`, new English and Ukrainian localizations |
| `resources.db.patch_16` (the SVD's damage) | 17 998 | `add87569…c036d093` | a new `gameplay/items/weapons/wpn_svd.options` |
| `resources.db.patch_17` (the H&K 416A5 763, its 5.56x45 841 and EOTech 552 819: the `weapons-hk416` content pack over the three above) | 91 849 868 | `b4576fc1…b0d94e` | a new dictionary, the three items' configs, new English and Ukrainian localizations |

Each patch's URL in it is `http://203.0.113.5/patches/<its file name>` (a
documentation address), so `GET /patches` of a server on this data lists
the four with those URLs.

Made again whenever a test patch changes, on a machine with the archives
(`nix develop` in `srp-026g`):

```sh
cargo run --release -p resources-db --bin srp-game-data -- build \
  --db <0.26g>/game/resources.db \
  --patch <…>/resources.db.patch_14 --patch <…>/resources.db.patch_15 \
  --patch <…>/resources.db.patch_16 --patch <…>/resources.db.patch_17 \
  --patch-url http://203.0.113.5/patches \
  --out resources/game-026g/server-game-data.db
```

The ignored test `game_data::tests::the_test_data_is_0_26g_with_the_test_patches`
makes it again and compares the bytes, and checks that 0.26g's own archive
gives the same tables but for the balaclava (and the SVD's parameters):

```sh
SRP_TEST_RESOURCES_DB=<0.26g>/game/resources.db \
SRP_TEST_PATCHES="<…>/resources.db.patch_14 <…>/resources.db.patch_15 <…>/resources.db.patch_16 <…>/resources.db.patch_17" \
cargo test --release -p survarium -- --ignored the_test_data_is
```
