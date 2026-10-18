#!/usr/bin/env python3
"""Build the body-part doll SVGs from the Survarium 0.26g HUD critical indicator.

Source: flash_movies/hud.swf, sprite 119 (class SrvCriticalIndicator /
survarium.controls.CriticalIndicator). Parts (instance name -> sprite -> bitmap):
  head  -> 112 -> 110 (12x16)
  body  -> 109 -> 107 (22x36)
  lHand -> 115 -> 113 (18x35)   rHand = same bitmap mirrored
  lLeg  -> 106 -> 104 (16x49)   rLeg  = same bitmap mirrored
  outline (drawn on top) -> shape 118 -> bitmap 117 (63x107)
Positions below are the PlaceObject matrices of sprite 119 converted to the
63x107 frame of the outline bitmap (outline sits at sprite x=63, y=-24.65).

usage: build_doll.py <hud_images_dir> <out_dir> <potrace>
"""
import base64, json, os, re, subprocess, sys, tempfile
from PIL import Image

src, out, potrace = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(out, exist_ok=True)
W, H = 63, 107
K = 12  # supersampling factor for tracing

# name, bitmap id, x, y, mirrored  (top-left of the bitmap in outline coords)
PARTS = [
    ('head',  110, 25.0,  0.3,  False),
    ('body',  107, 19.95, 12.6, False),
    ('lHand', 113, 40.1,  15.65, False),
    ('rHand', 113, 4.35,  15.65, True),
    ('lLeg',  104, 30.0,  41.05, False),
    ('rLeg',  104, 16.25, 41.05, True),
]
# The doll faces the viewer: game "lHand"/"lLeg" (character's left) is on the viewer's right.
# Cut lines (outline px) used to split the six game parts into the damage-calculator zones.
# Zones follow the hit_param <-> bone mapping of base_character_full.skinned_model/hit_targets:
#   *_forearm = UpperArm+ForeArm (shoulder..wrist), *_arm = Hand; *_hip = UpLeg+Leg (hip..ankle),
#   *_foot = Foot; front/back = Spine+Chest(+Neck); brain = Head sphere, face = Head front box.
# Wrist/ankle = narrowest rows of the arm/leg bitmaps (y 43.6 / 83).
HEAD_CUT, WRIST_CUT, ANKLE_CUT = 6.0, 44.0, 84.0
ZONES = {  # zone id -> (game part, y_from, y_to)
    'brain':       ('head', -1, HEAD_CUT),
    'face':        ('head', HEAD_CUT, 99),
    'front':       ('body', -1, 999),
    'forearm_l':   ('lHand', -1, WRIST_CUT),   # whole arm
    'arm_l':       ('lHand', WRIST_CUT, 999),  # hand
    'forearm_r':   ('rHand', -1, WRIST_CUT),
    'arm_r':       ('rHand', WRIST_CUT, 999),
    'hip_l':       ('lLeg', -1, ANKLE_CUT),    # whole leg
    'foot_l':      ('lLeg', ANKLE_CUT, 999),   # foot
    'hip_r':       ('rLeg', -1, ANKLE_CUT),
    'foot_r':      ('rLeg', ANKLE_CUT, 999),
}
LABEL = {'brain': 'Head (brain)', 'face': 'Face', 'front': 'Torso (front)', 'back': 'Torso (back)',
         'arm_l': 'Left hand', 'forearm_l': 'Left arm', 'arm_r': 'Right hand',
         'forearm_r': 'Right arm', 'hip_l': 'Left leg', 'foot_l': 'Left foot',
         'hip_r': 'Right leg', 'foot_r': 'Right foot'}


def part_mask(bid, x, y, mirror):
    """Full-canvas supersampled alpha of one part bitmap."""
    bm = Image.open(os.path.join(src, f'{bid}.png')).convert('RGBA').getchannel('A')
    bw, bh = bm.size
    big = Image.new('L', (W * K, H * K), 0)
    # upscale smoothly first, then place
    up = bm.resize((bw * K, bh * K), Image.BICUBIC)
    if mirror:
        up = up.transpose(Image.FLIP_LEFT_RIGHT)
    big.paste(up, (round(x * K), round(y * K)))
    return big


