"""Add labels from the function-map exports to the v0.26g kit TSVs.

--inferred takes inferred.json-style exports (inferred.json and the
target-*-inferred.json files); their medium/high rows go to inferred.tsv.
--observed takes target-observed.json; accepted names go to functions.tsv
with their ABI observations. A name already listed at an address is kept
as it is, except that inferred-high replaces inferred-medium.
"""
import argparse
import json
from pathlib import Path

RANK = {'low': 0, 'medium': 1, 'high': 2}
LABELS = ('va', 'status', 'ref_021d', 'name')
HEADERS = {
    'functions.tsv': LABELS,
    'inferred.tsv': LABELS,
    'abi.tsv': ('va', 'name', 'source', 'ret_cleanup', 'result'),
    'abi_inputs.tsv': ('va', 'name', 'location', 'role', 'basis'),
}


def hexva(value):
    return '' if value is None else f'0x{value:08x}' if isinstance(value, int) else value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kit', type=Path, required=True, help='tools/re-kit/v026g')
    parser.add_argument('--inferred', type=Path, nargs='*', default=[])
    parser.add_argument('--observed', type=Path, nargs='*', default=[])
    parser.add_argument('--min-confidence', choices=RANK, default='medium')
    args = parser.parse_args()

    tables = {name: [tuple(line.split('\t')) for line in
                     (args.kit / name).read_text().splitlines()[1:]] for name in HEADERS}
    reviewed = {(r[0], r[3]) for r in tables['functions.tsv']}
    inferred = {(r[0], r[3]): r for r in tables['inferred.tsv']}
    added = 0

    for path in args.observed:
        for row in json.loads(path.read_text())['entries']:
            key = (row['preferred_va'], row['name'])
            if row['status'] != 'accepted_target_observed_name' or key in reviewed:
                continue
            reviewed.add(key)
            inferred.pop(key, None)
            tables['functions.tsv'].append((key[0], 'observed', '', key[1]))
            abi = row['abi']
            tables['abi.tsv'].append(
                (*key, 'manual', ','.join(map(str, abi['local_ret_cleanup_bytes'])), ''))
            for i in abi['inputs']:
                tables['abi_inputs.tsv'].append(
                    (*key, i['entry_location'], i['proposed_semantic_role'], 'observed'))
            added += 1

    for path in args.inferred:
        for row in json.loads(path.read_text())['entries']:
            if RANK[row['confidence']] < RANK[args.min_confidence] or row.get('area') == 'vendor':
                continue
            status = f"inferred-{row['confidence']}"
            for name in row['names']:
                key = (row['preferred_va'], name)
                if key in reviewed:
                    continue
                if key in inferred and RANK[inferred[key][1][9:]] >= RANK[row['confidence']]:
                    continue
                added += key not in inferred
                inferred[key] = (key[0], status, hexva(row.get('reference_va')), name)
    tables['inferred.tsv'] = list(inferred.values())

    for name, header in HEADERS.items():
        rows = tables[name]
        # Label files sort by address and name; ABI rows keep their order within an address.
        rows = sorted(rows) if header == LABELS else sorted(rows, key=lambda r: r[0])
        (args.kit / name).write_text('\n'.join('\t'.join(r) for r in [header, *rows]) + '\n')
    print(f'added {added} labels')


if __name__ == '__main__':
    main()
