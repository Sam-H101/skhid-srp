"""Generate crates/survarium/src/v021d/item_params.rs from the 0.21d item
configs (dumped to text by bc8.py into prog/itxt) and the dictionary's
cfg_name list (scope021d/txt/gameplay__db_static_dictionaries.txt)."""
import os
import re
import sys

itxt, dicttxt, out = sys.argv[1], sys.argv[2], sys.argv[3]

# dict_id -> cfg_name
d = open(dicttxt, encoding='utf8').read()
entries = re.findall(r'cfg_name: "([^"]+)"\n(?:\s+\w+: [^\n]+\n)*?\s+dict_id: (\d+)', d)
by_id = {}
for block in re.split(r'\n    \d+_\[\d+\]\n', d.split('items_dict[')[1].split('\n  ')[0] if False else d):
    pass
# parse items_dict blocks robustly
sect = d[d.index('items_dict['):]
blocks = re.split(r'\n    \d+_\[\d+\]', sect)[1:]
for b in blocks:
    cfg = re.search(r'cfg_name: "([^"]+)"', b)
    did = re.search(r'dict_id: (\d+)', b)
    if cfg and did:
        by_id[int(did.group(1))] = cfg.group(1)

rows = []
for dict_id in sorted(by_id):
    cfg = by_id[dict_id]
    path = os.path.join(itxt, cfg.replace('/', '__') + '.txt')
    if not os.path.exists(path):
        print('missing', dict_id, cfg, file=sys.stderr)
        continue
    t = open(path, encoding='utf8').read()

    def g(k, default=0):
        m = re.search(r'^\s*' + k + r': (.+)$', t, re.M)
        return m.group(1) if m else default

    rows.append((dict_id, int(g('faction')), int(g('faction_affinity')),
                 int(float(g('drop_value'))), int(g('drop_weight')),
                 int(g('clip_size')), int(g('clips_in_bag')), cfg))

with open(out, 'w', encoding='utf8', newline='\n') as f:
    f.write('''//! What srp needs from each 0.21d item's own config
//! (`gameplay/items/**.options` in 0.21d's `resources.db`): its faction and
//! faction affinity (for the gear faction, `get_profile_faction_affinity`),
//! its drop value and weight (`drops.options` rolls), and for ammo the pack a
//! drop gives. Generated from the game's files; an item without a field has 0.

/// One item's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemParams {
    pub dict_id: u16,
    /// 0 none, 1..=4 the lobby factions.
    pub faction: u8,
    pub faction_affinity: u32,
    pub drop_value: u32,
    pub drop_weight: u32,
    /// Rounds a clip of this ammo holds, and clips a pack has.
    pub clip_size: u32,
    pub clips_in_bag: u32,
}

/// By `dict_id`.
#[rustfmt::skip]
pub const ITEM_PARAMS: &[ItemParams] = &[
''')
    for r in rows:
        f.write(f'    ItemParams {{ dict_id: {r[0]}, faction: {r[1]}, faction_affinity: {r[2]}, drop_value: {r[3]}, drop_weight: {r[4]}, clip_size: {r[5]}, clips_in_bag: {r[6]} }}, // {r[7].split("/")[-1].replace(".options", "")}\n')
    f.write('''];

/// The parameters of `dict_id`, if it is an item.
pub fn item_params(dict_id: u16) -> Option<&'static ItemParams> {
    ITEM_PARAMS.iter().find(|item| item.dict_id == dict_id)
}
''')
print(len(rows), 'items')