def trace(mask, thr=110):
    """Threshold an L mask and trace with potrace; return (path d, transform)."""
    bw = mask.point(lambda v: 0 if v >= thr else 255).convert('1')
    with tempfile.TemporaryDirectory() as td:
        pbm = os.path.join(td, 'm.pbm'); svg = os.path.join(td, 'm.svg')
        bw.save(pbm)
        subprocess.run([potrace, '-s', '-a', '1.0', '-O', '0.4', '-t', '4', '-u', '10',
                        '-r', '72', '-o', svg, pbm], check=True)
        txt = open(svg).read()
    tr = re.search(r'<g transform="([^"]*)"', txt).group(1)
    ds = re.findall(r'<path d="([^"]*)"', txt, re.S)
    return ' '.join(' '.join(d.split()) for d in ds), tr


def zone_masks():
    masks = {}
    for name, bid, x, y, mir in PARTS:
        masks[name] = part_mask(bid, x, y, mir)
    zones = {}
    for zid, (part, y0, y1) in ZONES.items():
        m = masks[part].copy()
        a, b = max(0, round(y0 * K)), min(H * K, round(y1 * K))
        blank = Image.new('L', m.size, 0)
        blank.paste(m.crop((0, a, W * K, b)), (0, a))
        zones[zid] = blank
    return masks, zones


def outline_mask():
    im = Image.open(os.path.join(src, '117.png')).convert('RGBA')
    px = im.load()
    m = Image.new('L', (W, H), 0); mp = m.load()
    for yy in range(H):
        for xx in range(W):
            r, g, b, a = px[xx, yy]
            ring = (yy >= 66 and (xx < 13 or xx > 50)) or yy >= 90
            if not ring:
                mp[xx, yy] = a * r // 255
    return m.resize((W * K, H * K), Image.BICUBIC)


def svg_group(zid, d, tr, extra=''):
    return (f'  <path id="{zid}" class="zone" data-label="{LABEL[zid]}"{extra} '
            f'transform="scale({1 / K:.6f}) {tr}" d="{d}"/>\n')


def build(variant):
    _, zones = zone_masks()
    traced = {z: trace(m) for z, m in zones.items()}
    od, otr = trace(outline_mask(), 135)
    back = variant == 'back'
    head = ('<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<!-- Survarium 0.26g HUD body doll (hud.swf sprite 119 SrvCriticalIndicator), '
            f'{variant} view. Zones traced from the game part bitmaps; brain|face, forearm(arm)|arm(hand) and '
            f'hip(leg)|foot are split at y={HEAD_CUT}/{WRIST_CUT}/{ANKLE_CUT} (not separate in the game). -->\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="-1 -1 {W + 2} {H + 2}" '
            f'width="{(W + 2) * 4}" height="{(H + 2) * 4}">\n'
            '  <style>\n'
            '    .zone { fill: #ff5a5a; fill-opacity: .35; cursor: pointer; }\n'
            '    .zone:hover { fill-opacity: .6; }\n'
            '    .zone.selected { fill: #ff2020; fill-opacity: .85; }\n'
            '    #outline { fill: #e8e8e8; pointer-events: none; }\n'
            '    #ground { fill: none; stroke: #c8c8c8; stroke-width: .7; pointer-events: none; }\n'
            '  </style>\n'
            '  <defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">'
            '<stop offset=".25" stop-color="#fff" stop-opacity="0"/>'
            '<stop offset="1" stop-color="#fff" stop-opacity=".9"/></linearGradient>'
            '<mask id="gm"><rect x="-1" y="55" width="65" height="55" fill="url(#g)"/></mask></defs>\n'
            '  <ellipse id="ground" cx="31.5" cy="82" rx="30.8" ry="22.5" mask="url(#gm)"/>\n')
    flip = f' transform="translate({W} 0) scale(-1 1)"' if back else ''
    body = f'  <g id="zones"{flip}>\n'
    for zid, (d, tr) in traced.items():
        z = zid
        if back:
            z = {'front': 'back'}.get(z, z)
            # mirrored: character's left now on viewer's left; ids keep the character side
        extra = f' data-game-part="{ZONES[zid][0]}"'
        body += svg_group(z, d, tr, extra).replace('  <path', '    <path')
    body += '  </g>\n'
    body += (f'  <g{flip}><path id="outline" transform="scale({1 / K:.6f}) {otr}" d="{od}"/></g>\n')
    return head + body + '</svg>\n'


