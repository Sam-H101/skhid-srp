# resources-db

Reads the game's `resources.db` and its patches, lays them over one another
as Survarium 0.26g mounts them, and builds the server's game data from them:
`srp-game-data`.

## Why

The players' game mounts the retail `resources.db` and then every
`resources.db.patch_<n>` beside it (client patch revision 14, README patch 27:
n from 1 to 255, ascending, a missing n skipped; a later archive's file
replaces an earlier one's, new files are added, folders merge, nothing is
taken away). The patches live on a patches server; the players' updater
fetches the ones the game server names.

The server must run on exactly what the players' game mounts: the client
crashes on an item or modification id it lacks, and an item the server does
not know cannot be sold, dropped or worn. And the 6.3 GB archive is not kept
on the server. So `srp-game-data build` takes the same archives, in the same
order, and writes the few files srp reads into one small archive of the
game's own format, with what it was made of. The server offers the players
exactly the patches named there (`Provenance::offered`), so it can only offer
the patches it runs on.

## The crate

| Module | Does |
|---|---|
| `lib.rs` (`Archive`) | Reads an archive: its table when opened (10 MB of 0.26g's 6.3 GB), a file's bytes when asked for. Every offset is checked; node kinds 0.26g has none of are refused by name. From resource-porter's `deps/vostok-resources-db` (`src/fat.rs` at 5d3b53e) |
| `overlay` (`Overlay`) | Archives over one another in order: for each path the file of the last archive that has one. Only the tables are read to lay them; a file is read from its own archive. A path that is a file in one archive and a folder in another is refused |
| `write` | Writes an archive the game mounts, deterministically: the mount root, folders and files in name order, each node's `m_parent` the start of its folder's class (936 for the root's children, as in 0.26g's archive; the engine follows it), each file's CRC-32, a zeroed 256-byte signature after the table, the files' bytes from the next 2048-byte boundary after it. `write` takes the files' bytes in memory (a patch); `write_to` asks for each file's bytes when it writes them, keeps empty folders, and writes files that share a key (a hard link's target and its links, read from another archive) once, so resource-porter can write a whole game archive again with some files changed |
| `server` | The server's game data: `build`, `Provenance`, `read` |
| `pack` | A patch from a folder: every file under it at its path there, written by `write`, read back and compared with the folder |
| `bin/srp-game-data.rs` | The command line |

## The command

```
srp-game-data build --dir <game folder> [--patch-url <base URL>] --out server-game-data.db
srp-game-data build --db resources.db [--patch <file>[=<URL>]]... [--patch-url <base URL>] --out server-game-data.db
srp-game-data info server-game-data.db
srp-game-data check server-game-data.db <the same archives as build>
srp-game-data pack --dir <folder> --out resources.db.patch_<n>
```

- `--dir` takes what the game mounts from that folder, by the exe's own rule:
  `resources.db`, then each `resources.db.patch_<n>` there for n = 1..255
  ascending (decimal as `%u` writes it: `_01`, `_0`, `_256`, `.patch_1.db`
  are not mounted, nor taken). A folder of such a name is not taken either.
- `--db` and `--patch` name the files. A patch must be called
  `resources.db.patch_<n>` (the game mounts no other name), once per n; they
  are laid over `resources.db` in ascending n whatever their order on the
  command line.
- Every patch needs the URL the players' updater downloads it from:
  `--patch <file>=<URL>` (split at the first `=`), or `--patch-url <base>`,
  which gives each patch without one `<base>/<its file name>` (one `/`
  added unless the base ends with one). Plain `http://` and a bare IP are
  fine. A URL must be one the players' updater takes for a patch, or the
  build fails (`server::is_url`, the same rules as the updater's
  `url::patch`, both tested with
  `client/updater/logic/tests/data/patch_urls.txt`): `http://` or
  `https://`, a host name or a bare IPv4 address, an optional port, a path;
  no credentials, no `#`, no spaces, quotes or backslashes; printable ASCII,
  at most 1024 bytes.
- `build` writes `<out>.part` and renames it. It prints what each archive
  gave, the output's size and sha256, and the provenance.
- `info` checks every file of the data against its table and prints the
  provenance; `check` also hashes the archives given and says whether the
  data was made of exactly those, with those URLs (exit status 1 if not).

- `pack` makes a patch of a folder whose files are at the paths the game
  looks them up by (`<folder>/gameplay/items/x.options` is
  `gameplay/items/x.options`), by `write` (above), reads the written file
  back and compares every file with the folder's. A symbolic link, a name
  that is not UTF-8 or has a `\`, an empty folder (`write` makes none) and a
  folder without files are refused; `--out` is never overwritten. The
  release script `release/build-patch` calls it.

For example, on the release machine:

```
cargo run --release -p resources-db --bin srp-game-data -- build \
  --dir ~/srp/0.26g/game --patch-url http://203.0.113.5/patches \
  --out server-game-data.db
```

## Which files

srp reads, of the archive the players mount:

- every file under `gameplay/`: the dictionary (`gameplay/db_static_dictionaries`)
  and each item's config it names (`gameplay/items/...`), the modifications,
  craft and reroll (`gameplay/items/modifications/modifications.options`,
  `.../craft/craft.options`, `.../reroll/reroll.options`), the game modes
  (`gameplay/game_modes/`), the player's config
  (`gameplay/players/default.player`), drops, quests, the skills tree, the
  unique awards, the test profiles, the bundles. The whole folder (4 MB), so
  an item a patch adds comes with its config without a list to keep;
- each project's marker `projects/<project>/client_project`, kept empty:
  srp only asks whether a map's project is there;
- and the data adds `srp/provenance`.

## Provenance

`srp/provenance` in the output, text, fields by one space:

```
skhid server game data 2
db resources.db 6285533906 c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c
patch resources.db.patch_14 43691157 7535d0d0…d4c6c3 http://203.0.113.5/patches/resources.db.patch_14
patch resources.db.patch_15 46429158 191b24fb…b993ea0 http://203.0.113.5/patches/resources.db.patch_15
patch resources.db.patch_16 17998 add87569…c036d093 http://203.0.113.5/patches/resources.db.patch_16
```

`db <file name> <size> <sha256>`, then `patch <file name> <size> <sha256>
<URL>` for each patch in the game's order. `server::read` parses it
(`Provenance`); `Provenance::offered()` gives what the server tells the
players' updater, in order: `("patch_14", sha256, URL)`, ...

The same archives with the same URLs give the same bytes; another URL, or a
patch of other bytes (even one that changes no file srp reads), gives other
provenance and so other bytes.

## Checked

`cargo test -p resources-db`: small archives made by `write` (a later file
over an earlier one, new files, folders merging over three archives, a file
over a folder refused, patches given out of order, gaps in the numbers, the
names the game does not mount, the same bytes twice, other bytes for another
URL or patch, the provenance read back and wrong ones refused, a file unlike
its table refused), resource-porter's `tests/fixture.db` for the reader,
the writer's `m_parent` and signature room, `write_to` (the bytes `write` gives, empty folders, one copy of shared bytes, bytes unlike their size or checksum refused), and `pack` (a folder's files at
their paths, read back, the same bytes twice, what an archive cannot say
refused).

`pack` of the folders patches 14, 15 and 16 were made of gives those
patches byte for byte (sha256 `7535d0d0…d4c6c3`, `191b24fb…b993ea0`,
`add87569…c036d093`).

On the real files (ignored tests):

```
SRP_TEST_RESOURCES_DB=<…/game/resources.db> \
SRP_TEST_PATCHES="<…>/resources.db.patch_14 <…>/resources.db.patch_15 <…>/resources.db.patch_16" \
SRP_TEST_PATCH_URL=http://203.0.113.5/patches \
cargo test --release -p resources-db -- --ignored --nocapture
```

`the_archive_of_0_26g_reads` checks that each of 0.26g's 95 215 nodes
names its folder as `write` does. `the_server_data_of_0_26g_with_its_patches`
builds the data of 0.26g's archive with patches 14 (menus), 15 (the Bandit
balaclava 1810) and 16 (the SVD's damage) and compares every file with the
archives read one over the other again: 2 845 784 bytes, 509 `gameplay/`
files and 73 projects (581 files from `resources.db`, 0 from patch 14, 2 from
15 of which 1 over the stock dictionary, 1 from 16 over the stock SVD), in
under 5 s with the archives cached, and the same bytes twice. With the URLs
above its sha256 is `5a712084…0c8ebc`. With the old `srp/provenance` of the
scratch build in its place (`skhid server game data 1` / `plan none`), the
same files give byte for byte the scratch `server-game-data.db` (sha256
`072cb41a…cf806`) made earlier with the two writer fixes.
