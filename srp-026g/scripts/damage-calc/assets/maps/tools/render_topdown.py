#!/usr/bin/env python3
"""Orthographic top-down render of a 0.26g level, in the HUD minimap's frame.

    render_topdown.py LEVEL_DIR KEY SIZE POSX POSY PX_PER_M OUT_DIR [--ceiling M]

LEVEL_DIR is what extract_level.py wrote. The square covers 2*SIZE metres
around (POSX, POSY) exactly like the 0.26g HUD (hud.swf Minimap.setProps):

    px = (x - POSX + SIZE) * s      py = (POSY - z + SIZE) * s,   s = PX_PER_M

so +x is to the right and +z is up. Writes OUT_DIR/<KEY>-render.png (RGB),
OUT_DIR/<KEY>-height.npy (top surface height per pixel, NaN where empty) and
OUT_DIR/<KEY>-ground.npy (terrain-model height per pixel) and
OUT_DIR/<KEY>-solid.npy (the top of what is not alpha-tested: no foliage).

The rasterizer (subdivide/rasterize) is copied from the playable-map-pack's
render_minimap.py.
"""
import argparse
import json
import math
from pathlib import Path
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / 'gen-data-026g'))  # bincfg
import bincfg  # noqa: E402


def chunks(b):
    out = {}; off = 0
    while off < len(b):
        kind, size = struct.unpack_from('<II', b, off); off += 8
        out[kind] = b[off:off + size]; off += size
    return out


def rotation(a):
    sx, sy, sz = map(math.sin, a)
    cx, cy, cz = map(math.cos, a)
    return np.array([[cy*cz, -cy*sz, sy],
                     [sy*sx*cz+cx*sz, -sx*sy*sz+cx*cz, -sx*cy],
                     [-cx*sy*cz+sx*sz, cx*sy*sz+sx*cz, cx*cy]])


def place(points, o):
    s = np.array(o.get('scale', [1, 1, 1]), dtype=np.float64)
    return (points * s) @ rotation(o.get('rotation', [0, 0, 0])) + np.array(o.get('position', [0, 0, 0]))


def leaves(d, p=''):
    if isinstance(d, dict):
        for k, v in d.items():
            yield from leaves(v, f'{p}.{k}' if p else k)
    elif isinstance(d, list):
        for i, v in enumerate(d):
            yield from leaves(v, f'{p}[{i}]')
    else:
        yield p, d


SKIP = ('/glas', 'glass', '/glow', '/lights', '/fx/', 'decal', '/sky', 'shadow', 'invisible', 'collision',
        'underground_black', 'occluder', 'portal', 'fog', 'lightshaft', 'light_shaft', 'godray')


