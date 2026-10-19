#!/usr/bin/env python3
"""Shade a top-down albedo with its height map and frame it with the native
minimap's play area.

    shade.py RENDER_DIR KEY NATIVE_MINIMAP_PNG OUT.png [--ppm 6] [--area alpha|alpha:<t>|line]

Lighting: Lambert from a sun in the north-west at 45 degrees on the height
map's normals, plus cast shadows marched along the sun direction. The native
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
from minimap_area import area_mask  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('dir'); ap.add_argument('key'); ap.add_argument('native'); ap.add_argument('out')
    ap.add_argument('--ppm', type=float, default=6)
    ap.add_argument('--area', default='alpha')
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
    px = 1.0 / a.ppm                                      # metres per pixel
    gy, gx = np.gradient(ndimage.gaussian_filter(hf, 0.7), px)   # d/dy(px rows: -z), d/dx(+x)
    # normal in (x, y-up, z) with image rows pointing to -z
    nx, nz = -gx, gy
    n = np.stack([nx, np.ones_like(nx), nz], -1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    sun = np.array([-1.0, 2.0, 1.0]); sun /= np.linalg.norm(sun)    # from NW (-x, +z), elevated
    lam = np.clip(n @ sun, 0, 1)
    # cast shadows: march toward the sun (image: left = -x, up = +z)
    horiz = np.hypot(sun[0], sun[2]); tan_e = sun[1] / horiz
    dx_img, dy_img = sun[0] / horiz, -sun[2] / horiz             # image step per metre toward the sun
    shadow = np.zeros_like(hf, bool)
    for step_m in np.arange(0.5, 30, 0.5):
        sh = ndimage.shift(hf, (-dy_img * step_m * a.ppm, -dx_img * step_m * a.ppm), order=0, mode='nearest')
        shadow |= sh > hf + step_m * tan_e + 0.15
    shadow = ndimage.gaussian_filter(shadow.astype(np.float32), 0.8)
    light = 0.55 + 0.6 * lam * (1 - 0.5 * shadow)
    # gentle height tint: higher = slightly brighter (helps read roofs)
    ha = hf[area] if area.any() else hf.ravel()
    rel = (hf - np.percentile(ha, 5)) / max(np.percentile(ha, 99) - np.percentile(ha, 5), 1)
    light *= 0.9 + 0.2 * np.clip(rel, 0, 1)
    # albedo: tame saturation and normalise exposure
    lum = albf @ np.array([0.299, 0.587, 0.114], np.float32)
    albf = lum[..., None] * 0.35 + albf * 0.65
    sel = lum[area & ~empty] if (area & ~empty).any() else lum.ravel()
    lo, hi = np.percentile(sel, 2), np.percentile(sel, 99)
    albf = 25 + (albf - lo) * (215 / max(hi - lo, 1))
    rgb = albf * light[..., None] * 1.1
    rgb[empty] = (30, 32, 34)
    area_f = ndimage.gaussian_filter(area.astype(np.float32), 1.0)
    rgb = rgb * (0.62 + 0.38 * area_f[..., None])
    edge = area ^ ndimage.binary_erosion(area, iterations=2)
    edge = ndimage.gaussian_filter(edge.astype(np.float32), 0.7)
    if draw_edge:
        rgb = rgb * (1 - edge[..., None]) + np.array([235, 225, 120]) * edge[..., None]
    img = Image.fromarray(rgb.clip(0, 255).astype(np.uint8), 'RGB')
    img = img.filter(ImageFilter.UnsharpMask(radius=1.2, percent=40, threshold=2))
    img.save(a.out)
    np.save(d / f'{a.key}-area.npy', area)
    print(a.out, img.size)


if __name__ == '__main__':
    main()
