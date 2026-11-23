#!/usr/bin/env python3
"""Materialize independent reference diffs for reproducible spot checks."""

import argparse
import difflib
import json
from pathlib import Path
import random

from bench import CORPUS, ROOT, Worker, records, sha


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('results', type=Path)
    cli.add_argument('binary', type=lambda p: Path(p).resolve())
    cli.add_argument('directory', type=Path)
    args = cli.parse_args()
    passed = sorted((r for r in records(args.results) if r['pass']), key=lambda r: r['sha256'])
    nonexact = [r for r in passed if not r.get('exact_prettier') and r.get('prettier_line_max')]
    high = sorted(nonexact, key=lambda r: r['prettier_line_distance'] / r['prettier_line_max'], reverse=True)
    selected = [('random', r) for r in random.Random('markdown-output-audit-v1').sample(passed, 24)]
    selected.extend(('largest-delta', r) for r in high[:10])
    args.directory.mkdir(parents=True, exist_ok=False)
    with Worker([str(args.binary), '--worker'], 10) as native, Worker(['node', str(ROOT / 'scripts/prettier.mjs')], 10) as reference:
        for group, row in selected:
            digest = row['sha256']
            directory = args.directory / f'{group}-{digest[:12]}'
            directory.mkdir()
            raw = (CORPUS / row['blob']).read_bytes()
            assert sha(raw) == digest
            source = raw.decode('utf-8')
            actual = native.call({'source': source})['output']
            assert sha(actual.encode()) == row['output_sha256']
            expected = reference.call({'source': source}).get('output')
            (directory / 'input.md').write_bytes(raw)
            (directory / 'topiary.md').write_bytes(actual.encode())
            if expected is not None:
                (directory / 'prettier.md').write_bytes(expected.encode())
                diff = difflib.unified_diff(actual.splitlines(keepends=True), expected.splitlines(keepends=True), fromfile='topiary', tofile='prettier')
                (directory / 'prettier.diff').write_bytes(''.join(diff).encode())
            print(json.dumps({'group': group, 'directory': str(directory), **row}, ensure_ascii=True), flush=True)


if __name__ == '__main__':
    main()
