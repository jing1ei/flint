"""Validate all application version manifests and print the version for release automation."""
import json
from pathlib import Path
import re
import sys
import tomllib


def version(root=Path('.')):
    package=json.loads((root/'package.json').read_text())
    lock=json.loads((root/'package-lock.json').read_text())
    workspace=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
    shell=tomllib.loads((root/'src-tauri/Cargo.toml').read_text())['package']['version']
    shell=workspace if isinstance(shell,dict) and shell.get('workspace') else shell
    found={'package':package['version'],'npm lock':lock['version'],
           'npm root':lock['packages']['']['version'],'workspace':workspace,'shell':shell,
           'Tauri':json.loads((root/'src-tauri/tauri.conf.json').read_text())['version']}
    crates=tomllib.loads((root/'Cargo.lock').read_text())['package']
    for name in ['flint','convert-core']:
        entries=[p['version'] for p in crates if p['name']==name and 'source' not in p]
        if len(entries)!=1:raise ValueError(f'Expected one local {name} entry in Cargo.lock')
        found[name+' lock']=entries[0]
    values=set(found.values())
    if len(values)!=1:raise ValueError(f'Version manifests disagree: {found}')
    value=values.pop()
    if not re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?',value):
        raise ValueError(f'Invalid release version: {value}')
    if '-' in value and any(p.isdigit() and len(p)>1 and p[0]=='0' for p in value.split('-',1)[1].split('.')):
        raise ValueError(f'Invalid numeric prerelease identifier: {value}')
    return value


if __name__=='__main__':
    try:
        value=version()
        if len(sys.argv)>1 and sys.argv[1]!=value:raise ValueError(f'Tag says {sys.argv[1]}, manifests say {value}')
        print(value)
    except (ValueError,KeyError,OSError) as error:
        sys.exit(str(error))
