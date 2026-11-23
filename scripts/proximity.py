#!/usr/bin/env python3
"""Compare pinned Topiary outputs to Prettier without changing acceptance scores."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import json
import math
from pathlib import Path
import random
import subprocess

from bench import CORPUS, ROOT, Worker, distance, records, sha


PROFILES = ('default', 'lf', 'auto', 'indent4', 'tabs', 'wrap', 'unwrap')
SEED = 'markdown-prettier-proximity-v1'


def lf(text):
    return text.replace('\r\n', '\n').replace('\r', '\n')


def eol(text):
    crlf = text.count('\r\n')
    kinds = [name for name, count in zip(('crlf', 'lf', 'cr'),
             (crlf, text.count('\n') - crlf, text.count('\r') - crlf)) if count]
    return 'mixed' if len(kinds) > 1 else next(iter(kinds), 'none')


def proximity(first, second):
    return (0, len(first.splitlines(keepends=True))) if first == second else distance(first, second)


def metrics(first, second, normalized_first):
    a = proximity(first, second)
    b = a if first == normalized_first and second == lf(second) else proximity(normalized_first, lf(second))
    return {'raw': a, 'lf': b}


def summary(values):
    if not values:
        return {'files': 0}
    ratios = sorted(d / n if n else 0 for d, n in values)
    edits, lines = map(sum, zip(*values))
    return {'files': len(values), 'exact': sum(d == 0 for d, _ in values),
            'edits': edits, 'lines': lines, 'weighted_distance': edits / lines if lines else 0,
            'median_edits': sorted(d for d, _ in values)[(len(values) - 1) // 2],
            'p90_distance': ratios[math.ceil(.9 * len(ratios)) - 1],
            'within_five_edits': sum(d <= 5 for d, _ in values),
            'within_five_percent': sum(r <= .05 for r in ratios)}


def tally(rows, names, sample):
    report = {'files': len(rows), 'formatted': sum('eol' in r for r in rows),
              'profiles': {}}
    common = [r for r in rows if all(name in r for name in names)]
    for name in names:
        compared = [r for r in rows if name in r]
        pools = {'all': compared, 'passing': [r for r in compared if r['pass']]}
        if sample:
            pools['common'] = common
        elif name in ('auto', 'lf'):
            for key in ('lf', 'crlf', 'mixed', 'cr', 'none'):
                pools['input_' + key] = [r for r in compared if r['eol'] == key]
        report['profiles'][name] = {
            label: {kind: summary([r[name][kind] for r in pool]) for kind in ('raw', 'lf')}
            for label, pool in pools.items()
        }
    report['input_to_auto_lf'] = summary([r['input_to_auto_lf'] for r in rows if 'input_to_auto_lf' in r])
    report['errors'] = dict(Counter(name for r in rows for name in r.get('errors', ())))
    return report


def run(args):
    status = json.loads((args.acceptance / 'status.json').read_text())
    manifest = CORPUS / 'manifest.jsonl'
    if (status['state'] != 'complete' or status['manifest_sha256'] != sha(manifest.read_bytes())
            or status['binary_sha256'] != sha(args.binary.read_bytes())):
        raise ValueError('expected completed acceptance and matching manifest and binary')
    worker = ROOT / 'scripts/prettier.mjs'
    if status['prettier_version'] != subprocess.check_output(['node', str(worker), '--version'], text=True).strip():
        raise ValueError('Prettier version differs from the acceptance run')
    entries = records(args.acceptance / 'results.jsonl')
    if ({r['sha256'] for r in entries} != {r['sha256'] for r in records(manifest)}
            or len(entries) != status['done']):
        raise ValueError('acceptance results do not cover the manifest')
    if args.sample:
        entries = random.Random(SEED).sample(sorted(entries, key=lambda r: r['sha256']), args.sample)
        names = PROFILES
    else:
        names = ('auto', 'lf')
    args.out.mkdir(parents=True, exist_ok=False)

    def shard(chunk):
        # Node readline also treats literal U+2028/U+2029 as record boundaries.
        with Worker([str(args.binary), '--worker'], 30) as native, Worker(
                ['node', str(worker), '--proximity'], 30, ensure_ascii=True) as ref:
            for row in chunk:
                digest = row['sha256']
                result = {'sha256': digest, 'pass': row['pass']}
                if not row['formatted']:
                    yield result
                    continue
                raw = (CORPUS / row['blob']).read_bytes()
                if sha(raw) != digest:
                    raise ValueError(f'Source changed: {digest}')
                source = raw.decode('utf-8')
                output = native.call({'source': source}).get('output')
                if output is None or sha(output.encode()) != row['output_sha256']:
                    raise ValueError(f'Pinned output failed to reproduce: {digest}')
                result['eol'] = eol(source)
                normalized = lf(output)
                response = ref.call({'source': source, 'names': names})
                refs = response.get('results', {})
                result['errors'] = {}
                for name in names:
                    expected = refs.get(name, {}).get('output')
                    if expected is None:
                        result['errors'][name] = refs.get(name, {}).get('error', response.get('error', 'missing result'))
                        continue
                    score = metrics(output, expected, normalized)
                    result[name] = score
                    if name == 'lf' and 'prettier_line_distance' in row:
                        if score['raw'] != (row['prettier_line_distance'], row['prettier_line_max']):
                            raise ValueError(f'LF comparison drift: {digest}')
                    if name == 'auto':
                        result['input_to_auto_lf'] = proximity(lf(source), lf(expected))
                yield result

    # Each worker has its own process pair and log, so slow files don't stall others.
    def collect(index):
        rows = []
        with (args.out / f'shard-{index:02}.jsonl').open('x', buffering=1) as log:
            for row in shard(entries[index::args.jobs]):
                log.write(json.dumps(row, ensure_ascii=True) + '\n')
                rows.append(row)
        return rows

    with ThreadPoolExecutor(args.jobs) as pool:
        rows = [row for part in pool.map(collect, range(args.jobs)) for row in part]
    result = {'manifest_sha256': status['manifest_sha256'], 'binary_sha256': status['binary_sha256'],
              'revision': status['revision'], 'prettier_version': status['prettier_version'],
              'sample_seed': SEED if args.sample else None, **tally(rows, names, args.sample)}
    (args.out / 'report.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('acceptance', type=Path)
    cli.add_argument('binary', type=lambda p: Path(p).resolve())
    cli.add_argument('out', type=Path)
    cli.add_argument('--sample', type=int)
    cli.add_argument('--jobs', type=int, default=8)
    run(cli.parse_args())
