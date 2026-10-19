"""Recompose the Survarium 0.26g HUD critical indicator (hud.swf sprite 119) exactly.

usage: build_doll2.py <ffdec work dir (ui2/work/hud)> <out dir (ui2/doll)>

All coordinates are sprite-119 pixels (twips/20), taken from the PlaceObject2/3 tags of
sprite 119 and the bitmap-fill matrices of shapes 105/108/111/114/118 (see README).
"""
import base64, io, json, os, shutil, sys
from PIL import Image

work, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
os.makedirs(os.path.join(out, 'art'), exist_ok=True)

# frame of sprite 119 (union of all children): x 0..140, y -24.65..98.7
FX, FY, FW, FH = 0.0, -24.65, 140, 124

BITMAPS = {  # file -> (swf bitmap id, size)
    'leg.png': 104, 'body.png': 107, 'head.png': 110, 'arm.png': 113, 'outline.png': 117,
}
for f, bid in BITMAPS.items():
    shutil.copy(os.path.join(work, 'img', f'{bid}.png'), os.path.join(out, 'art', f))
shutil.copy(os.path.join(work, 'shape', '97.png'), os.path.join(out, 'art', 'icon_bleed.png'))
shutil.copy(os.path.join(work, 'shape', '100.png'), os.path.join(out, 'art', 'icon_rad_tox.png'))
shutil.copy(os.path.join(work, 'shape4', '97.png'), os.path.join(out, 'art', 'icon_bleed@4x.png'))
shutil.copy(os.path.join(work, 'shape4', '100.png'), os.path.join(out, 'art', 'icon_rad_tox@4x.png'))
shutil.copy(os.path.join(work, 'shapesvg', '97.svg'), os.path.join(out, 'art', 'icon_bleed.svg'))
shutil.copy(os.path.join(work, 'shapesvg', '100.svg'), os.path.join(out, 'art', 'icon_rad_tox.svg'))
shutil.copy(os.path.join(work, 'sprite', 'DefineSprite_119_SrvCriticalIndicator_SrvCriticalIndicator', '1.png'),
            os.path.join(out, 'ffdec_sprite119_authoring_1x.png'))
shutil.copy(os.path.join(work, 'sprite4', 'DefineSprite_119_SrvCriticalIndicator_SrvCriticalIndicator', '1.png'),
            os.path.join(out, 'ffdec_sprite119_authoring_4x.png'))

# Elements in depth order. sprite = PlaceObject matrix (sx = -1 for mirrored), off = bitmap
# fill matrix translate inside the shape (bitmap top-left in sprite-local coords).
ELEMS = [
    # name,   depth, part_id, file,         sprite tx, ty,   sx,  bitmap off x, y,  w,  h
    ('lLeg',   8, 3, 'leg.png',     97.0,   35.65,  1, -4.0,   -19.25, 16, 49),
    ('rLeg',  10, 4, 'leg.png',     91.25,  35.65, -1, -4.0,   -19.25, 16, 49),
    ('body',  12, 6, 'body.png',    74.95,  -2.05,  1,  8.0,   -10.0,  22, 36),
    ('head',  14, 5, 'head.png',    74.0,  -14.35,  1, 14.0,   -10.0,  12, 16),
    ('rHand', 16, 2, 'arm.png',     75.2,    5.0,  -1, -10.15, -14.0,  18, 35),
    ('lHand', 18, 1, 'arm.png',    113.25,   5.0,   1, -10.15, -14.0,  18, 35),
    ('outline', 21, None, 'outline.png', 0.0, 0.0,  1, 63.0,   -24.65, 63, 107),
]
ICONS = [  # name, depth, file, tx, ty, shape bounds (xmin, ymin, w, h)
    ('bleedInd', 2, 'icon_bleed.png', 16.0, 69.0, (-5.0, -7.5, 10, 15)),
    ('toxInd', 4, 'icon_rad_tox.png', 40.0, 69.0, (-10.0, -9.5, 20, 19)),
    ('radInd', 6, 'icon_rad_tox.png', 16.0, 47.45, (-10.0, -9.5, 20, 19)),
]


