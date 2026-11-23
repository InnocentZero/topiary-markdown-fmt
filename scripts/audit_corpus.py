#!/usr/bin/env python3
"""Recompute pinned corpus integrity, concentration and normalized duplication."""

from collections import Counter, defaultdict
import hashlib
import json
import re

from bench import CORPUS, records


def summarize(values):
    values = sorted(values)
    return {k: values[round((len(values) - 1) * p)] for k, p in
            [('min', 0), ('p10', .1), ('median', .5), ('p90', .9), ('max', 1)]}


def histogram(mapping):
    groups = [group for group in mapping.values() if len(group) > 1]
    return {'groups': len(groups), 'redundant_files': sum(len(g) - 1 for g in groups),
            'largest_groups': [g[:5] for g in sorted(groups, key=len, reverse=True)[:5]]}


def body_of(text):
    if text.startswith(('---\n', '---\r\n', '+++\n', '+++\r\n')):
        opener, sep, body = text.partition('\n')
        delimiter = re.escape(opener.removesuffix('\r'))
        end = re.search(rf'(?m)^{delimiter}[ \t]*\r?$', body)
        if end:
            text = body[end.end():]
    return re.sub(r'<!--.*?-->', '', text, flags=re.S)


def main():
    path = CORPUS / 'manifest.jsonl'
    rows = records(path)
    sizes, lines, bodies, errors = [], [], [], []
    feature_counts, repos, shard = Counter(), Counter(), Counter()
    raw_hashes, whitespace, naked = set(), defaultdict(list), defaultdict(list)
    patterns = {key: re.compile(pattern, re.M) for key, pattern in {
        'list_marker': r'^ *([-+*]|\d+[.)])\s',
        'table_delimiter_candidate': r'^\s*\|?[ :\t-]*---[ :\t-]*\|[ :\t|-]*$',
        'fence': r'^\s*(`{3,}|~{3,})',
        'reference_candidate': r'^ *\[[^\]\n]+\]:',
        'slash_math_candidate': r'\\[([]',
    }.items()}
    for row in rows:
        try:
            data = (CORPUS / row['blob']).read_bytes()
            digest = hashlib.sha256(data).hexdigest()
            if digest in raw_hashes or len(data) < 4096 or len(data) != row['bytes'] or digest != row['sha256']:
                errors.append(row['sha256'])
            text = data.decode('utf-8')
        except (OSError, UnicodeError) as error:
            errors.append(str(error))
            continue
        raw_hashes.add(digest)
        sizes.append(len(data))
        lines.append(len(text.splitlines()))
        repo = row['repo']
        source = f"{repo}/{row['path']}"
        repos[repo] += 1
        shard[row['blob'].split('/')[1]] += 1
        whitespace[hashlib.sha256(' '.join(text.split()).encode()).hexdigest()].append(source)
        body = body_of(text)
        normalized = ' '.join(body.split())
        bodies.append(len(normalized.encode()))
        naked[hashlib.sha256(normalized.encode()).hexdigest()].append(source)
        feature_counts.update(k for k, pattern in patterns.items() if pattern.search(body))
    print(json.dumps({'rows': len(rows), 'manifest_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
        'unique_raw': len(raw_hashes), 'integrity_errors': errors[:20], 'integrity_error_count': len(errors),
        'raw_bytes': summarize(sizes), 'total_bytes': sum(sizes), 'lines': summarize(lines),
        'normalized_body_bytes': summarize(bodies), 'repos': len(repos), 'top_repos': repos.most_common(12),
        'normalized_body_under_4k': sum(n < 4096 for n in bodies), 'normalized_empty_bodies': bodies.count(0),
        'shards': shard, 'feature_candidates': feature_counts,
        'same_after_whitespace': histogram(whitespace),
        'same_after_whitespace_frontmatter_comments': histogram(naked)}, indent=2))
    return bool(errors)


if __name__ == '__main__':
    raise SystemExit(main())
