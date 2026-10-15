# `players.db`: the game data of tests and tools

The server's game data of the release the players run, exactly as
`srp-game-data extract` makes it (`crates/resources-db`, `server`): every file
under `gameplay/` of the players' `resources.db`, each project's
`client_project` (empty: srp only asks whether a project is there), and
`srp/provenance`, which names the players' plan it was made for. 2 856 185
bytes, sha256 `ecc419ff74d30f2b343aed6a3ec703cde7fb6b535f49f4aee27027c4b500337c`.
Tests and tools read it (`crates/survarium/src/v026g/game_data.rs`); `srp`
itself reads `SRP_GAME_DATA`.

It is 0.26g's 508 `gameplay/` files with the release's: the Duty exoskeleton
(1805) and the Bandit balaclava (1810), a new `gameplay/db_static_dictionaries`,
`gameplay/items/armour/back/duty_exo_026.options` and
`gameplay/items/armour/mask/bandit_026.options`; made of the players' ZIP of
sha256 `059e4420ed7bd2d77ae3b08a10b82469565ef8ca2551431923bb755b3c831fac`.

Made again for each release that changes `gameplay/`: it is the
`server-game-data.db` that `make_client_026g.py` writes beside the release's
ZIP, or by hand:

```sh
srp-game-data extract --db <0.26g>/game/resources.db \
  --zip <Survarium-0.26g-srp-files.zip> --blobs <its resources/ blobs> \
  --out resources/game-026g/players.db
```

Two ignored tests check it against the real archive (`game_data::tests`):
`the_server_data_of_the_release_is_the_test_data` (`SRP_TEST_RESOURCES_DB`,
`SRP_TEST_PLAYERS_ZIP`, `SRP_TEST_RESOURCE_BLOBS`: the extraction gives this
file byte for byte) and
`the_installed_archive_of_0_26g_is_the_test_data_without_the_release`
(`SRP_TEST_RESOURCES_DB`).
