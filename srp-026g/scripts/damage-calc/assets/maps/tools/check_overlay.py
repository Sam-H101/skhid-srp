#!/usr/bin/env python3
"""Plot known world positions of a level onto its top-down image.

    check_overlay.py IMAGE KEY SIZE POSX POSY OUT.png PROJECT_FILE [PROJECT_FILE...]

Each PROJECT_FILE is a raw client_project (bincfg). Markers:
  mode 2/3/4 respawn points: team 0 = cyan circle, team 1 = red circle, other = white
  battery containers (victory_items_containers): yellow square
  battery spawners (victory_item_spawners, mode 2): green dot
  mode 3 charging stations (victory_item_spawn_points): magenta diamond
A 50 m scale bar is drawn bottom-left. Prints how many points fall outside
the image and how many land on empty (never-rendered) pixels.
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bincfg  # noqa: E402


def main():
    img_path, key, size, posx, posy, out = sys.argv[1], sys.argv[2], *map(float, sys.argv[3:6]), sys.argv[6]
    projects = [a for a in sys.argv[7:] if not a.startswith('--height=')]
    hpath = next((a.split('=', 1)[1] for a in sys.argv[7:] if a.startswith('--height=')), None)
    pts = []
    im = Image.open(img_path).convert('RGB')
    W, H = im.size
    s = W / (2 * size)
    to_px = lambda x, z: ((x - posx + size) * s, (posy - z + size) * s)
    d = ImageDraw.Draw(im)
    stats = {'points': 0, 'outside': 0}
    r = max(3, W // 300)
    for pf in projects:
        p = bincfg.load(pf)
        so = p.get('server_objects', {})
        for o in so.get('respawn_points', []):
            x, _, z = o['position']; u, v = to_px(x, z); pts.append(o['position'])
            col = {0: (0, 230, 255), 1: (255, 60, 60)}.get(o.get('team'), (255, 255, 255))
            d.ellipse([u - r, v - r, u + r, v + r], outline=col, width=2)
            stats['points'] += 1; stats['outside'] += not (0 <= u < W and 0 <= v < H)
        for o in so.get('victory_items_containers', []):
            x, _, z = o['position']; u, v = to_px(x, z); pts.append(o['position'])
            d.rectangle([u - r - 2, v - r - 2, u + r + 2, v + r + 2], outline=(255, 230, 0), width=3)
            stats['points'] += 1; stats['outside'] += not (0 <= u < W and 0 <= v < H)
        for o in so.get('victory_item_spawners', []):
            x, _, z = o['position']; u, v = to_px(x, z); pts.append(o['position'])
            d.ellipse([u - 2, v - 2, u + 2, v + 2], fill=(60, 255, 60))
            stats['points'] += 1; stats['outside'] += not (0 <= u < W and 0 <= v < H)
        sp = so.get('victory_item_spawn_points', [])
        if isinstance(sp, dict):
            sp = list(sp.values())
        for o in sp:
            if not isinstance(o, dict) or 'position' not in o:
                continue
            x, _, z = o['position']; u, v = to_px(x, z); pts.append(o['position'])
            q = r + 4
            d.polygon([(u, v - q), (u + q, v), (u, v + q), (u - q, v)], outline=(255, 0, 255), width=3)
            stats['points'] += 1; stats['outside'] += not (0 <= u < W and 0 <= v < H)
    # scale bar 50 m
    L = 50 * s; x0, y0 = 20, H - 30
    d.rectangle([x0 - 4, y0 - 18, x0 + L + 40, y0 + 10], fill=(0, 0, 0))
    d.line([x0, y0, x0 + L, y0], fill=(255, 255, 255), width=4)
    d.line([x0, y0 - 8, x0, y0 + 6], fill=(255, 255, 255), width=2)
    d.line([x0 + L, y0 - 8, x0 + L, y0 + 6], fill=(255, 255, 255), width=2)
    d.text((x0 + L + 6, y0 - 12), '50 m', fill=(255, 255, 255))
    d.text((10, 10), f'{key}: +x right, +z up; {1 / s:.4f} m/px', fill=(255, 255, 255))
    im.quantize(256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).save(out, optimize=True)
    if hpath:
        # Height test: a placed object must stand on (or under) rendered geometry
        # at its pixel: y <= top surface + 0.5 m. A wrong transform leaves
        # points floating over lower ground. Negative controls: the same test
        # with the image mirrored in z and with a 5 m shift.
        h = np.load(hpath).astype(np.float64)
        hw = h.shape[0]; hs = hw / (2 * size)
        P = np.array(pts)

        def test(xs, zs, ys):
            u = np.floor((xs - posx + size) * hs).astype(int); v = np.floor((posy - zs + size) * hs).astype(int)
            ok = (u >= 0) & (v >= 0) & (u < hw) & (v < hw)
            top = np.full(len(xs), np.nan)
            top[ok] = h[v[ok], u[ok]]
            # top surface within 0.35 m around the point (feet may sit on an edge)
            k = max(1, int(round(0.35 * hs)))
            from scipy import ndimage
            hm = ndimage.maximum_filter(np.nan_to_num(h, nan=-1e9), size=2 * k + 1)
            top_n = np.full(len(xs), np.nan); top_n[ok] = hm[v[ok], u[ok]]
            floating = ~(ys <= top_n + 0.5)
            on_top = np.abs(top - ys) < 1.0
            return dict(n=int(len(xs)), floating=int(floating.sum()), on_top_surface=int(on_top.sum()),
                        covered_or_indoor=int((~floating & ~on_top).sum()))
        stats['height_test'] = test(P[:, 0], P[:, 2], P[:, 1])
        stats['control_mirrored_z'] = test(P[:, 0], 2 * posy - P[:, 2], P[:, 1])
        stats['control_shift_5m'] = test(P[:, 0] + 5, P[:, 2] + 5, P[:, 1])
    Path(out).with_suffix('.json').write_text(json.dumps(dict(stats, key=key), indent=1))
    print(json.dumps(dict(stats, key=key, out=out)))


if __name__ == '__main__':
    main()
