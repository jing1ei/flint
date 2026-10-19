"""Reject Mac artifacts whose architecture or deployment target contradicts the app contract."""
import argparse
import json
from pathlib import Path
import re
import subprocess


def version(value):
    parts = tuple(int(part) for part in value.split('.'))
    return parts + (0,) * (3 - len(parts))


def check(path, arch, maximum='11.0'):
    arches = subprocess.check_output(['lipo', '-archs', str(path)], text=True).split()
    if arch not in arches:
        raise ValueError(f'{path}: expected {arch}, found {arches}')
    commands = subprocess.check_output(['otool', '-arch', arch, '-l', str(path)], text=True)
    minimum = []
    for block in commands.split('Load command'):
        if 'LC_BUILD_VERSION' in block:
            minimum.extend(re.findall(r'\bminos\s+(\d+(?:\.\d+){1,2})', block))
        elif 'LC_VERSION_MIN_MACOSX' in block:
            minimum.extend(re.findall(r'\bversion\s+(\d+(?:\.\d+){1,2})', block))
    if not minimum or any(version(value) > version(maximum) for value in minimum):
        raise ValueError(f'{path}: requires macOS {minimum or "unknown"}; maximum allowed is {maximum}')
    print(f'PASS {path.name}: {arch}, minimum macOS {", ".join(minimum)}')


def minimum_for_arch(root, arch):
    config = root / 'src-tauri' / ('tauri.arm64.conf.json' if arch == 'arm64' else 'tauri.conf.json')
    return json.loads(config.read_text())['bundle']['macOS']['minimumSystemVersion']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch', required=True, choices=['arm64', 'x86_64'])
    parser.add_argument('paths', nargs='+', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    minimum = minimum_for_arch(root, args.arch)
    try:
        for path in args.paths:
            check(path, args.arch, minimum)
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Compatibility check failed: {error}\nUse a build compiled for macOS {minimum}; do not patch its Mach-O version header.\n')


if __name__ == '__main__':
    main()
