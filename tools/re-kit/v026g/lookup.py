"""Look up v0.26g game/engine labels by name or address, with their ABI observations."""
import argparse
import csv
import json
from collections import defaultdict
from pathlib import Path

DIRECTORY = Path(__file__).resolve().parent


def address(text):
    try:
        value = int(text, 0)
    except ValueError as exc:
        raise argparse.ArgumentTypeError('Expected a 32-bit address') from exc
    if not 0 <= value <= 0xffffffff:
        raise argparse.ArgumentTypeError('Expected a 32-bit address')
    return value


def table(name):
    with open(DIRECTORY / name, newline='') as f:
        return list(csv.DictReader(f, delimiter='\t', quoting=csv.QUOTE_NONE))


def file_offset(target, rva):
    for section in target['sections']:
        start = int(section['rva'], 16)
        if start <= rva < start + section['raw_size']:
            return section['raw_offset'] + rva - start
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('query', help='Case-insensitive name fragment, or a VA such as 0x00379930')
    parser.add_argument('--exact', action='store_true', help='Match the whole name')
    parser.add_argument('--reviewed', action='store_true', help='Leave out inferred.tsv')
    parser.add_argument('--runtime-base', type=address)
    args = parser.parse_args()
    target = json.loads((DIRECTORY / 'identity.json').read_text())['target']
    base = int(target['preferred_image_base'], 16)

    labels = table('functions.tsv') + ([] if args.reviewed else table('inferred.tsv'))
    query = args.query.casefold()
    try:
        va = address(args.query) if query.startswith('0x') else None
    except argparse.ArgumentTypeError:
        va = None
    if va is not None:
        hits = [r for r in labels if int(r['va'], 16) == va]
    else:
        hits = [r for r in labels
                if (r['name'].casefold() == query if args.exact else query in r['name'].casefold())]
    if not hits:
        parser.error('No label matches this query')

    contracts, inputs = defaultdict(list), defaultdict(list)
    for row in table('abi.tsv'):
        contracts[row['va'], row['name']].append(row)
    for row in table('abi_inputs.tsv'):
        inputs[row['va'], row['name']].append(row)

    by_va = defaultdict(list)
    for row in hits:
        by_va[row['va']].append(row)
    for va_text, rows in sorted(by_va.items()):
        rva = int(va_text, 16) - base
        offset = file_offset(target, rva)
        line = f'{va_text}  rva 0x{rva:08x}'
        if offset is not None:
            line += f'  file 0x{offset:08x}'
        if args.runtime_base is not None:
            if args.runtime_base + rva > 0xffffffff:
                parser.error('Module base plus RVA exceeds the 32-bit address space')
            line += f'  runtime 0x{args.runtime_base + rva:08x}'
        print(line)
        # Observations without a name (automated scans) belong to the whole body.
        for name in [r['name'] for r in rows] + ['']:
            if name:
                row = next(r for r in rows if r['name'] == name)
                ref = f"  (0.21d {row['ref_021d']})" if row['ref_021d'] else ''
                print(f"  {row['status']:<16} {name}{ref}")
            for c in contracts[va_text, name]:
                result = f"  result {c['result']}" if c['result'] else ''
                ret = c['ret_cleanup'] or '?'
                print(f"    {c['source']} abi: ret {ret}{result}")
            for i in inputs[va_text, name]:
                role = f"{i['role']} ({i['basis']})" if i['role'] else i['basis']
                print(f"      {i['location']:<14} {role}")


if __name__ == '__main__':
    main()
