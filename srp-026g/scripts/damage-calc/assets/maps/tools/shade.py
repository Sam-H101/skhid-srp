#!/usr/bin/env python3
"""Shade a top-down albedo with its height map and frame it with the native
minimap's play area.

    shade.py RENDER_DIR KEY NATIVE_MINIMAP_PNG OUT.png [--ppm 6] [--area alpha|alpha:<t>|line]
             [--look classic|natural|vivid|soft]

Lighting and grading: look.py (the sun, its cast shadows, sky light and
ambient occlusion from the height map, exposure and colour). The native
HUD bitmap (same square frame) gives the play area (minimap_area.py): outside
it the render is dimmed, and its edge is drawn as a light line.
"""
import argparse
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageFilter
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
from look import shade_rgb  # noqa: E402
from minimap_area import area_mask  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('dir'); ap.add_argument('key'); ap.add_argument('native'); ap.add_argument('out')
    ap.add_argument('--ppm', type=float, default=6)
    ap.add_argument('--area', default='alpha')
    ap.add_argument('--look', default='classic', help='look.py: classic, natural, vivid, soft')
    a = ap.parse_args()
    d = Path(a.dir)
    alb = np.load(d / f'{a.key}-albedo.npy').astype(np.float32)
    h = np.load(d / f'{a.key}-height.npy').astype(np.float64)
    W = h.shape[0]
    empty = ~np.isfinite(h)
    fill = np.nanmin(h) if np.isfinite(h).any() else 0
    # fill holes from the nearest covered pixel
    idx = ndimage.distance_transform_edt(empty, return_distances=False, return_indices=True)
    hf = h[tuple(idx)]
    albf = alb[tuple(idx)]
    # play area from the native minimap, in this frame
    if a.native == 'none':          # no native minimap: no play-area frame
        area = np.ones((W, W), bool); draw_edge = False
    else:
        area = area_mask(a.native, W, a.area); draw_edge = True
    rgb = shade_rgb(albf, hf, area, empty, a.ppm, a.look)
    rgb[empty] = (30, 32, 34)
    area_f = ndimage.gaussian_filter(area.astype(np.float32), 1.0)
    rgb = rgb * (0.62 + 0.38 * area_f[..., None])
    edge = area ^ ndimage.binary_erosion(area, iterations=2)
    edge = ndimage.gaussian_filter(edge.astype(np.float32), 0.7)
    if draw_edge:
        rgb = rgb * (1 - edge[..., None]) + np.array([235, 225, 120]) * edge[..., None]
    img = Image.fromarray(rgb.clip(0, 255).astype(np.uint8), 'RGB')
    img = img.filter(ImageFilter.UnsharpMask(radius=1.2, percent=40, threshold=2) if a.look == 'classic'
                     else ImageFilter.UnsharpMask(radius=1.0, percent=70, threshold=2))
    img.save(a.out)
    np.save(d / f'{a.key}-area.npy', area)
    print(a.out, img.size)


if __name__ == '__main__':
    main()