def bbox(e):
    name, d, pid, f, tx, ty, sx, ox, oy, w, h = e
    if sx == 1:
        x0 = tx + ox
    else:
        x0 = tx - (ox + w)
    return x0, ty + oy, w, h


placement = {'frame': {'x': FX, 'y': FY, 'w': FW, 'h': FH,
                       'note': 'sprite-119 px; canvas pixel = (x - frame.x, y - frame.y)'},
             'parts': [], 'icons': []}
for e in ELEMS:
    name, d, pid, f, tx, ty, sx, ox, oy, w, h = e
    x0, y0, _, _ = bbox(e)
    placement['parts'].append({
        'name': name, 'depth': d, 'part_id': pid, 'file': 'art/' + f, 'w': w, 'h': h,
        'mirrored': sx == -1,
        # full affine of the bitmap in sprite-119 px: X = a*u + tx, Y = v + ty (u,v bitmap px)
        'matrix': [sx, 0, 0, 1, round(tx + sx * ox, 4), round(ty + oy, 4)],
        'bbox_sprite': [round(x0, 4), round(y0, 4), w, h],
        'bbox_canvas': [round(x0 - FX, 4), round(y0 - FY, 4), w, h],
        'initial_colorTransform': 'alphaMultiplier 0.5 (128/256), rgb x1' if pid else 'none',
    })
for name, d, f, tx, ty, (bx, by, w, h) in ICONS:
    placement['icons'].append({'name': name, 'depth': d, 'file': 'art/' + f, 'w': w, 'h': h,
                               'origin_sprite': [tx, ty],
                               'bbox_canvas': [round(tx + bx - FX, 4), round(ty + by - FY, 4), w, h],
                               'note': 'white vector; hidden at start, exe never shows it'})
json.dump(placement, open(os.path.join(out, 'placement.json'), 'w'), indent=1)


def tint(value, bp):
    """CriticalIndicator.partHP: (rgb, alphaMultiplier)."""
    g = min(value / bp * 180.0, 180.0)
    g = int(g) & 0xff  # getColor(255, green:uint, 0): uint() truncates
    a = 0.5 if value < bp else 0.5 * ((1 - value) / (1 - bp))
    return (255, g, 0), a


def load(f):
    return Image.open(os.path.join(out, 'art', f)).convert('RGBA')


def compose(state, scale, smooth, show_icons=(), bg=None):
    """state: part name -> (value, break_point); parts not listed keep the authoring look
    (red bitmap, alpha 0.5); pass value 1 to hide a part like the game does."""
    W, H = round(FW * scale), round(FH * scale)
    canvas = Image.new('RGBA', (W, H), bg or (0, 0, 0, 0))
    layers = []
    for name, d, f, tx, ty, (bx, by, w, h) in ICONS:
        if name in show_icons:
            layers.append((d, f, tx + bx, ty + by, w, h, False, None))
    for e in ELEMS:
        name, d, pid, f, tx, ty, sx, ox, oy, w, h = e
        x0, y0, _, _ = bbox(e)
        ct = None
        if pid is not None:
            if name in state:
                ct = tint(*state[name])
            else:
                ct = ((None), 0.5)
        layers.append((d, f, x0, y0, w, h, sx == -1, ct))
    for d, f, x0, y0, w, h, mir, ct in sorted(layers, key=lambda l: l[0]):
        im = load(f)
        if mir:
            im = im.transpose(Image.FLIP_LEFT_RIGHT)
        if ct is not None:
            rgb, am = ct
            r, g, b, a = im.split()
            a = a.point(lambda v: round(v * am))
            if rgb is not None:
                im = Image.merge('RGBA', (Image.new('L', im.size, rgb[0]), Image.new('L', im.size, rgb[1]),
                                          Image.new('L', im.size, rgb[2]), a))
            else:
                im = Image.merge('RGBA', (r, g, b, a))
        im = im.resize((round(w * scale), round(h * scale)),
                       Image.BILINEAR if smooth else Image.NEAREST)
        layer = Image.new('RGBA', (W, H), (0, 0, 0, 0))
        layer.paste(im, (round((x0 - FX) * scale), round((y0 - FY) * scale)))
        canvas = Image.alpha_composite(canvas, layer)
    return canvas


