"""Approximate the lobby item hint's modification block (InventoryItemHint) with PIL,
using the values read from lobby_menu.swf (see README). Not a pixel-exact Flash render.
usage: mock_mods.py <ui2 dir>
"""
import os, sys
from PIL import Image, ImageDraw, ImageFilter, ImageFont

root = sys.argv[1]
S = 2  # output scale
FONT = ImageFont.truetype(os.path.join(root, 'style/fonts/CalibriBold_swf133.ttf'), 14 * S)
LABEL, GREEN, RED, SEP_TXT = (0xbc, 0xcb, 0xcb), (0x87, 0xa8, 0x16), (0x96, 0x0f, 0x0e), (0xbc, 0xcb, 0xcb)
W = 250

def icon(idx, tint=None):
    im = Image.open(os.path.join(root, 'mod_icons', f'mod_icon_{idx}.png')).convert('RGBA')
    if tint:
        a = im.getchannel('A'); im = Image.new('RGBA', im.size, tint + (255,)); im.putalpha(a)
    n = round(38 * 0.8065 * S)
    return im.resize((n, n), Image.BILINEAR)

def text_layer(size, xy, parts):
    """parts: [(text, colour)] on one line; returns RGBA layer with Flash-like drop shadow
    (black, blur 4, distance 1, angle 90deg, strength 1)."""
    fg = Image.new('RGBA', size, (0, 0, 0, 0)); d = ImageDraw.Draw(fg); x, y = xy
    for t, c in parts:
        d.text((x, y), t, font=FONT, fill=c + (255,)); x += d.textlength(t, font=FONT)
    sh = Image.new('RGBA', size, (0, 0, 0, 0)); sh.putalpha(fg.getchannel('A'))
    sh = sh.filter(ImageFilter.GaussianBlur(2 * S / 2)).transform(size, Image.AFFINE, (1, 0, 0, 0, 1, -1 * S))
    out = Image.alpha_composite(sh, fg); return out

rows_default = []
rows_mods = [(32, 'Damage', '+5%', GREEN), (34, 'Dispersion', '-10%', GREEN), (40, 'Weapon recoil', '-6%', GREEN)]
rows_crit = [(33, 'Distance', '-10%', RED), (36, 'Reload speed', '-10%', RED)]
H = 20 + 25 * len(rows_mods) + 15 + 26 + 25 * len(rows_crit) + 15
img = Image.new('RGBA', (W * S, H * S), (24, 26, 26, 255))
# bottomBackContainer: #000 alpha 89/255
img.alpha_composite(Image.new('RGBA', ((W - 10) * S, H * S), (0, 0, 0, 89)), (5 * S, 0))
y = 10
def row(y, idx, label, value, col, tint=None, x=10):
    global img
    img.alpha_composite(icon(idx, tint), (x * S, (y - 1) * S))
    img = Image.alpha_composite(img, text_layer(img.size, ((x + 40) * S, (y + 4) * S - 3 * S), [(label + ': ', LABEL), (value, col)]))
for r in rows_mods:
    row(y, *r); y += 25
# critical separator: lines 54 px at x=5 and x=190.95 (bitmap 41: #ececec a27 over #000 a103), centred caption
y += 15
d = ImageDraw.Draw(img)
for x0 in (5, 190.95):
    d.rectangle([x0 * S, y * S, (x0 + 54) * S - 1, y * S + S - 1], fill=(0xec, 0xec, 0xec, 27))
    d.rectangle([x0 * S, (y + 1) * S, (x0 + 54) * S - 1, (y + 2) * S - 1], fill=(0, 0, 0, 103))
cap = 'Critical damages'; tw = d.textlength(cap, font=FONT)
img = Image.alpha_composite(img, text_layer(img.size, (int(5 * S + (240 * S - tw) / 2), (y - 10) * S - 2 * S), [(cap, SEP_TXT)]))
y += 26 - 10 + 5
for r in rows_crit:
    row(y, *r, tint=RED); y += 25
img.save(os.path.join(root, 'style', 'mock_modifiers_2x.png'))
print(img.size)
