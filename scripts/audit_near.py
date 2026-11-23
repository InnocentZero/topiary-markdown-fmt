#!/usr/bin/env python3
"""Find near-duplicate bodies via bottom-k sampling then exact line-set overlap.

Counts are lower bounds: rewrapping changes line features, and indexing drops
high-frequency fingerprints and documents with fewer than 12 substantive lines.
"""
from collections import Counter
from functools import lru_cache
import hashlib
import heapq
import json

from audit_corpus import body_of
from bench import CORPUS, records


def source(row):
    return f"{row['repo']}/{row['path']}"


def main():
    rows = records(CORPUS / 'manifest.jsonl')
    k, limit = 24, 48
    index, sketches, membership = {}, [], {}
    pairs, skipped, identical = 0, 0, 0
    examples = []

    @lru_cache(maxsize=1024)
    def featured(i):
        text = body_of((CORPUS / rows[i]['blob']).read_bytes().decode('utf-8'))
        return frozenset(line for raw in text.splitlines() if len(line := ' '.join(raw.split())) >= 24)

    def root(i):
        while i in membership:
            i = membership[i]
        return i

    for i, row in enumerate(rows):
        lines = featured(i)
        sketch = frozenset(heapq.nsmallest(k, (
            hashlib.blake2s(line.encode(), digest_size=8).digest() for line in lines))) if len(lines) >= 12 else frozenset()
        sketches.append(sketch)
        skipped += not bool(sketch)
        candidates = Counter(j for h in sketch if (bucket := index.get(h)) for j in bucket)
        for j, overlap in sorted(candidates.items(), key=lambda item: (-item[1], item[0])):
            if overlap < 6:
                break
            if root(i) == root(j):
                continue
            union = heapq.nsmallest(k, sketch | sketches[j])
            if sum(h in sketch and h in sketches[j] for h in union) < .67 * len(union):
                continue
            other = featured(j)
            shared = len(lines & other)
            similarity = shared / (len(lines) + len(other) - shared)
            if similarity >= .85:
                membership[root(i)] = root(j)
                pairs += 1
                identical += similarity == 1 and body_of((CORPUS / row['blob']).read_text()).split() == body_of((CORPUS / rows[j]['blob']).read_text()).split()
                if len(examples) < 25 and similarity != 1:
                    examples.append({'jaccard': round(similarity, 3), 'sources': [source(row), source(rows[j])]})
        for h in sketch:
            bucket = index.setdefault(h, [])
            if bucket is not None:
                bucket.append(i)
                if len(bucket) >= limit:
                    index[h] = None
    print(json.dumps({'rows': len(rows), 'too_few_long_lines': skipped, 'redundant_lower_bound': pairs,
                      'same_body_edges': identical, 'additional_near_duplicate_edges': pairs - identical,
                      'examples': examples}, indent=2))


if __name__ == '__main__':
    main()
