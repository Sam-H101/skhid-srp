"""Cut the lobby modifiers atlas (lobby_menu.swf character 3279) exactly as IconContainer does.

IconContainer("modifiers", 38, 38, bmp, vertical=true, count=0, stepH=0, stepV=0, atStart=false)
  columns = floor(W/38), rows = floor(H/38); icons pushed column-major:
  index = c*rows + r  ->  rect (38*c, 38*r, 38, 38)
usage: cut_mods.py <atlas.png> <modifications.options.json> <out_dir>
"""
import json, os, sys
from PIL import Image

atlas, cfg, out = sys.argv[1:4]
os.makedirs(out, exist_ok=True)
im = Image.open(atlas).convert('RGBA')
W, H = im.size
S = 38
cols, rows = W // S, H // S
d = json.load(open(cfg))
users = {}
for group in ('modifications', 'defects'):
    for m in d[group]:
        ui = m.get('ui_desc', {})
        users.setdefault(ui.get('icon'), []).append({'group': group, 'id': m.get('id'), 'name': ui.get('name'),
                                                    'props': ui.get('props_list')})
icons = []
for idx in range(cols * rows):
    c, r = divmod(idx, rows)
    cell = im.crop((S * c, S * r, S * c + S, S * r + S))
    empty = cell.getchannel('A').getbbox() is None
    fn = f'mod_icon_{idx}.png'
    if not empty:
        cell.save(os.path.join(out, fn))
    icons.append({'index': idx, 'col': c, 'row': r, 'rect': [S * c, S * r, S, S],
                  'file': None if empty else fn, 'empty': empty, 'used_by': users.get(idx, [])})
# contact sheet: index order, 10 per row, 2x
sheet_cols = 13
n = len(icons)
sh = Image.new('RGBA', (sheet_cols * 48, ((n + sheet_cols - 1) // sheet_cols) * 48), (30, 32, 30, 255))
for ic in icons:
    if ic['file']:
        x, y = (ic['index'] % sheet_cols) * 48 + 5, (ic['index'] // sheet_cols) * 48 + 5
        sh.alpha_composite(Image.open(os.path.join(out, ic['file'])), (x, y))
sh = sh.resize((sh.width * 2, sh.height * 2), Image.NEAREST)
sh.save(os.path.join(out, '_contact_sheet_2x.png'))
missing = sorted(k for k in users if k is not None and (k >= n or icons[k]['empty']))
json.dump({'atlas': {'swf_character': 3279, 'class': 'LobbyMenu_ModifierIcons', 'size': [W, H],
                     'cell': S, 'columns': cols, 'rows': rows,
                     'rule': 'index = col*rows + row (column-major); rect = (38*col, 38*row, 38, 38)'},
           'used_indices': sorted(k for k in users if k is not None),
           'used_but_empty_or_out_of_range': missing,
           'icons': icons}, open(os.path.join(out, 'mod_icons.json'), 'w'), indent=1)
print(W, H, cols, rows, 'non-empty', sum(not i['empty'] for i in icons), 'used', sorted(users), 'missing', missing)
