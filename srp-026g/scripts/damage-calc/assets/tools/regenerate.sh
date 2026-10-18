#!/usr/bin/env bash
# Rebuild assets/doll and assets/icons from a pristine Survarium 0.26g resources.db.
# usage: regenerate.sh <resources.db> <config_json_dir> <ffdec-cli.jar> <work_dir> <assets_dir>
#   config_json_dir: gameplay/items/** and localization/english decoded to JSON, named like
#                    gameplay__items__weapons__wpn_ak_74u.options.json (paths '/' -> '__').
# Needs: java, python3 with Pillow, potrace, rsvg-convert, and the repo's dbgrab. On NixOS:
#   nix shell --impure --expr 'with import <nixpkgs>{}; [ jdk potrace librsvg (python3.withPackages(p:[p.pillow])) ]'
set -euo pipefail
DB=$1 CFG=$2 FFDEC=$3 WORK=$4 OUT=$5
HERE=$(cd "$(dirname "$0")" && pwd)
DBGRAB=${DBGRAB:-/home/sheep/Projects/survarium/skhid-srp/tools/re-kit/common/dbgrab/target/release/dbgrab}
mkdir -p "$WORK/swf" "$OUT"
"$DBGRAB" "$DB" get flash_movies/lobby_menu.swf "$WORK/swf"
"$DBGRAB" "$DB" get flash_movies/hud.swf "$WORK/swf"
rm -f "$WORK/swf/"*.orig
# Bitmaps (ffdec names them <characterId>[_<class>].png)
java -Xmx6g -jar "$FFDEC" -export image "$WORK/lobby_images" "$WORK/swf/flash_movies__lobby_menu.swf"
java -Xmx6g -jar "$FFDEC" -export image "$WORK/hud_images" "$WORK/swf/flash_movies__hud.swf"
# Icons: lobby atlases 3273/3276/3277/3282
python3 -I "$HERE/build_icons.py" "$CFG" "$WORK/lobby_images" "$OUT"
# Doll: hud bitmaps 104/107/110/113/117
python3 -I "$HERE/build_doll.py" "$WORK/hud_images" "$OUT/doll" "$(command -v potrace)"
cd "$OUT/doll"
for f in doll_front doll_back doll_zonemap; do rsvg-convert -w 260 $f.svg -o $f.png; done
rsvg-convert -w 252 doll_hud_original.svg -o doll_hud_original.png
