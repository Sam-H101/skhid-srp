#!/usr/bin/env python3
"""Builds the website's Maps tab (`/maps`) from the game as Skhid's players have it.

    python3 -I build.py --stock <resources.db> --patches <dir> --dbgrab <dbgrab> --ffdec <ffdec-cli.jar>
                        --icons <dir> [--list release/patches.tsv] [--out srp-026g/servers/web-server/maps]

The patches are exactly the rows of release/patches.tsv (each checked by its
sha256), mounted over the stock resources.db in their order, as the game does.
Nothing about a map is written here by hand: the maps are the playable ids of
`maps_dict` (a config of mode 2, 3 or 4 whose project exists, as the server
plays them), and of each map

- the name, the modes and their parameters: `gameplay/game_modes/`;
- respawn points, battery spawners, charging stations ("chargers") and the
  teams' battery containers: the project's `server_objects`;
- artefact containers and what spawns there: the project's `client_objects`
  (`artefact_container`, grouped by their `generic_anomaly_core`);
- the minimap and its frame: `flash_movies/hud.swf`, class
  `survarium.controls.Minimap` (`_minimapProps`, `_levels`), keyed by the
  project's `project_name` as the HUD does;
- the top-down render: srp-026g/scripts/damage-calc/assets/maps/ (maps.json
  and the webp in servers/web-server/damage/maps/), the one thing not read
  from the game. A map without a render fails the build: render it first
  (that folder's README).

Every weather of a map and mode must have the same objects (they do); if a
patch makes them differ the build stops rather than pick one.

Writes <out>/maps.html ({{LANG}} and <!--SITE-HEADER--> for the server),
<out>/maps.js (data and UI), <out>/minimap/<project_name>.webp and
<out>/files.rs (those files, for the server's include) and
<out>/built-from.tsv (the patches it was built from: release/release.sh
refuses a release that offers other patches; a content release builds with
--with <its patch> first). Needs pillow, java on PATH and the
icons the damage calculator's build exports (assets/tools/build_icons.py).
"""
import argparse
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
SRP = HERE.parents[1]
sys.path.insert(0, str(SRP / "scripts" / "gen-data-026g"))
import bincfg  # noqa: E402

MODES = (2, 3, 4)
TYPES = {"js": "text/javascript; charset=utf-8", "webp": "image/webp", "svg": "image/svg+xml"}
# The rule classes (a mode config's `rules`) that use the battery objects
# (docs/notes/modes_maps.md section 4): gather_victory_items (88) the spawners and
# containers; match_time_vi_spawn (89) those and the charging stations. A
# project can hold objects no rule of its mode reads (level_05 has a station
# in modes 2 and 4): those are not shown.
GATHER, TIMED = 88, 89
SITE = "/maps"
RENDERS = SRP / "servers" / "web-server" / "damage" / "maps"
RENDER_TABLE = SRP / "scripts" / "damage-calc" / "assets" / "maps" / "maps.json"


def die(message):
    sys.exit(f"build.py: {message}")


def version(blob):
    return hashlib.sha256(blob).hexdigest()[:12]


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def patch_rows(list_path):
    rows = []
    for line in Path(list_path).read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        name, sha, size, *_ = line.split("\t")
        rows.append((name, sha, int(size)))
    return rows


def extract(args, rows, raw):
    """Every file the tab reads, the stock db first, each patch over it."""
    archives = [Path(args.stock)]
    for name, sha, size in rows:
        path = Path(args.with_patch) if args.with_patch and name == Path(args.with_patch).name else Path(args.patches) / name
        if not path.is_file():
            die(f"{path}: no such file (a row of {args.list})")
        if path.stat().st_size != size or sha256_file(path) != sha:
            die(f"{path}: not the file {args.list} names (size or sha256)")
        archives.append(path)
    wanted = ["client_project", "gameplay/game_modes/", "gameplay/db_static_dictionaries",
              "localization/english/localization", "localization/ukrainian/localization", "flash_movies/hud.swf"]
    for archive in archives:
        for what in wanted:
            subprocess.run([args.dbgrab, str(archive), "get", what, str(raw)], check=True, capture_output=True)


def load(raw, path):
    file = raw / path.replace("/", "__")
    return bincfg.parse(file.read_bytes()) if file.is_file() else None


