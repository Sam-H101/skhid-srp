#!/usr/bin/env python3
"""Extract what a top-down render of one 0.26g project needs from resources.db.

    extract_level.py DBGRAB RESOURCES_DB PROJECT(e.g. level_01/day_2) OUT_DIR [PACK_FILES_DIR]

With PACK_FILES_DIR (a content pack's files/ tree, e.g. a ported map), files
are taken from it first and from the database otherwise.

OUT_DIR/project            decoded client_project (bincfg JSON)
OUT_DIR/models/<lib>.model/{settings,render/<surface>/converted_model}
OUT_DIR/material_instances/<name>.material
OUT_DIR/tex/<name>.png      diffuse textures downscaled to <= 256 px (DDS dropped)
Only LOD0 render data, materials and their diffuse texture are kept, to stay small.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bincfg  # noqa: E402
from PIL import Image  # noqa: E402

DIFFUSE_SLOTS = ('texture_diffuse', 'texture_detail_0', 'texture_vertex_blended_diffuse')


OVERLAY = None   # a pack's files/ directory, searched before the database


def grab(dbgrab, db, prefix, dest):
    """Extract every file under prefix into dest, restoring '/' layout."""
    if OVERLAY is not None:
        src = OVERLAY / prefix.rstrip('/')
        if src.is_file():
            (dest / prefix).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dest / prefix)
            return 1
        if prefix.endswith('/') and src.is_dir():
            shutil.copytree(src, dest / prefix.rstrip('/'), dirs_exist_ok=True)
            return 1
    with tempfile.TemporaryDirectory(dir=dest.parent if dest.parent.exists() else None) as tmp:
        out = subprocess.run([dbgrab, db, 'get', prefix, tmp], check=True, capture_output=True, text=True).stdout
        flat_prefix = prefix.replace('/', '__')
        n = 0
        for name in out.split():
            if not name.startswith(flat_prefix):
                continue
            rel = prefix + name[len(flat_prefix):].replace('__', '/')
            target = dest / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(os.path.join(tmp, name), target)
            n += 1
        return n


def leaves(d, p=''):
    if isinstance(d, dict):
        for k, v in d.items():
            yield from leaves(v, f'{p}.{k}' if p else k)
    elif isinstance(d, list):
        for i, v in enumerate(d):
            yield from leaves(v, f'{p}[{i}]')
    else:
        yield p, d


def main():
    global OVERLAY
    dbgrab, db, project, out = sys.argv[1], sys.argv[2], sys.argv[3], Path(sys.argv[4])
    if len(sys.argv) > 5:
        OVERLAY = Path(sys.argv[5])
    out.mkdir(parents=True, exist_ok=True)
    grab(dbgrab, db, f'projects/{project}/client_project', out)
    proj = bincfg.load(out / 'projects' / project / 'client_project')
    (out / 'project.json').write_text(json.dumps(proj, default=list))
    libs = sorted({o['lib_name'] for o in proj.get('render_visuals', [])})
    materials = set()
    for lib in libs:
        base = f'models/{lib}.model/'
        if not (out / base / 'settings').exists():
            grab(dbgrab, db, base + 'settings', out)
            grab(dbgrab, db, base + 'render/', out)
        sp = out / base / 'settings'
        if not sp.exists():
            print('missing model', lib, file=sys.stderr)
            continue
        s = bincfg.load(sp)
        for surf, ms in s.get('material_settings', {}).items():
            if isinstance(ms, dict) and ms.get('material_name'):
                materials.add(ms['material_name'])
        # drop non-LOD0 surfaces to save space
        lod0 = set(s.get('lod_hierrarchy', {}).get('LOD0', {}).get('surfaces', []))
        rd = out / base / 'render'
        if rd.exists():
            for d in rd.iterdir():
                if d.is_dir() and d.name not in lod0:
                    shutil.rmtree(d)
    textures = set()
    for m in sorted(materials):
        p = out / 'material_instances' / (m + '.material')
        if not p.exists():
            grab(dbgrab, db, f'material_instances/{m}.material', out)
        if not p.exists():
            print('missing material', m, file=sys.stderr)
            continue
        for path, v in leaves(bincfg.load(p)):
            if isinstance(v, str) and v and path.endswith('.value') and path.split('.')[-2] in DIFFUSE_SLOTS:
                textures.add(v)
    (out / 'tex').mkdir(exist_ok=True)
    for t in sorted(textures):
        png = out / 'tex' / (t + '.png')
        if png.exists():
            continue
        grab(dbgrab, db, f'textures/{t}.dds', out)
        dds = out / 'textures' / (t + '.dds')
        if not dds.exists():
            print('missing texture', t, file=sys.stderr)
            continue
        try:
            im = Image.open(dds).convert('RGBA')
            if max(im.size) > 256:
                im = im.resize((256, 256 * im.size[1] // im.size[0]), Image.BOX)
            png.parent.mkdir(parents=True, exist_ok=True)
            im.save(png)
        except Exception as e:  # noqa: BLE001
            print('bad texture', t, e, file=sys.stderr)
        dds.unlink()
    shutil.rmtree(out / 'textures', ignore_errors=True)
    print(json.dumps({'project': project, 'models': len(libs), 'materials': len(materials),
                      'textures': len(textures)}))


if __name__ == '__main__':
    main()
