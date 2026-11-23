#!/usr/bin/env python3
"""Validate content-addressed shard manifests and deterministically freeze a deduplicated corpus."""

import argparse
from collections import Counter, defaultdict, deque
import hashlib
import json
from pathlib import Path, PurePosixPath
import re

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
MIN_BYTES = 4096


def merge(target, minimum, output):
    unique, duplicates, shards = {}, 0, {}
    for manifest in sorted((CORPUS / "shards").glob("*/manifest.jsonl")):
        records = manifest.read_bytes().splitlines(keepends=True)
        if records and not records[-1].endswith(b"\n"):
            raise ValueError(f"Partial record in {manifest}; retry once the shard is published")
        counts = Counter()
        for line in records:
            rec = json.loads(line)
            digest, raw_path = rec["sha256"], rec["path"]
            path = PurePosixPath(raw_path)
            if not (re.fullmatch(r"[a-f0-9]{64}", digest)
                    and re.fullmatch(r"[a-f0-9]{40}", rec["revision"])
                    and re.fullmatch(r"[\w.-]+/[\w.-]+", rec["repo"])
                    and not path.is_absolute() and ".." not in path.parts
                    and path.suffix.lower() in {".md", ".markdown"}):
                raise ValueError(f"Invalid provenance: {rec}")
            blob = manifest.parent / "blobs" / f"{digest}.md"
            data = blob.read_bytes()
            data.decode("utf-8", errors="strict")
            if len(data) < MIN_BYTES or len(data) != rec["bytes"] or hashlib.sha256(data).hexdigest() != digest:
                raise ValueError(f"Invalid blob {blob}")
            counts["verified"] += 1
            if digest in unique:
                counts["duplicates"] += 1
                duplicates += 1
                continue
            unique[digest] = {"sha256": digest, "bytes": len(data), "repo": rec["repo"],
                              "revision": rec["revision"], "path": raw_path,
                              "blob": blob.relative_to(CORPUS).as_posix()}
        shards[manifest.parent.name] = dict(counts)
    if len(unique) < minimum:
        raise ValueError(f"Only {len(unique):,} unique valid files, need {minimum:,}")
    # Round-robin source repositories so a large translation/version tree cannot
    # displace all smaller projects. SHA ordering breaks ties reproducibly.
    repos = defaultdict(deque)
    for digest, rec in sorted(unique.items()):
        repos[rec["repo"]].append(rec)
    queue = deque(repos[repo] for repo in sorted(repos, key=str.casefold))
    selected = []
    while queue and len(selected) < target:
        group = queue.popleft()
        selected.append(group.popleft())
        if group:
            queue.append(group)
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise FileExistsError(f"Refusing to overwrite pinned manifest {output}")
    with output.open("xb") as out:
        for rec in selected:
            out.write((json.dumps(rec, sort_keys=True, ensure_ascii=True) + "\n").encode())
    print(json.dumps({"selected": len(selected), "unique": len(unique), "cross_shard_duplicates": duplicates,
                      "repos": len({r["repo"] for r in selected}), "shards": shards,
                      "manifest_sha256": hashlib.sha256(output.read_bytes()).hexdigest()}, indent=2))


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--target", type=int, default=60000)
    cli.add_argument("--minimum", type=int, default=60000)
    cli.add_argument("--out", type=Path, default=CORPUS / "manifest.jsonl")
    args = cli.parse_args()
    if not 1 <= args.minimum <= args.target:
        cli.error("require 1 <= minimum <= target")
    merge(args.target, args.minimum, args.out)