def hud_minimaps(args, raw, work):
    """{project_name: (frame, bitmap png path)} from hud.swf's Minimap class."""
    swf = raw / "flash_movies__hud.swf"
    for what in ("script", "image"):
        cmd = ["java", "-jar", args.ffdec]
        if what == "script":
            cmd += ["-selectclass", "survarium.controls.Minimap"]
        subprocess.run(cmd + ["-export", what, str(work / what), str(swf)], check=True, capture_output=True)
    source = (work / "script" / "scripts" / "survarium" / "controls" / "Minimap.as").read_text()
    num = r"(-?[\d.]+)"
    props = {m.group(1): {"size": float(m.group(2)), "posX": float(m.group(3)), "posY": float(m.group(4))}
             for m in re.finditer(r"_minimapProps\.(\w+) = \{\s*\"size\":" + num + r",\s*\"posX\":" + num
                                  + r",\s*\"posY\":" + num, source)}
    for alias, of in re.findall(r"_minimapProps\.(\w+) = this\._minimapProps\.(\w+);", source):
        props[alias] = props[of]
    fields = dict(re.findall(r"private var (\w+):BitmapData = new (\w+)\(\);", source))
    levels = {}
    block = re.search(r"_levels(?::Object)? = \{(.*?)\};", source, re.S)
    if not block:
        die("hud.swf: Minimap has no _levels table")
    for key, cls, field in re.findall(r"\"(\w+)\":(?:new (\w+)\(\)|this\.(\w+))", block.group(1)):
        levels[key] = cls or fields[field]
    images = os.listdir(work / "image")
    out = {}
    for key, cls in levels.items():
        found = [n for n in images if re.fullmatch(rf"\d+_{re.escape(cls)}(_.*)?\.png", n)]
        if len(found) != 1 or key not in props:
            continue
        out[key] = (props[key], work / "image" / found[0])
    return out


# The HUD minimap's own icons (Minimap._icons): the class of each, frame 1.
HUD_ICONS = {"player": "PlayerBlue", "base": "Base", "charger": "SpawnIcon", "resupply": "ResupplyIcon"}


def hud_icons(args, raw, work, out):
    """{name: {src, w, h, ax, ay}}: each icon as an SVG file and its anchor."""
    swf = raw / "flash_movies__hud.swf"
    target = work / "sprites"
    subprocess.run(["java", "-jar", args.ffdec, "-format", "sprite:svg", "-selectclass", ",".join(HUD_ICONS.values()),
                    "-export", "sprite", str(target), str(swf)], check=True, capture_output=True)
    frame = lambda cls: next((d / "1.svg" for d in target.iterdir() if d.name.endswith(f"_{cls}_{cls}")), None)
    found = {name: frame(cls) for name, cls in HUD_ICONS.items()}
    if not all(found.values()):
        die("hud.swf: a minimap icon is missing (" + ", ".join(c for n, c in HUD_ICONS.items() if not found[n]) + ")")
    icons = {}
    (out / "icons").mkdir(parents=True, exist_ok=True)
    for name, path in found.items():
        svg = re.sub(r'\s(ffdec:\w+)="[^"]*"', "", path.read_text())
        width = float(re.search(r'<svg[^>]*\swidth="([\d.]+)', svg).group(1))
        height = float(re.search(r'<svg[^>]*\sheight="([\d.]+)', svg).group(1))
        ax, ay = map(float, re.search(r'<g transform="matrix\(1\.0, 0\.0, 0\.0, 1\.0, (-?[\d.]+), (-?[\d.]+)\)"', svg).groups())
        (out / "icons" / f"hud_{name}.svg").write_text(svg)
        icons[name] = {"src": f"{SITE}/icons/hud_{name}.svg?v={version(svg.encode())}",
                       "w": width, "h": height, "ax": ax, "ay": ay}
    return icons


def localization(raw):
    return {lang: load(raw, f"localization/{name}/localization")["strings"]
            for lang, name in (("en", "english"), ("uk", "ukrainian"))}


def artefact_names(texts, key):
    """The names the game's pickup hints give: "Onyx artifact. ..." and "Артефакт Онікс. ..."."""
    hint = f"st_{key}_pickup_hint"
    en = re.match(r"\s*(.+?) artifact\.", texts["en"].get(hint, ""))
    uk = re.match(r"\s*Артефакт (.+?)\.", texts["uk"].get(hint, ""))
    if not (en and uk):
        die(f"{hint}: no artefact name in its text")
    return {"en": en.group(1), "uk": uk.group(1)}


