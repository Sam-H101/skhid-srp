#!/usr/bin/env python3
"""Measure one building three ways: model geometry (world metres), the render
(pixels * metres_per_pixel) and the native HUD bitmap (pixels * its m/px).

    measure_building.py KEY SIZE POSX POSY NATIVE_PNG X Z
(X, Z) is a world point on the building's roof."""
import json, sys, struct
from pathlib import Path
import numpy as np
from PIL import Image
from scipy import ndimage
sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_topdown as R, bincfg

key = sys.argv[1]; size, posx, posy = map(float, sys.argv[2:5]); native = sys.argv[5]; X, Z = map(float, sys.argv[6:8])
h = np.load(f'out/{key}-height.npy').astype(float); W = h.shape[0]; mpp = 2 * size / W
u, v = int((X - posx + size) / mpp), int((posy - Z + size) / mpp)
roof = h[v, u]
# render: connected region of pixels within 0.6 m of the roof height around (u, v)
m = np.abs(h - roof) < 0.6
lab, _ = ndimage.label(m); comp = lab == lab[v, u]
ys, xs = np.nonzero(comp)
render = {'x_extent_m': round((xs.max() - xs.min() + 1) * mpp, 2), 'z_extent_m': round((ys.max() - ys.min() + 1) * mpp, 2)}
# native: dark connected component at the same world point
nat = np.asarray(Image.open(native).convert('RGBA'), np.float32); NW = nat.shape[0]; nmpp = 2 * size / NW
lum = nat[..., :3] @ np.array([0.299, 0.587, 0.114]); dark = (lum < 105) & (nat[..., 3] > 120)
nu, nv = int((X - posx + size) / nmpp), int((posy - Z + size) / nmpp)
nl, _ = ndimage.label(dark); nc = nl == nl[nv, nu]
nys, nxs = np.nonzero(nc)
nativem = {'x_extent_m': round((nxs.max() - nxs.min() + 1) * nmpp, 2), 'z_extent_m': round((nys.max() - nys.min() + 1) * nmpp, 2),
           'label_nonzero': bool(nl[nv, nu])}
# geometry: render-visual vertices at roof height (+-0.6 m) inside the render component's footprint
proj = json.loads(Path(f'lv/{key}/project.json').read_text()); pts = []
x0, x1 = posx - size + xs.min() * mpp - 1, posx - size + (xs.max() + 1) * mpp + 1
z1, z0 = posy + size - ys.min() * mpp + 1, posy + size - (ys.max() + 1) * mpp - 1
for o in proj['render_visuals']:
    model = Path(f'lv/{key}/models') / (o['lib_name'] + '.model')
    if not (model / 'settings').exists(): continue
    s = bincfg.load(model / 'settings')
    for surf in s.get('lod_hierrarchy', {}).get('LOD0', {}).get('surfaces', []):
        f = model / 'render' / surf / 'converted_model'
        if not f.exists(): continue
        c = R.chunks(f.read_bytes()); n = struct.unpack_from('<I', c[3])[0]
        if not n: continue
        st = (len(c[3]) - 4) // n
        P = R.place(np.frombuffer(c[3], np.uint8, offset=4, count=n * st).reshape(n, st)[:, :12].copy().view('<f4').reshape(-1, 3).astype(float), o)
        sel = (P[:, 0] > x0) & (P[:, 0] < x1) & (P[:, 2] > z0) & (P[:, 2] < z1) & (np.abs(P[:, 1] - roof) < 0.6)
        if sel.any(): pts.append(P[sel])
P = np.concatenate(pts)
geom = {'x_extent_m': round(P[:, 0].max() - P[:, 0].min(), 2), 'z_extent_m': round(P[:, 2].max() - P[:, 2].min(), 2)}
print(json.dumps({'key': key, 'point': [X, Z], 'roof_y': round(float(roof), 2), 'geometry': geom, 'render': render,
                  'native_minimap': nativem, 'render_m_per_px': mpp, 'native_m_per_px': nmpp}))
