"""Adds each map's play area to maps.json: the box, in render pixels, around
the native HUD minimap's play area (minimap_area.py; shade.py outlines it),
with a margin. The calculator shows that box rather than the whole square.

usage: play_area.py <maps.json> <hud_images dir>   (ffdec export of hud.swf)
"""
import json
from pathlib import Path
import re
import sys

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_all import NATIVE, PATCHED  # noqa: E402  (the _levels bitmap per project, and its kind)
from minimap_area import area_mask  # noqa: E402

BITMAPS = {key: (m['bitmap'], m.get('area', 'alpha')) for key, m in {**NATIVE, **PATCHED}.items()}
MARGIN = 0.04  # of the image side, around the play area

path, hud = sys.argv[1], sys.argv[2]
data = json.load(open(path, encoding="utf-8"))
for key, m in data["maps"].items():
    W, H = m["image_size_px"]
    if key not in BITMAPS:  # the 0.21d maps: no native minimap of their layout
        m["play_area_px"] = None
        print(key, "whole image")
        continue
    bitmap, kind = BITMAPS[key]
    area = area_mask(f"{hud}/{bitmap}", W, kind)
    ys, xs = np.nonzero(area)
    pad = round(MARGIN * W)
    x0, y0 = max(0, xs.min() - pad), max(0, ys.min() - pad)
    x1, y1 = min(W, xs.max() + 1 + pad), min(H, ys.max() + 1 + pad)
    m["play_area_px"] = [int(x0), int(y0), int(x1 - x0), int(y1 - y0)]
    print(key, m["play_area_px"], f"{(x1 - x0) * (y1 - y0) / (W * H):.0%} of the image")
# As make_json.py writes it: number rows (elevation) on one line each.
text = json.dumps(data, indent=1)
text = re.sub(r'\[\s*(-?[0-9.]+(?:,\s*-?[0-9.]+)*)\s*\]', lambda mm: '[' + re.sub(r'\s+', '', mm.group(1)).replace(',', ', ') + ']', text)
open(path, "w", encoding="utf-8").write(text + "\n")