def r1(v):
    return round(v, 1)


def xzy(o):
    x, y, z = o["position"]
    return [r1(x), r1(z), r1(y)]


def heading(o):
    """Which way a respawn point faces, as the SVG rotation of the HUD's
    player triangle (it points down, +y): the engine's yaw is rotation y,
    turned by pi when x and z are both pi; forward is (-sin yaw, cos yaw) in
    x/z. Checked on every map: the points face the enemy's side, at 60
    degrees from its spawns on average."""
    rx, ry, rz = o.get("rotation") or (0, 0, 0)
    yaw = math.pi - ry if abs(abs(rx) - math.pi) < 0.1 and abs(abs(rz) - math.pi) < 0.1 else ry
    vx, vz = -math.sin(yaw), math.cos(yaw)
    return round(math.degrees(math.atan2(-vz, vx)) - 90) % 360


def objects(project, artefact_keys):
    """What the tab shows of one project."""
    so = project.get("server_objects") or {}
    spawners = so.get("victory_item_spawners") or []
    spawner_at = {o.get("full_name"): i for i, o in enumerate(spawners)}
    stations = []
    for o in so.get("victory_item_spawn_points") or []:
        attached = [spawner_at[s["name"]] for s in (o.get("attached_objects") or {}).get("spawners") or []
                    if s["name"] in spawner_at]
        stations.append(xzy(o) + [attached])
    client = project.get("client_objects") or []
    kinds_of = defaultdict(set)
    for o in client:
        if o.get("game_object_type") != "generic_anomaly_core":
            continue
        kinds = {artefact_keys[a["dict_id"]] for a in o.get("artefacts") or [] if a["dict_id"] in artefact_keys}
        for name in o.get("artefact_containers") or []:
            kinds_of[name] |= kinds
    # A container's own position is its anomaly's pivot, shared by all of
    # them; where the artefact is found is its collision geometry (the zone
    # the search runs in).
    by_name = {o.get("full_name"): o for o in client}
    artefacts = []
    for o in client:
        if o.get("game_object_type") == "artefact_container":
            kinds = sorted(kinds_of.get(o.get("full_name"), ()))
            zones = [by_name[c["name"]] for c in o.get("collision_geometries") or [] if c.get("name") in by_name]
            if kinds:
                artefacts.append(xzy(zones[0] if zones else o) + [kinds])
    # A team's container and its stash are where their collision zone is:
    # the object's own position can be a pivot far away (level_04's two
    # containers both stand at one team's side by it).
    def zone_of(o):
        zones = [by_name[c["name"]] for c in o.get("collision_geometries") or [] if c.get("name") in by_name]
        return zones[0] if zones else o
    resupply = [xzy(zone_of(o)) + [o.get("team", -1)] for o in client if o.get("game_object_type") == "resupply_zone"]
    return {
        "respawn": sorted(xzy(o) + [o.get("team", -1), heading(o)] for o in so.get("respawn_points") or []),
        "spawners": [xzy(o) for o in spawners],
        "stations": stations,
        "containers": sorted(xzy(zone_of(o)) + [o.get("team", -1)] for o in so.get("victory_items_containers") or []),
        "resupply": sorted(resupply),
        "artefacts": sorted(artefacts),
    }


