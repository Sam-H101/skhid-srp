# 7zr.exe: the extractor the updater runs

`7zr.exe` here is the standalone console extractor of 7-Zip, version 26.04,
unchanged: the file `https://www.7-zip.org/a/7zr.exe` as fetched over HTTPS on
2026-10-13.

| | |
|---|---|
| Size | 602 624 bytes |
| SHA-256 | `256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282` |
| Author, licence | 7-Zip Copyright (C) 1999-2025 Igor Pavlov. The GNU LGPL (below). Source code: <https://www.7-zip.org/download.html> |

## Licence

From 7-Zip's licence (<https://www.7-zip.org/license.txt>, read 2026-10-13),
which says of every file of 7-Zip but `7z.dll`: the "GNU LGPL".

> This library is free software; you can redistribute it and/or
> modify it under the terms of the GNU Lesser General Public
> License as published by the Free Software Foundation; either
> version 2.1 of the License, or (at your option) any later version.
>
> This library is distributed in the hope that it will be useful,
> but WITHOUT ANY WARRANTY; without even the implied warranty of
> MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
> Lesser General Public License for more details.
>
> You can receive a copy of the GNU Lesser General Public License from
> http://www.gnu.org/

The players' archive says the same in its `README.txt`.

## Why it is in the repository

The updater (`../`) installs the original game for a player who has none: it
downloads the game's `.7z` archive and unpacks five files of it. It does that
with this program and with no other file: the size and SHA-256 above are
compiled into it (`EXTRACTOR` in `../logic/src/game.rs`), and it checks the
file beside it against them before it downloads anything.

7-Zip's own address for the file has no version in it, so the bytes there
change with every 7-Zip release, and an updater that fetched them would stop
installing on that day. So the file is not fetched: `make_client_026g.py`
puts this copy into the players' archive as `game/binaries/x86/7zr.exe`
(`--extractor=<path>` names another copy, which must be the same file), the
tests of the updater use it, and a build of the archive needs no network.

## A newer 7-Zip

Replace the file, and change the size and SHA-256 in three places: the table
above, `EXTRACTOR` in `../logic/src/game.rs`, and `EXTRACTOR_SIZE` /
`EXTRACTOR_SHA256` in `srp-026g/client-patches/make_client_026g.py` (a test
there compares the last two). Then build the updater and the archive again:
players get the new pair together, as one archive. Nothing forces this: the
extractor only ever opens the one archive whose SHA-256 the updater checks
first.
