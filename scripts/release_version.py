"""Check that every version manifest agrees, and print the version for release automation.

    python3 scripts/release_version.py            # print the version
    python3 scripts/release_version.py v1.2.3     # also require it to match a tag (v optional)
"""
import json
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(
    r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)'
    r'(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?'
)


def version(root=ROOT):
    package = json.loads((root / 'package.json').read_text())
    lock = json.loads((root / 'package-lock.json').read_text())
    workspace = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
    shell = tomllib.loads((root / 'src-tauri/Cargo.toml').read_text())['package']['version']
    if isinstance(shell, dict) and shell.get('workspace'):
        shell = workspace
    found = {
        'package.json': package['version'],
        'package-lock.json': lock['version'],
        'package-lock.json packages[""]': lock['packages']['']['version'],
        'Cargo.toml': workspace,
        'src-tauri/Cargo.toml': shell,
        'tauri.conf.json': json.loads((root / 'src-tauri/tauri.conf.json').read_text())['version'],
    }
    crates = tomllib.loads((root / 'Cargo.lock').read_text())['package']
    for name in ['flint', 'convert-core']:
        entries = [p['version'] for p in crates if p['name'] == name and 'source' not in p]
        if len(entries) != 1:
            raise ValueError(f'Expected one local {name} entry in Cargo.lock')
        found[f'Cargo.lock ({name})'] = entries[0]
    values = set(found.values())
    if len(values) != 1:
        raise ValueError(f'Version manifests disagree: {found}')
    value = values.pop()
    if not SEMVER.fullmatch(value):
        raise ValueError(f'Invalid release version: {value}')
    return value


if __name__ == '__main__':
    try:
        value = version()
        if len(sys.argv) > 1 and sys.argv[1].removeprefix('v') != value:
            raise ValueError(f'Tag says {sys.argv[1]}, manifests say {value}')
        print(value)
    except (ValueError, KeyError, OSError) as error:
        sys.exit(str(error))