def render_set(tag, state, icons=()):
    # 1x: exact area coverage of the fractional positions (compose at 20x = twips, box down)
    big = compose(state, 20, False, icons)
    one = big.resize((FW, FH), Image.BOX)
    one.save(os.path.join(out, f'{tag}_1x.png'))
    one.resize((FW * 4, FH * 4), Image.NEAREST).save(os.path.join(out, f'{tag}_4x_nearest.png'))
    compose(state, 4, True, icons).save(os.path.join(out, f'{tag}_4x_smooth.png'))
    # same on a dark scene-like background, for eyeballing
    for sfx in ('1x', '4x_nearest', '4x_smooth'):
        im = Image.open(os.path.join(out, f'{tag}_{sfx}.png'))
        bgim = Image.new('RGBA', im.size, (38, 40, 36, 255))
        Image.alpha_composite(bgim, im).save(os.path.join(out, f'{tag}_{sfx}_on_dark.png'))


ALL1 = {n: (1.0, 0.5) for n in ('head', 'body', 'lHand', 'rHand', 'lLeg', 'rLeg')}
BP = {'head': 0.5, 'body': 0.5, 'lHand': 0.2, 'rHand': 0.2, 'lLeg': 0.2, 'rLeg': 0.2}
# reference: body at 40 %, left leg at 0 %, everything else full (alpha 0 -> invisible)
hit = {n: (1.0, BP[n]) for n in BP}
hit['body'] = (0.4, BP['body'])
hit['lLeg'] = (0.0, BP['lLeg'])
render_set('ref_hit_body40_lleg0', hit)
# a gradient sample: every part at a different value (break points from the exe = regen threshold)
grad = {'head': (0.75, 0.5), 'body': (0.6, 0.5), 'lHand': (0.5, 0.2), 'rHand': (0.15, 0.2),
        'lLeg': (0.05, 0.2), 'rLeg': (0.9, 0.2)}
render_set('ref_mixed', grad)
# authoring state (what the swf shows before any partHP call; the game hides the doll then)
render_set('ref_authoring', {}, icons=('bleedInd', 'toxInd', 'radInd'))

# SVG recomposition: each part is a flood rect masked by the part bitmap's alpha.
def b64(f):
    return base64.b64encode(open(os.path.join(out, 'art', f), 'rb').read()).decode()

defs, body = [], []
for f in ('leg.png', 'body.png', 'head.png', 'arm.png'):
    defs.append(f'<image id="bm-{f[:-4]}" width="{Image.open(os.path.join(out,"art",f)).size[0]}" '
                f'height="{Image.open(os.path.join(out,"art",f)).size[1]}" href="data:image/png;base64,{b64(f)}"/>')
for e in ELEMS[:-1]:
    name, d, pid, f, tx, ty, sx, ox, oy, w, h = e
    m = f'matrix({sx} 0 0 1 {tx + sx * ox:g} {ty + oy:g})'
    defs.append(f'<mask id="m-{name}" mask-type="alpha" maskUnits="userSpaceOnUse" x="0" y="-30" width="140" height="130">'
                f'<use href="#bm-{f[:-4]}" transform="{m}"/></mask>')
    rgb, a = tint(*hit[name])
    body.append(f'<rect id="{name}" data-part-id="{pid}" mask="url(#m-{name})" x="0" y="-30" width="140" height="130" '
                f'fill="rgb({rgb[0]},{rgb[1]},{rgb[2]})" fill-opacity="{a:g}"/>')
body.append(f'<image id="outline" width="63" height="107" x="63" y="-24.65" href="data:image/png;base64,{b64("outline.png")}"/>')
svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{FX} {FY} {FW} {FH}" width="{FW*4}" height="{FH*4}">\n'
       '<!-- Survarium 0.26g hud.swf sprite 119 recomposed. Set fill/fill-opacity of each part rect with partHP(). '
       'State shown: body 0.4/bp0.5, lLeg 0/bp0.2, rest 1.0 -->\n'
       '<defs>\n' + '\n'.join(defs) + '\n</defs>\n' + '\n'.join(body) + '\n</svg>\n')
open(os.path.join(out, 'doll_recompose.svg'), 'w').write(svg)
print('ok')
