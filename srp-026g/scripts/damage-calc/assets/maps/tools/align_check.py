#!/usr/bin/env python3
"""Independent check of a render's frame against the native HUD minimap.

    align_check.py RENDER_DIR KEY NATIVE_PNG SIZE OUT_JSON

The native 0.26g minimap bitmap was drawn by the developers for the HUD frame
{size, posX, posY}. Buildings are dark blocks in it. In the render, buildings
are where the top surface stands > 2.5 m above the terrain. Both masks are
compared at 2 px/m while the native one is scaled by k about the frame centre
and shifted by (dx, dz) metres; the best (k, dx, dz) should be (1, 0, 0) when
the render's world transform equals the HUD's.
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage


def main():
    d, key, native, size, out = Path(sys.argv[1]), sys.argv[2], sys.argv[3], float(sys.argv[4]), sys.argv[5]
    R = 2.0                                             # px per metre for the comparison
    N = int(round(2 * size * R))
    h = np.load(d / f'{key}-height.npy').astype(np.float64)
    g = np.load(d / f'{key}-ground.npy').astype(np.float64)
    hf = np.where(np.isfinite(h), h, np.nanmin(h))
    opened = ndimage.grey_opening(hf, size=int(h.shape[0] / (2 * size) * 30))     # 30 m
    ground = np.where(np.isfinite(g), np.minimum(g, hf), opened)
    tall = (hf - ground) > 2.5
    rmask = np.asarray(Image.fromarray(tall.astype(np.uint8) * 255).resize((N, N), Image.BOX), np.float32) > 127
    nat = np.asarray(Image.open(native).convert('RGBA'), np.float32)
    lum = nat[..., :3] @ np.array([0.299, 0.587, 0.114])
    dark = ((lum < 105) & (nat[..., 3] > 120)).astype(np.uint8) * 255
    area = nat[..., 3] > 40
    nimg = Image.fromarray(dark)
    aimg = Image.fromarray(area.astype(np.uint8) * 255)
    c = N / 2
    best = None; table = {}
    for k in np.arange(0.95, 1.0501, 0.005):
        # native bitmap resized to N*k, centred on the frame centre, then shifted
        M = int(round(N * k))
        nm = np.asarray(nimg.resize((M, M), Image.BILINEAR), np.float32) > 127
        am = np.asarray(aimg.resize((M, M), Image.NEAREST), np.float32) > 127
        for dx in np.arange(-8, 8.01, 0.5):
            for dz in np.arange(-8, 8.01, 0.5):
                ox = int(round(c - M / 2 + dx * R)); oy = int(round(c - M / 2 - dz * R))
                canvas = np.zeros((N, N), bool); acanvas = np.zeros((N, N), bool)
                x0, y0 = max(ox, 0), max(oy, 0); x1, y1 = min(ox + M, N), min(oy + M, N)
                canvas[y0:y1, x0:x1] = nm[y0 - oy:y1 - oy, x0 - ox:x1 - ox]
                acanvas[y0:y1, x0:x1] = am[y0 - oy:y1 - oy, x0 - ox:x1 - ox]
                rm = rmask & acanvas                     # compare inside the native play area only
                inter = (rm & canvas).sum(); union = (rm | canvas).sum()
                iou = inter / max(union, 1)
                table[(round(k, 3), dx, dz)] = iou
                if best is None or iou > best[0]:
                    best = (iou, round(float(k), 3), float(dx), float(dz))
    res = {'key': key, 'iou_identity': round(float(table[(1.0, 0.0, 0.0)]), 4),
           'best': {'iou': round(float(best[0]), 4), 'scale': best[1], 'dx_m': best[2], 'dz_m': best[3]},
           'search': 'scale 0.95..1.05 step 0.005, shift +-8 m step 0.5 m, at 2 px/m'}
    Path(out).write_text(json.dumps(res, indent=1))
    print(json.dumps(res))


if __name__ == '__main__':
    main()