class Materials:
    def __init__(self, root):
        self.root = root; self.cache = {}; self.tex = {}

    def texture(self, name):
        """Mip pyramid (list of RGBA arrays, level 0 = full) or None."""
        if name not in self.tex:
            p = self.root / 'tex' / (name + '.png')
            if p.exists():
                im = Image.open(p).convert('RGBA')
                mips = [np.asarray(im, np.float32)]
                while min(im.size) > 1:
                    im = im.resize((max(1, im.size[0] // 2), max(1, im.size[1] // 2)), Image.BOX)
                    mips.append(np.asarray(im, np.float32))
                self.tex[name] = mips
            else:
                self.tex[name] = None
        return self.tex[name]

    def get(self, name):
        """(rgba mips or None, flat rgb or None, alpha_tested, blend mips or None)."""
        if name in self.cache:
            return self.cache[name]
        res = (None, None, False, None)
        p = self.root / 'material_instances' / (name + '.material')
        low = name.lower()
        if p.exists() and not any(k in low for k in SKIP):
            cfg = bincfg.load(p)
            lv = list(leaves(cfg))
            slots = {path.split('.')[-2]: v.strip() for path, v in lv
                     if isinstance(v, str) and v.strip() and path.endswith('.value')}
            flags = {path.split('.')[-2]: v for path, v in lv if isinstance(v, bool) and path.endswith('.value')}
            effect = (next((v for path, v in lv if path.endswith('effect_id')), '') or '').lower()
            colour = slots.get('texture_diffuse') or slots.get('texture_detail_0')
            if 'water' in effect or 'water' in low:
                res = (None, np.array([70., 84., 92.]), False, None)
            elif effect.startswith('forward') or any(k in effect for k in ('decal', 'glass', 'sky', 'particle')):
                pass
            elif colour and not any(k in colour.lower() for k in SKIP):
                t = self.texture(colour)
                if t is not None:
                    blend = None
                    if flags.get('use_vertex_blended_diffuse') and slots.get('texture_vertex_blended_diffuse'):
                        blend = self.texture(slots['texture_vertex_blended_diffuse'])
                    res = (t, None, bool(flags.get('use_alpha_test')), blend)
        self.cache[name] = res
        return res


def surfaces(root, project, materials, ceiling):
    for o in project.get('render_visuals', []):
        model = root / 'models' / (o['lib_name'] + '.model')
        sp = model / 'settings'
        if not sp.exists():
            continue
        s = bincfg.load(sp)
        terrain = 'terrain' in o['lib_name'].lower()
        for surf in s.get('lod_hierrarchy', {}).get('LOD0', {}).get('surfaces', []):
            data = model / 'render' / surf / 'converted_model'
            if not data.exists():
                continue
            mat = s.get('material_settings', {}).get(surf, {}).get('material_name', '')
            tex, flat, atest, blend = materials.get(mat)
            if tex is None and flat is None:
                continue
            c = chunks(data.read_bytes())
            n = struct.unpack_from('<I', c[3])[0]
            if n == 0:
                continue
            stride = (len(c[3]) - 4) // n
            raw = np.frombuffer(c[3], np.uint8, offset=4, count=n * stride).reshape(n, stride)
            pos = raw[:, :12].copy().view('<f4').reshape(-1, 3).astype(np.float64)
            uv = raw[:, 24:28].copy().view('<f2').reshape(-1, 2).astype(np.float64)
            ni = struct.unpack_from('<I', c[4])[0]
            isz = (len(c[4]) - 4) // max(ni, 1)
            ix = np.frombuffer(c[4], '<u4' if isz == 4 else '<u2', offset=4, count=ni).reshape(-1, 3).astype(np.int64)
            if ix.size == 0 or ix.max() >= n:
                continue
            tri = place(pos, o)[ix]
            keep = tri[:, :, 1].min(1) < ceiling
            if not keep.any():
                continue
            if blend is not None and stride >= 32:
                w = (raw[:, 31].astype(np.float64) / 255)[:, None]
            else:
                w = np.ones((n, 1)); blend = None
            yield tri[keep], np.concatenate([uv, w], 1)[ix][keep], tex, flat, atest, terrain, blend


# ---- rasterizer (from render_minimap.py) ----

def subdivide(tri, attr, limit, width, height):
    done_t, done_a = [], []
    while len(tri):
        lo, hi = tri[:, :, :2].min(1), tri[:, :, :2].max(1)
        seen = (hi[:, 0] >= 0) & (hi[:, 1] >= 0) & (lo[:, 0] <= width) & (lo[:, 1] <= height)
        tri, attr = tri[seen], attr[seen]
        span = tri[:, :, :2].max(1) - tri[:, :, :2].min(1)
        big = span.max(1) > limit
        done_t.append(tri[~big]); done_a.append(attr[~big])
        t, a = tri[big], attr[big]
        if not len(t):
            break
        m = lambda x, i, j: (x[:, i] + x[:, j]) / 2
        tm = [m(t, 0, 1), m(t, 1, 2), m(t, 2, 0)]
        am = [m(a, 0, 1), m(a, 1, 2), m(a, 2, 0)]
        pick = lambda x: np.stack(x, 1)
        tri = np.concatenate([pick([t[:, 0], tm[0], tm[2]]), pick([tm[0], t[:, 1], tm[1]]),
                              pick([tm[2], tm[1], t[:, 2]]), pick([tm[0], tm[1], tm[2]])])
        attr = np.concatenate([pick([a[:, 0], am[0], am[2]]), pick([am[0], a[:, 1], am[1]]),
                               pick([am[2], am[1], a[:, 2]]), pick([am[0], am[1], am[2]])])
    return np.concatenate(done_t), np.concatenate(done_a)


def rasterize(tri, attr, width, height):
    tri, attr = subdivide(tri, attr, 13, width, height)
    if not len(tri):
        return np.zeros(0, np.int64), np.zeros(0), np.zeros((0, attr.shape[2]))
    lo = np.floor(tri[:, :, :2].min(1) - 0.5).astype(np.int64)
    out_p, out_h, out_a = [], [], []
    for size in (2, 4, 8, 16, 32):
        span = np.ceil(tri[:, :, :2].max(1) - 0.5).astype(np.int64) - lo + 1
        sel = (span.max(1) <= size) & (span.max(1) > size // 2 if size > 2 else True)
        if not sel.any():
            continue
        t, a, l = tri[sel], attr[sel], lo[sel]
        gx, gy = np.meshgrid(np.arange(size), np.arange(size))
        px = l[:, None, 0] + gx.ravel()[None]
        py = l[:, None, 1] + gy.ravel()[None]
        cx, cy = px + 0.5, py + 0.5
        x0, y0 = t[:, 0, 0:1], t[:, 0, 1:2]
        x1, y1 = t[:, 1, 0:1], t[:, 1, 1:2]
        x2, y2 = t[:, 2, 0:1], t[:, 2, 1:2]
        den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        ok = np.abs(den) > 1e-12
        den = np.where(ok, den, 1.0)
        w0 = ((y1 - y2) * (cx - x2) + (x2 - x1) * (cy - y2)) / den
        w1 = ((y2 - y0) * (cx - x2) + (x0 - x2) * (cy - y2)) / den
        w2 = 1 - w0 - w1
        inside = ok & (w0 >= -1e-9) & (w1 >= -1e-9) & (w2 >= -1e-9) \
            & (px >= 0) & (py >= 0) & (px < width) & (py < height)
        k, j = np.nonzero(inside)
        w = np.stack([w0[k, j], w1[k, j], w2[k, j]], 1)
        out_p.append(py[k, j] * width + px[k, j])
        out_h.append((w * t[k, :, 2]).sum(1))
        out_a.append(np.einsum('ni,nij->nj', w, a[k]))
    if not out_p:
        return np.zeros(0, np.int64), np.zeros(0), np.zeros((0, attr.shape[2]))
    return np.concatenate(out_p), np.concatenate(out_h), np.concatenate(out_a)


def bilinear(t, u, v):
    """Wrapped bilinear sample of texture t (H, W, C) at uv."""
    th, tw = t.shape[:2]
    x = u * tw - 0.5; y = v * th - 0.5
    x0 = np.floor(x); y0 = np.floor(y); fx = (x - x0)[:, None]; fy = (y - y0)[:, None]
    x0 = x0.astype(np.int64) % tw; y0 = y0.astype(np.int64) % th
    x1 = (x0 + 1) % tw; y1 = (y0 + 1) % th
    return (t[y0, x0] * (1 - fx) + t[y0, x1] * fx) * (1 - fy) + (t[y1, x0] * (1 - fx) + t[y1, x1] * fx) * fy


def top(pixels, heights):
    order = np.lexsort((heights, pixels))
    p = pixels[order]
    last = np.r_[p[1:] != p[:-1], True]
    return p[last], order[last]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('level'); ap.add_argument('key')
    ap.add_argument('size', type=float); ap.add_argument('posx', type=float); ap.add_argument('posy', type=float)
    ap.add_argument('ppm', type=float); ap.add_argument('out')
    ap.add_argument('--ceiling', type=float, default=None)
    ap.add_argument('--mip-bias', type=float, default=0.75, help='extra mip levels (smoother, smaller files)')
    ap.add_argument('--blend-invert', action='store_true')
    a = ap.parse_args()
    root = Path(a.level); out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    project = json.loads((root / 'project.json').read_text())
    spawns = [r['position'] for r in project['server_objects'].get('respawn_points', [])]
    ceiling = a.ceiling if a.ceiling is not None else max(s[1] for s in spawns) + 45
    W = int(round(2 * a.size * a.ppm))
    s = a.ppm
    x0 = a.posx - a.size; ztop = a.posy + a.size
    zbuf = np.full(W * W, -np.inf); col = np.zeros((W * W, 3), np.float32)
    gbuf = np.full(W * W, -np.inf)
    sbuf = np.full(W * W, -np.inf)
    mats = Materials(root)
    ntri = 0
    for tri, uvw, tex, flat, atest, terrain, blend in surfaces(root, project, mats, ceiling):
        uv = uvw[..., :2]
        screen = np.stack([(tri[..., 0] - x0) * s, (ztop - tri[..., 2]) * s, tri[..., 1]], -1)
        # mip level per triangle: texels per screen pixel
        if tex is not None:
            th0, tw0 = tex[0].shape[:2]
            e1, e2 = screen[:, 1, :2] - screen[:, 0, :2], screen[:, 2, :2] - screen[:, 0, :2]
            sa = np.abs(e1[:, 0] * e2[:, 1] - e1[:, 1] * e2[:, 0]) / 2
            f1, f2 = (uv[:, 1] - uv[:, 0]) * (tw0, th0), (uv[:, 2] - uv[:, 0]) * (tw0, th0)
            ta = np.abs(f1[:, 0] * f2[:, 1] - f1[:, 1] * f2[:, 0]) / 2
            lvl = np.clip(0.5 * np.log2(np.maximum(ta, 1e-9) / np.maximum(sa, 1e-6)) + a.mip_bias, 0, len(tex) - 1)
            attr = np.concatenate([uv, np.repeat(lvl[:, None, None], 3, 1), uvw[..., 2:3]], -1)
        else:
            attr = uvw
        pix, hgt, att = rasterize(screen, attr, W, W)
        if not len(pix):
            continue
        ntri += len(tri)
        if tex is not None:
            li = np.round(att[:, 2]).astype(np.int64)
            c = np.zeros((len(pix), 4), np.float32)
            for L in np.unique(li):
                m = li == L
                c[m] = bilinear(tex[L], att[m, 0], att[m, 1])
                if blend is not None:
                    tb = blend[min(L, len(blend) - 1)]
                    wt = 1 - att[m, 3:4] if a.blend_invert else att[m, 3:4]
                    c[m, :3] = c[m, :3] * wt + bilinear(tb, att[m, 0], att[m, 1])[:, :3] * (1 - wt)
            if atest:
                ok = c[:, 3] >= 110
                pix, hgt, c = pix[ok], hgt[ok], c[ok]
            c = c[:, :3]
        else:
            c = np.broadcast_to(flat, (len(pix), 3))
        if not len(pix):
            continue
        p, idx = top(pix, hgt)
        h = hgt[idx]
        better = h > zbuf[p]
        zbuf[p[better]] = h[better]; col[p[better]] = c[idx][better]
        if terrain:
            gb = h > gbuf[p]
            gbuf[p[gb]] = h[gb]
        if not atest:
            sb = h > sbuf[p]
            sbuf[p[sb]] = h[sb]
    zbuf[~np.isfinite(zbuf)] = np.nan; gbuf[~np.isfinite(gbuf)] = np.nan; sbuf[~np.isfinite(sbuf)] = np.nan
    np.save(out / f'{a.key}-height.npy', zbuf.reshape(W, W).astype(np.float32))
    np.save(out / f'{a.key}-ground.npy', gbuf.reshape(W, W).astype(np.float32))
    np.save(out / f'{a.key}-solid.npy', sbuf.reshape(W, W).astype(np.float32))
    np.save(out / f'{a.key}-albedo.npy', col.reshape(W, W, 3).astype(np.uint8))
    print(json.dumps({'key': a.key, 'width': W, 'triangles': ntri, 'ceiling': ceiling,
                      'covered': float(np.isfinite(zbuf).mean())}))


if __name__ == '__main__':
    main()