def build_original():
    """Faithful composition with the game's own bitmaps (embedded PNG)."""
    def b64(bid):
        return base64.b64encode(open(os.path.join(src, f'{bid}.png'), 'rb').read()).decode()
    parts = ''
    ids = {'head': 'head', 'body': 'body', 'lHand': 'arm_left', 'rHand': 'arm_right',
           'lLeg': 'leg_left', 'rLeg': 'leg_right'}
    for name, bid, x, y, mir in PARTS:
        bm = Image.open(os.path.join(src, f'{bid}.png')); bw, bh = bm.size
        t = f'translate({x + bw} {y}) scale(-1 1)' if mir else f'translate({x} {y})'
        parts += (f'  <image id="{ids[name]}" data-game-name="{name}" width="{bw}" height="{bh}" '
                  f'transform="{t}" opacity=".5" href="data:image/png;base64,{b64(bid)}"/>\n')
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            '<!-- Survarium 0.26g hud.swf sprite 119 (SrvCriticalIndicator): original bitmaps 104/107/110/113 '
            '(red part masks, game tints them by HP at alpha .5) and 117 (outline), placed as in the swf. -->\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W * 4}" height="{H * 4}" '
            'style="image-rendering:auto">\n' + parts +
            f'  <image id="outline" width="{W}" height="{H}" href="data:image/png;base64,{b64(117)}"/>\n'
            '</svg>\n')


front = build('front')
open(os.path.join(out, 'doll_front.svg'), 'w').write(front)
open(os.path.join(out, 'doll_back.svg'), 'w').write(build('back'))
# flat-colour zone map (for colour-picking hit tests on the PNG fallback)
COLORS = {'brain': '#ff0000', 'face': '#ff8000', 'front': '#00c000', 'arm_l': '#0040ff',
          'forearm_l': '#00a0ff', 'arm_r': '#a000ff', 'forearm_r': '#ff00ff', 'hip_l': '#ffff00',
          'foot_l': '#808000', 'hip_r': '#00ffff', 'foot_r': '#008080'}
css = ' '.join(f'#{z}{{fill:{c}}}' for z, c in COLORS.items())
zm = re.sub(r'<style>.*?</style>', '<style>.zone{fill-opacity:1;shape-rendering:crispEdges} '
            + css + ' #outline,#ground{display:none}</style>', front, flags=re.S)
open(os.path.join(out, 'doll_zonemap.svg'), 'w').write(zm)
json.dump({'source': 'flash_movies/hud.swf sprite 119 SrvCriticalIndicator (survarium.controls.CriticalIndicator)',
           'viewBox': [-1, -1, W + 2, H + 2],
           'game_parts': {'head': 'head', 'body': 'torso', 'lHand': 'left arm (whole)',
                          'rHand': 'right arm (whole)', 'lLeg': 'left leg (whole)', 'rLeg': 'right leg (whole)'},
           'cuts_y': {'brain|face': HEAD_CUT, 'forearm(whole arm)|arm(hand) at wrist': WRIST_CUT,
                      'hip(whole leg)|foot at ankle': ANKLE_CUT},
           'bones': {'front': 'Spine+Chest+Neck', 'back': 'Chest+Spine', 'brain': 'Head (sphere)',
                     'face': 'Head (front box)', 'forearm_*': 'Arm+ForeArm (shoulder to wrist)',
                     'arm_*': 'Hand', 'hip_*': 'UpLeg+Leg (hip to ankle)', 'foot_*': 'Foot',
                     'source': 'models/character/human/base/base_character_full.skinned_model/hit_targets'},
           'zones': {z: {'label': LABEL[z], 'game_part': ZONES[z][0], 'zonemap_color': COLORS[z]}
                     for z in ZONES},
           'back_view': 'doll_back.svg: mirrored, torso id "back", sides keep the character\'s left/right'},
          open(os.path.join(out, 'doll.json'), 'w'), indent=1)
open(os.path.join(out, 'doll_hud_original.svg'), 'w').write(build_original())
print('ok')
