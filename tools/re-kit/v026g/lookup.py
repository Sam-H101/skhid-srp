"""Look up v0.26g game/engine labels and condensed target ABI observations."""
import argparse
import json
from pathlib import Path


def address(text):
    try:
        value = int(text, 0)
    except ValueError as exc:
        raise argparse.ArgumentTypeError('Expected a 32-bit module base') from exc
    if not 0 <= value <= 0xffffffff:
        raise argparse.ArgumentTypeError('Expected a 32-bit module base')
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('name', help='Case-insensitive qualified name or fragment')
    parser.add_argument('--exact', action='store_true')
    parser.add_argument('--runtime-base', type=address)
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    def read(name):
        return json.loads((directory / name).read_text())
    identity = read('identity.json')
    names, info, abi = read('functions.json'), read('functions.info.json'), read('abi.json')
    matches = []
    query = args.name.casefold()
    for name, vas in names.items():
        if not (name.casefold() == query if args.exact else query in name.casefold()):
            continue
        entries = []
        for va in vas:
            entry = {'preferred_va': va, **info[va], 'abi': abi[va]}
            # Keep complementary observations for this exact name together.
            entry['abi'] = {**abi[va], 'contracts': [c for c in abi[va]['contracts'] if c['name'] == name]}
            if abi[va]['contracts'] and not entry['abi']['contracts']:
                entry['abi']['status'] = 'no_manual_contract_for_this_alias'
            if args.runtime_base is not None:
                runtime = args.runtime_base + int(entry['rva'], 16)
                if runtime > 0xffffffff:
                    parser.error('Module base plus RVA exceeds the 32-bit address space')
                entry['runtime_va'] = f'0x{runtime:08x}'
            entries.append(entry)
        matches.append({'name': name, 'entries': entries})
    if not matches:
        parser.error('No accepted game/engine label matches this query')
    print(json.dumps({'target': identity['target'], 'matches': matches}, indent=2))


if __name__ == '__main__':
    main()
