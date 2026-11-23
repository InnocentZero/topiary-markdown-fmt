#!/usr/bin/env python3
"""Corpus benchmark, independent replay, deterministic before/after sample gate."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import random
import re
import select
import signal
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
MEASURE = ROOT / "measurements"
STOP = threading.Event()
CHECKS = ("formatted", "ast", "protected", "idempotent", "reproducible", "style", "pass", "changed", "exact_prettier")
HARD_CHECKS = CHECKS[:7]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def distance(a, b):
    """Exact Levenshtein distance on Unicode lines, using Python ints as bit vectors."""
    a, b = a.splitlines(keepends=True), b.splitlines(keepends=True)
    n = len(a)
    if not n:
        return len(b), len(b)
    equal = {}
    for i, line in enumerate(a):
        equal[line] = equal.get(line, 0) | (1 << i)
    pos, neg, score, top = -1, 0, n, 1 << (n - 1)
    for line in b:
        eq = equal.get(line, 0)
        x = eq | neg
        d = (((eq & pos) + pos) ^ pos) | eq
        hp, hn = neg | ~(d | pos), d & pos
        score += bool(hp & top) - bool(hn & top)
        hp, hn = (hp << 1) | 1, hn << 1
        pos, neg = hn | ~(x | hp), hp & x
    return score, max(n, len(b))


class Worker:
    def __init__(self, argv, timeout, *, ensure_ascii=False):
        self.argv, self.timeout = argv, timeout
        self.ensure_ascii = ensure_ascii
        self.proc = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        if self.proc is not None:
            if self.proc.poll() is None:
                self.proc.kill()
            self.proc.wait()
            self.proc.stdin.close()
            self.proc.stdout.close()
            self.proc = None

    def call(self, request):
        try:
            if self.proc is None:
                self.proc = subprocess.Popen(self.argv, cwd=ROOT, stdin=subprocess.PIPE,
                                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            payload = (json.dumps(request, ensure_ascii=self.ensure_ascii) + "\n").encode()
            stdin, stdout = self.proc.stdin.fileno(), self.proc.stdout.fileno()
            deadline = time.monotonic() + self.timeout
            pos, response = 0, bytearray()
            os.set_blocking(stdin, False)
            os.set_blocking(stdout, False)
            while pos < len(payload):
                if not select.select([], [stdin], [], max(0, deadline - time.monotonic()))[1]:
                    raise TimeoutError
                pos += os.write(stdin, payload[pos:pos + 65536])
            while not response.endswith(b"\n"):
                if not select.select([stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                    raise TimeoutError
                data = os.read(stdout, 65536)
                if not data:
                    raise EOFError("worker exited before response")
                response.extend(data)
            return json.loads(response)
        except TimeoutError:
            self.close()
            return {"error": "timeout"}
        except (OSError, EOFError, ValueError) as error:
            self.close()
            return {"error": f"worker failure: {error}"}


@dataclass
class Bench:
    first: Worker
    replay: Worker
    checker: Worker | None = None
    verifier: Worker | None = None

    def check(self, source, prettier):
        first = self.first.call({"source": source, "audit": True})
        replay = self.replay.call({"source": source})
        output = first.get("output")
        row = {key: False for key in HARD_CHECKS}
        row["formatted"] = output is not None
        row["reproducible"] = output is not None and output == replay.get("output")
        row["changed"] = output is not None and output != source
        if output is None:
            row["error"] = first.get("error", "missing output")[:1000]
        else:
            checked = self.checker.call({"source": output}) if self.checker else {"style": first["style"]}
            verified = self.verifier.call({"original": source, "source": output}) if self.verifier else first
            issues = checked.get("style", ["style-checker-error"])
            row.update({"ast": verified.get("ast") is True, "protected": verified.get("protected") is True,
                         "style": not issues, "style_issues": issues,
                        "idempotent": first["idempotent"], "output_sha256": sha(output.encode())})
            if checked.get("error"):
                row["style_error"] = checked["error"]
            if verified.get("error") or verified.get("verification"):
                row["verification"] = verified.get("error") or verified["verification"]
            if first.get("second_error"):
                row["second_error"] = first["second_error"][:1000]
        if replay.get("error"):
            row["replay_error"] = replay["error"][:1000]
        row["pass"] = all(row[key] for key in HARD_CHECKS if key != "pass")
        ref = prettier.get("output")
        if ref is not None and output is not None:
            row["exact_prettier"] = output == ref
            row["prettier_line_distance"], row["prettier_line_max"] = distance(output, ref)
        elif prettier.get("error"):
            row["prettier_error"] = prettier["error"][:1000]
        return row


def records(path):
    with path.open(encoding="utf-8") as manifest:
        return [json.loads(line) for line in manifest]


def title(row):
    return row["error"] if "error" in row else row.get("verification") or next(
        (", ".join(row["style_issues"]) if key == "style" else key for key in HARD_CHECKS if not row[key]), "pass")


def atomic(path, data):
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, ensure_ascii=True) + "\n")
    tmp.replace(path)


def run(args):
    STOP.clear()
    signal.signal(signal.SIGINT, lambda *_: STOP.set())
    signal.signal(signal.SIGTERM, lambda *_: STOP.set())
    all_records = records(args.manifest)
    by_hash = {r["sha256"]: r for r in all_records}
    positions = {r["sha256"]: i for i, r in enumerate(all_records)}
    if len(by_hash) != len(all_records):
        raise ValueError("manifest is not unique")
    pool = sorted(by_hash)
    affected = set(args.sha)
    for file in args.affected:
        affected.update(ln.strip() for ln in file.read_text().splitlines() if ln.strip())
    if not affected <= by_hash.keys():
        raise ValueError(f"Unknown affected IDs: {sorted(affected - by_hash.keys())}")
    sample = set(random.Random(args.seed).sample(pool, args.sample)) if args.sample else set()
    if affected or sample:
        selected = [by_hash[digest] for digest in sorted(affected | sample)]
    else:
        selected = all_records
    directory = MEASURE / args.name
    directory.mkdir(parents=True, exist_ok=False)
    pin = subprocess.check_output(["jj", "log", "-r", "@", "--no-graph", "-T", "commit_id"], cwd=ROOT, text=True).strip()
    stats = {"name": args.name, "revision": args.revision or pin, "total": len(selected), "done": 0,
             "state": "running", "upstream": args.upstream, "started": time.time(),
             "manifest_sha256": sha(args.manifest.read_bytes()), "binary_sha256": sha(args.binary.read_bytes()),
             "prettier_version": None if args.no_prettier else subprocess.check_output(
                 ["node", str(ROOT / "scripts/prettier.mjs"), "--version"], text=True).strip(),
             "compared_binary_sha256": sha(args.before.read_bytes()) if args.before else None,
             "sample_seed": args.seed, "sample_sha256": sorted(sample), "affected": sorted(affected),
              "style_profile": args.style_profile if args.style_binary else "preliminary",
               "style_binary_sha256": sha(args.style_binary.read_bytes()) if args.style_binary else None,
               "verifier_binary_sha256": sha(args.verifier_binary.read_bytes()) if args.verifier_binary else None,
               "checks": {}, "denominators": {}, "failures": {}, "style_issues": {},
               "regressions": 0, "improvements": 0, "fixed": 0,
             "distance_sum": 0, "distance_max_sum": 0, "recent_failures": []}
    atomic(directory / "status.json", stats)
    grid = bytearray(b"0" * len(all_records))

    def publish_grid():
        tmp = directory / "grid.tmp"
        tmp.write_bytes(grid)
        tmp.replace(directory / "grid.txt")

    publish_grid()
    flag = "--upstream-worker" if args.upstream else "--worker"
    lock = threading.Lock()
    items = iter(selected)
    errors = []
    stats["recent_results"] = []
    with (directory / "results.jsonl").open("x", encoding="utf-8") as log:
        published = 0

        def publish():
            nonlocal published
            log.flush()
            publish_grid()
            atomic(directory / "status.json", stats)
            published = stats["done"]

        def flush_interval():
            while not finished.wait(0.30):
                with lock:
                    if published != stats["done"]:
                        publish()

        finished = threading.Event()

        def consume():
            with ExitStack() as children:
                def worker(*argv):
                    return children.enter_context(Worker(argv, args.timeout))
                checker = worker(str(args.style_binary), "--style-worker") if args.style_binary else None
                verifier = worker(str(args.verifier_binary), "--verify-worker") if args.verifier_binary else None
                new = Bench(worker(str(args.binary), flag), worker(str(args.binary), flag), checker, verifier)
                old = Bench(worker(str(args.before), "--worker"), worker(str(args.before), "--worker"), checker, verifier) if args.before else None
                ref = None if args.no_prettier else worker("node", str(ROOT / "scripts/prettier.mjs"))
                while not STOP.is_set():
                    with lock:
                        rec = next(items, None)
                    if rec is None:
                        return
                    raw = (CORPUS / rec["blob"]).read_bytes()
                    if sha(raw) != rec["sha256"]:
                        raise ValueError(f"Blob changed: {rec['blob']}")
                    source = raw.decode("utf-8")
                    prettier = ref.call({"source": source}) if ref else {}
                    result = new.check(source, prettier)
                    row = {**rec, **result, "sample": rec["sha256"] in sample, "affected": rec["sha256"] in affected}
                    if old:
                        row["before"] = old.check(source, prettier)
                        row["regressed"] = [k for k in HARD_CHECKS if row["before"][k] and not result[k]]
                        row["improved"] = [k for k in HARD_CHECKS if result[k] and not row["before"][k]]
                    with lock:
                        log.write(json.dumps(row, ensure_ascii=True) + "\n")
                        stats["done"] += 1
                        verdict = ("3" if row.get("regressed") else
                                   "4" if row["pass"] and row.get("improved") and not row["before"]["pass"] else
                                   "1" if row["pass"] else "2")
                        grid[positions[rec["sha256"]]] = ord(verdict)
                        stats["recent_results"] = [{"path": row["path"], "repo": row["repo"], "verdict": verdict,
                                                    "reason": title(row)[:165]}, *stats["recent_results"][:11]]
                        print(".", end="", flush=True)
                        if stats["done"] % 80 == 0:
                            print(flush=True)
                        for key in CHECKS:
                            if key in row:
                                d, c = stats["denominators"], stats["checks"]
                                d[key] = d.get(key, 0) + 1
                                c[key] = c.get(key, 0) + int(row[key])
                        for issue in row.get("style_issues", []):
                            stats["style_issues"][issue] = stats["style_issues"].get(issue, 0) + 1
                        if not row["pass"]:
                            why = (title(row).strip() or "unspecified formatter error").splitlines()[0][:165]
                            stats["failures"][why] = stats["failures"].get(why, 0) + 1
                            stats["recent_failures"] = [{"path": row["path"], "repo": row["repo"],
                                                         "sha256": row["sha256"], "reason": why}, *stats["recent_failures"][:19]]
                        stats["regressions"] += bool(row.get("regressed"))
                        stats["improvements"] += bool(row.get("improved"))
                        stats["fixed"] += verdict == "4"
                        stats["distance_sum"] += row.get("prettier_line_distance", 0)
                        stats["distance_max_sum"] += row.get("prettier_line_max", 0)
                        if stats["done"] - published >= 24:
                            publish()
                        if stats["done"] % 400 == 0:
                            print(f"{args.name}: {stats['done']:,}/{len(selected):,}; pass {stats['checks']['pass']:,}; regressions {stats['regressions']:,}", flush=True)

        publisher = threading.Thread(target=flush_interval)
        publisher.start()
        try:
            with ThreadPoolExecutor(max_workers=args.jobs) as threads:
                for task in [threads.submit(consume) for _ in range(args.jobs)]:
                    try:
                        task.result()
                    except Exception as error:
                        STOP.set()
                        errors.append(str(error))
        finally:
            finished.set()
            publisher.join()
    stats["state"] = "error" if errors else "interrupted" if stats["done"] != len(selected) else "complete"
    stats["ended"] = time.time()
    if errors:
        stats["errors"] = errors
    publish_grid()
    atomic(directory / "status.json", stats)
    if stats["done"] % 80:
        print()
    print(json.dumps({k: v for k, v in stats.items() if k not in {"recent_results", "recent_failures", "sample_sha256", "affected"}}, indent=2))
    return 2 if stats["state"] != "complete" else int(bool(stats["regressions"]))


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("name")
    ap.add_argument("--manifest", type=Path, default=CORPUS / "manifest.jsonl")
    ap.add_argument("--binary", type=lambda p: Path(p).resolve(), default=ROOT / "target/release/topiary-markdown")
    ap.add_argument("--before", type=lambda p: Path(p).resolve())
    ap.add_argument("--style-binary", type=lambda p: Path(p).resolve())
    ap.add_argument("--style-profile", choices=[f"prettier-like-v{n}" for n in range(1, 7)], default="prettier-like-v1")
    ap.add_argument("--verifier-binary", type=lambda p: Path(p).resolve(), help="Pinned independent AST and protected-byte checker")
    ap.add_argument("--revision", help="Exact revision of the binary being tested")
    ap.add_argument("--sample", type=int, default=0)
    ap.add_argument("--seed", default="topiary-markdown")
    ap.add_argument("--sha", action="append", default=[])
    ap.add_argument("--affected", action="append", type=Path, default=[])
    ap.add_argument("--upstream", action="store_true")
    ap.add_argument("--no-prettier", action="store_true")
    ap.add_argument("--jobs", type=int, default=max(1, min(10, (os.cpu_count() or 2) - 2)))
    ap.add_argument("--timeout", type=float, default=20)
    args = ap.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", args.name):
        ap.error("use a simple run name")
    raise SystemExit(run(args))
