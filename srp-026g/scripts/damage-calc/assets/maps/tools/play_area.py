"""Adds each map's play area to maps.json: the box, in render pixels, around
the native HUD minimap's opaque part (the area shade.py outlines in yellow),
with a margin. The calculator shows that box rather than the whole square.

usage: play_area.py <maps.json> <hud_images dir>   (ffdec export of hud.swf)
"""
import json
import re
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

BITMAPS = {  # as build_all.py: the _levels bitmap per project
    "level_01": "967_minimap_rudnya_minimap_rudnya.png", "level_02": "499_minimap_school_minimap_school.png",
    "level_03": "976_minimap_base_minimap_base.png", "level_04": "911_minimap_zavod_minimap_zavod.png",
    "level_05": "955_minimap_fort_minimap_fort.png", "level_06": "975_Map_06_Map_06.png",
    "level_07": "974_Map_07_Map_07.png", "level_08": "972_Map_08_Map_08.png",
}
MARGIN = 0.04  # of the image side, around the play area

path, hud = sys.argv[1], sys.argv[2]
data = json.load(open(path, encoding="utf-8"))
for key, m in data["maps"].items():
    W, H = m["image_size_px"]
    if key not in BITMAPS:  # the 0.21d maps: no native minimap of their layout
        m["play_area_px"] = None
        print(key, "whole image")
        continue
    alpha = np.asarray(Image.open(f"{hud}/{BITMAPS[key]}").convert("RGBA").resize((W, H), Image.BILINEAR), np.float32)[..., 3] / 255
    area = ndimage.binary_fill_holes(alpha > 0.15)
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