def webp(png, out):
    im = Image.open(png).convert("RGBA")
    out.parent.mkdir(parents=True, exist_ok=True)
    im.save(out, "WEBP", quality=92, method=6)
    return im.getbbox(), im.size


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--stock", required=True)
    ap.add_argument("--patches", required=True, help="the folder with the list's resources.db.patch_<n>")
    ap.add_argument("--list", default=str(SRP.parent / "release" / "patches.tsv"))
    ap.add_argument("--with", dest="with_patch", metavar="PATCH",
                    help="the next release's resources.db.patch_<n>, not in the list yet")
    ap.add_argument("--dbgrab", required=True)
    ap.add_argument("--ffdec", required=True, help="ffdec-cli.jar")
    ap.add_argument("--icons", required=True, help="the lobby icons as <config name>.webp")
    ap.add_argument("--out", default=str(SRP / "servers" / "web-server" / "maps"))
    ap.add_argument("--allow-missing-renders", action="store_true", help="a map without a render shows its minimap only")
    ap.add_argument("--work", help="keep the extracted files here and reuse them (while working on the page)")
    args = ap.parse_args()
    rows = patch_rows(args.list)
    if args.with_patch:
        extra = Path(args.with_patch)
        if any(name == extra.name for name, _, _ in rows):
            die(f"--with {extra.name}: in {args.list} already")
        rows.append((extra.name, sha256_file(extra), extra.stat().st_size))
    revision = max((int(n.rsplit("_", 1)[1]) for n, _, _ in rows), default=0)
    out = Path(args.out)

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(args.work or tmp)
        raw = tmp / "raw"
        if not (args.work and raw.is_dir()):
            raw.mkdir(parents=True, exist_ok=True)
            extract(args, rows, raw)
        texts = localization(raw)
        text = lambda key: {lang: texts[lang].get(key, key) for lang in texts}
        dictionary = load(raw, "gameplay/db_static_dictionaries")
        artefact_keys = {i["dict_id"]: i["name"].removeprefix("artefact_") for i in dictionary["items_dict"]
                         if i["cfg_name"].startswith("gameplay/items/artefacts/")}

        # The playable ids, by map (the project's folder) and mode.
        maps = defaultdict(lambda: {"modes": defaultdict(lambda: {"ids": [], "projects": []})})
        for entry in dictionary["maps_dict"]:
            config = load(raw, "gameplay/game_modes/" + entry["cfg_mode_name"])
            if not config or config["mode"].get("mode") not in MODES:
                continue
            mode = config["mode"]
            project_path = mode["match_making_params"]["map_name"]
            if not (raw / f"projects__{project_path.replace('/', '__')}__client_project").is_file():
                continue
            folder = project_path.split("/")[0]
            m = maps[folder]
            m["map_name"] = mode["ui"]["map_name"]
            entry_mode = m["modes"][mode["mode"]]
            entry_mode["ids"].append(entry["id"])
            entry_mode["projects"].append(project_path)
            entry_mode["mode_name"] = mode["ui"]["mode_name"]
            entry_mode["parameters"] = mode.get("parameters") or {}
            entry_mode["rules"] = set(mode.get("rules") or [])
        if not maps:
            die("no playable map in the data")

        minimaps = hud_minimaps(args, raw, tmp)
        for sub in ("minimap", "icons"):
            shutil.rmtree(out / sub, ignore_errors=True)
        hud = hud_icons(args, raw, tmp, out)
        renders = json.loads(RENDER_TABLE.read_text())["maps"]
        out.mkdir(parents=True, exist_ok=True)
        data_maps, missing = [], []
        used_artefacts = set()
        for folder in sorted(maps, key=lambda f: (f.startswith("v"), f)):
            m = maps[folder]
            modes, project_name, artefacts = [], None, None
            for mode_id in sorted(m["modes"]):
                mm = m["modes"][mode_id]
                seen = None
                for path in mm["projects"]:
                    project = load(raw, f"projects/{path}/client_project")
                    got = objects(project, artefact_keys)
                    if seen is not None and got != seen:
                        die(f"{path}: its objects differ from {mm['projects'][0]}'s (same map and mode)")
                    seen = got
                    project_name = project.get("project_name") or folder
                if artefacts is not None and seen["artefacts"] != artefacts:
                    die(f"{folder}: artefact containers differ between modes")
                artefacts = seen.pop("artefacts")
                rules = mm["rules"]
                if not rules & {GATHER, TIMED}:
                    seen["spawners"], seen["containers"] = [], []
                if TIMED not in rules:
                    seen["stations"] = []
                p = mm["parameters"]
                modes.append({"mode": mode_id, "ids": sorted(mm["ids"]), "name": text(mm["mode_name"]),
                              "stations_active": p.get("spawn_points_count"), "batteries": p.get("victory_items_count"),
                              **seen})
            for a in artefacts:
                used_artefacts.update(a[3])
            if project_name not in minimaps:
                die(f"{folder}: hud.swf has no minimap for project_name {project_name}")
            frame, bitmap = minimaps[project_name]
            name = f"{project_name}.webp"
            bbox, size = webp(bitmap, out / "minimap" / name)
            blob = (out / "minimap" / name).read_bytes()
            s, px, py = frame["size"], frame["posX"], frame["posY"]
            k = 2 * s / size[0]
            # World rectangles as [x0, -z1, x1, -z0]: the SVG's x right, y down = -z.
            box = [r1(px - s), r1(-py - s), r1(px + s), r1(-py + s)]
            play = [r1(px - s + bbox[0] * k), r1(-py - s + bbox[1] * k), r1(px - s + bbox[2] * k), r1(-py - s + bbox[3] * k)]
            entry = {"key": folder, "name": text(m["map_name"]), "box": box, "play": play,
                     "minimap": f"{SITE}/minimap/{name}?v={version(blob)}", "modes": modes, "artefacts": artefacts}
            render = renders.get(folder)
            if render and (RENDERS / render["image"]).is_file():
                a, b, c, d, e, f = render["pixel_to_world_affine"]
                w, h = render["image_size_px"]
                blob = (RENDERS / render["image"]).read_bytes()
                entry["render"] = {"src": f"/damage/maps/{render['image']}?v={version(blob)}",
                                   "box": [r1(c), r1(-f), r1(a * w + c), r1(-(e * h + f))]}
                # The render's play area (the native minimap's, per map: that
                # folder's play_area.py) where it has one: some minimaps are
                # opaque to their edge.
                if render.get("play_area_px"):
                    x0, y0, x1, y1 = render["play_area_px"]
                    entry["play"] = [r1(a * x0 + c), r1(-(e * y0 + f)), r1(a * x1 + c), r1(-(e * y1 + f))]
            else:
                missing.append(folder)
            data_maps.append(entry)
        if missing and not args.allow_missing_renders:
            die("no top-down render of " + ", ".join(missing)
                + " (srp-026g/scripts/damage-calc/assets/maps/README.md; then the site's copy in servers/web-server/damage/maps/)")

        icons = {}
        for key in sorted(used_artefacts):
            icon = Path(args.icons) / f"artefact_{key}.webp"
            if not icon.is_file():
                die(f"{icon}: no such icon")
            icons[key] = {"name": artefact_names(texts, key)}
            # The lobby icon cut to the artefact, on a square.
            im = Image.open(icon).convert("RGBA")
            im = im.crop(im.getbbox())
            side = max(im.size)
            square = Image.new("RGBA", (side, side))
            square.paste(im, ((side - im.width) // 2, (side - im.height) // 2))
            (out / "icons").mkdir(exist_ok=True)
            square.save(out / "icons" / f"{key}.webp", "WEBP", quality=92, method=6)
            blob = (out / "icons" / f"{key}.webp").read_bytes()
            icons[key]["icon"] = f"{SITE}/icons/{key}.webp?v={version(blob)}"

    data = {"revision": revision, "maps": data_maps, "artefacts": icons, "hud": hud}
    script = ('"use strict";\n// Built by srp-026g/scripts/maps-tab/build.py; do not edit.\n'
              f"const MAPS = {json.dumps(data, ensure_ascii=False, separators=(',', ':'))};\n"
              + (HERE / "ui.js").read_text())
    (out / "maps.js").write_text(script)
    template = (HERE / "template.html").read_text()
    faces = "\n".join(line for line in (SRP / "servers" / "web-server" / "damage" / "damage.html").read_text().splitlines()
                      if line.startswith("@font-face"))
    page = (template.replace("/*__FONT__*/", faces)
            .replace("__SCRIPT__", f"{SITE}/maps.js?v={version(script.encode())}"))
    (out / "maps.html").write_text(page)
    served = ["maps.js"] + [f"{sub}/{f.name}" for sub in ("minimap", "icons") for f in sorted((out / sub).iterdir())]
    (out / "files.rs").write_text(
        "// Built by srp-026g/scripts/maps-tab/build.py; do not edit. The files maps.html names.\n"
        "files! {\n" + "".join(
            f'    "{f}" => "{TYPES[f.rsplit(".", 1)[1]]}",\n' for f in served)
        + "}\n")
    (out / "built-from.tsv").write_text(
        "# The patches the Maps tab was built from (srp-026g/scripts/maps-tab/build.py): name, sha256.\n"
        + "".join(f"{name}\t{sha}\n" for name, sha, _ in rows))
    points = sum(len(mo["respawn"]) + len(mo["spawners"]) + len(mo["stations"]) + len(mo["containers"])
                 for m in data_maps for mo in m["modes"]) + sum(len(m["artefacts"]) for m in data_maps)
    print(f"{out}: revision {revision}, {len(data_maps)} maps, {points} points, maps.js {len(script) / 1024:.0f} KB")


if __name__ == "__main__":
    main()
