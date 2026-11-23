#!/usr/bin/env python3
"""Create the Nix release manifest from packaged CI binaries."""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import re


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('repository')
    cli.add_argument('tag')
    cli.add_argument('archives', nargs='+', type=Path)
    args = cli.parse_args()
    if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', args.tag):
        cli.error('expected a release tag such as v0.1.0')
    hashes = {}
    for archive in args.archives:
        name = re.fullmatch(r'topiary-markdown-([a-z_0-9-]+)\.tar\.gz', archive.name)
        if not name or name[1] in hashes:
            cli.error(f'unexpected or duplicate archive: {archive}')
        digest = hashlib.sha256(archive.read_bytes()).digest()
        hashes[name[1]] = 'sha256-' + base64.b64encode(digest).decode('ascii')
    print(json.dumps({'repository': args.repository, 'tag': args.tag, 'hashes': hashes},
                     sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
