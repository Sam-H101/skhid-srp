"""How a top-down render is lit and graded: albedo + height map -> RGB.

    rgb = shade_rgb(albedo, height, area, empty, ppm, look)

albedo (W, W, 3) 0-255 as render_topdown.py samples it, height (W, W) the top
surface in metres (holes already filled), area the play area (bool), empty the
never-rendered pixels, ppm pixels per metre. Looks:

  classic  the first renders: Lambert sun + flat ambient, hard cast shadows,
           albedo desaturated by 35 % and stretched (washed, grey).
  natural  linear-light: a warm sun (north-west, 42 degrees) with soft cast
           shadows whose penumbra grows with distance, a cool sky term with
           ambient occlusion from the height map, a filmic tone curve with the
           exposure that puts the play area's median at the look's target, and
           a gentle saturation/warmth grade.
  vivid    natural with deeper shadows and occlusion, more saturation and
           contrast, warmer light.
  soft     natural with light shadows and more sky: an airy, low-contrast
           satellite look.
"""
import numpy as np
from scipy import ndimage

LUMA = np.array([0.2126, 0.7152, 0.0722], np.float32)

LOOKS = {
    'natural': dict(sun=(1.0, 0.96, 0.9), sky=(0.8, 0.86, 1.0), sun_k=1.0, sky_k=0.45, shadow=0.8,
                    ao=0.6, sat=1.22, warm=(1.02, 1.0, 0.98), target=0.43, contrast=0.04),
    'vivid': dict(sun=(1.0, 0.95, 0.86), sky=(0.76, 0.84, 1.0), sun_k=1.05, sky_k=0.38, shadow=0.86,
                  ao=0.72, sat=1.38, warm=(1.03, 1.0, 0.97), target=0.42, contrast=0.12),
    'soft': dict(sun=(1.0, 0.97, 0.92), sky=(0.82, 0.87, 1.0), sun_k=0.85, sky_k=0.6, shadow=0.55,
                 ao=0.45, sat=1.12, warm=(1.02, 1.0, 0.98), target=0.47, contrast=-0.03),
}


def _classic(alb, hf, area, empty, ppm):
    px = 1.0 / ppm
    gy, gx = np.gradient(ndimage.gaussian_filter(hf, 0.7), px)
    n = np.stack([-gx, np.ones_like(gx), gy], -1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    sun = np.array([-1.0, 2.0, 1.0]); sun /= np.linalg.norm(sun)
    lam = np.clip(n @ sun, 0, 1)
    horiz = np.hypot(sun[0], sun[2]); tan_e = sun[1] / horiz
    dx_img, dy_img = sun[0] / horiz, -sun[2] / horiz
    shadow = np.zeros_like(hf, bool)
    for step_m in np.arange(0.5, 30, 0.5):
        sh = ndimage.shift(hf, (-dy_img * step_m * ppm, -dx_img * step_m * ppm), order=0, mode='nearest')
        shadow |= sh > hf + step_m * tan_e + 0.15
    shadow = ndimage.gaussian_filter(shadow.astype(np.float32), 0.8)
    light = 0.55 + 0.6 * lam * (1 - 0.5 * shadow)
    ha = hf[area] if area.any() else hf.ravel()
    rel = (hf - np.percentile(ha, 5)) / max(np.percentile(ha, 99) - np.percentile(ha, 5), 1)
    light *= 0.9 + 0.2 * np.clip(rel, 0, 1)
    lum = alb @ np.array([0.299, 0.587, 0.114], np.float32)
    alb = lum[..., None] * 0.35 + alb * 0.65
    sel = lum[area & ~empty] if (area & ~empty).any() else lum.ravel()
    lo, hi = np.percentile(sel, 2), np.percentile(sel, 99)
    alb = 25 + (alb - lo) * (215 / max(hi - lo, 1))
    return alb * light[..., None] * 1.1


def _soft_shadow(hf, ppm, sun):
    """0..1 occlusion of the sun: march toward it; an occluder d metres away
    casts a penumbra 0.25 + 0.06 d metres wide (a sun disc plus haze)."""
    horiz = np.hypot(sun[0], sun[2]); tan_e = sun[1] / horiz
    dx_img, dy_img = sun[0] / horiz, -sun[2] / horiz
    occ = np.zeros_like(hf, np.float32)
    step_m, d = 0.25, 0.25
    while d < 45:
        sh = ndimage.shift(hf, (-dy_img * d * ppm, -dx_img * d * ppm), order=0, mode='nearest')
        occ = np.maximum(occ, np.clip((sh - hf - d * tan_e) / (0.25 + 0.06 * d), 0, 1))
        d += step_m
        step_m = min(step_m * 1.08, 1.0)
    return ndimage.gaussian_filter(occ, 0.9)


def _ambient_occlusion(hf, ppm):
    """1 = open sky, lower in creases, at wall feet and between buildings:
    how far the surroundings rise above a pixel at several scales."""
    occ = np.zeros_like(hf, np.float32)
    for sigma_m, w in ((0.4, 0.2), (1.2, 0.3), (3.5, 0.3), (9.0, 0.2)):
        rise = ndimage.gaussian_filter(hf, sigma_m * ppm) - hf
        occ += w * np.clip(rise / (1.2 * sigma_m), 0, 1)
    return 1 - np.clip(occ, 0, 1)


def _aces(x):
    return np.clip((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0, 1)


def shade_rgb(alb, hf, area, empty, ppm, look='natural'):
    if look == 'classic':
        return _classic(alb, hf, area, empty, ppm)
    p = LOOKS[look]
    sel = area & ~empty if (area & ~empty).any() else ~empty
    a = (np.clip(alb, 0, 255) / 255.0) ** 2.2                      # linear albedo
    px = 1.0 / ppm
    gy, gx = np.gradient(ndimage.gaussian_filter(hf, 1.0), px)
    n = np.stack([-gx, np.ones_like(gx), gy], -1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    sun = np.array([-0.72, 1.0, 0.62]); sun /= np.linalg.norm(sun)  # north-west, ~42 degrees up
    lam = np.clip(n @ sun, 0, 1)
    shadow = _soft_shadow(hf, ppm, sun)
    ao = _ambient_occlusion(hf, ppm)
    ao_k = 1 - p['ao'] * (1 - ao)
    sky = 0.5 + 0.5 * n[..., 1]
    light = (np.array(p['sun']) * (p['sun_k'] * lam * (1 - p['shadow'] * shadow) * (0.75 + 0.25 * ao_k))[..., None]
             + np.array(p['sky']) * (p['sky_k'] * sky * ao_k)[..., None])
    lin = a * light
    # exposure: the play area's median brightness after the tone curve is the look's target
    med = float(np.median((lin @ LUMA)[sel]))
    lo, hi = -4.0, 4.0
    for _ in range(14):
        ev = (lo + hi) / 2
        out = _aces(med * 2 ** ev) ** (1 / 2.2)
        lo, hi = (ev, hi) if out < p['target'] else (lo, ev)
    rgb = _aces(lin * 2 ** ev) ** (1 / 2.2)
    if p['contrast']:
        rgb = np.clip(0.5 + (rgb - 0.5) * (1 + p['contrast']), 0, 1)
    l = (rgb @ LUMA)[..., None]
    rgb = np.clip(l + (rgb - l) * p['sat'], 0, 1) * np.array(p['warm'])
    return rgb * 255
