#!/usr/bin/env python3
"""Run a pinned 256-fix checkpoint and trace each lost check down linear JJ history."""

import argparse
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

from bench import CORPUS, MEASURE, ROOT, atomic, records, sha


def jj(*args):
    return subprocess.check_output(["jj", *args], cwd=ROOT)


def revision(rev):
    result = jj("log", "--no-graph", "-r", rev, "-T", 'commit_id ++ "\n"').decode().splitlines()
    if len(result) != 1:
        raise ValueError(f"expected one JJ commit: {rev}")
    return result[0]


def linear_history(base, tip):
    template = 'commit_id ++ "\t" ++ parents.map(|p| p.commit_id()).join(" ") ++ "\t" ++ description.first_line() ++ "\n"'
    raw = jj("log", "--no-graph", "-r", f"{base}::{tip}", "-T", template)
    history = [(commit, parent, title) for commit, parent, title in
               (line.split("\t", 2) for line in raw.decode().splitlines()[::-1])]
    if not history or history[0][0] != base or history[-1][0] != tip:
        raise ValueError("checkpoint must descend from the baseline")
    if any(current[1] != prev[0] for prev, current in zip(history, history[1:])):
        raise ValueError("history is not a single-parent JJ chain")
    return [(commit, title) for commit, _, title in history]


def first_losses(history, losses, probe):
    """Bisect by file/check, then confirm the first loss even after transient recovery."""
    candidates = {loss: (0, len(history) - 1) for loss in losses}
    while True:
        cohorts = {}
        for loss, (good, bad) in candidates.items():
            if bad > good + 1:
                cohorts.setdefault((good + bad) // 2, set()).add(loss)
        if not cohorts:
            break
        for index, cohort in cohorts.items():
            failed = probe(index, cohort)
            for loss in cohort:
                good, bad = candidates[loss]
                candidates[loss] = (good, index) if loss in failed else (index, bad)
    pending = set(losses)
    answer = {}
    for index in range(1, max((bad for _, bad in candidates.values()), default=0) + 1):
        cohort = {loss for loss in pending if index <= candidates[loss][1]}
        if not cohort:
            break
        for loss in probe(index, cohort):
            answer[loss] = index
            pending.remove(loss)
    return answer, pending


def run(args, base, tip, history):
    area = MEASURE / "checkpoint-state" / args.name
    area.mkdir(parents=True, exist_ok=False)
    binaries = area / "binaries"
    binaries.mkdir()
    cache = {}

    def binary(rev):
        path = binaries / rev
        if path.exists():
            return path
        # Rebuild the exact immutable source without moving the active workspace.
        with tempfile.TemporaryDirectory(dir=area) as tmp:
            source = Path(tmp)
            for name in jj("file", "list", "-r", rev).decode().splitlines():
                if name not in {"Cargo.toml", "Cargo.lock", "build.rs"} and not name.startswith(("src/", "queries/", ".cargo/")):
                    continue
                target = source / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(jj("file", "show", "-r", rev, "--", name))
            subprocess.run(["cargo", "build", "--release", "--locked", "--target-dir", str(area / "target")], cwd=source, check=True)
            shutil.copy2(area / "target/release/topiary-markdown", path)
        return path

    def bench(name, rev, subset=None, before=None):
        argv = [sys.executable, str(ROOT / "scripts/bench.py"), name, "--binary", str(binary(rev)),
                "--manifest", str(args.manifest), "--revision", rev, "--jobs", str(args.jobs),
                "--style-binary", str(args.style_binary), "--style-profile", args.style_profile,
                "--verifier-binary", str(args.verifier_binary)]
        if before:
            argv.extend(["--before", str(binary(before))])
        if subset:
            selection = area / f"{name}.affected"
            selection.write_text("\n".join(sorted(subset)) + "\n")
            argv.extend(["--affected", str(selection), "--no-prettier"])
        result = subprocess.run(argv, cwd=ROOT)
        if result.returncode not in (0, 1):
            raise RuntimeError(f"incomplete benchmark {name}")
        return records(MEASURE / name / "results.jsonl")

    if sha(args.manifest.read_bytes()) != "ed8191e32d929a1ac62c029cedf62beb8fdee6c5e4c7d7023481edeb82298ae2":
        raise ValueError("checkpoint requires the frozen 60,000-file manifest")
    full = bench(args.name, tip, before=base)
    losses = {(row["sha256"], key) for row in full for key in row["regressed"] if key != "pass"}

    def probe(index, cohort):
        nonlocal probe_count
        missing = {digest for digest, _ in cohort if (index, digest) not in cache}
        if missing:
            probe_count += 1
            for row in bench(f"{args.name}.probe-{index}-{probe_count}", history[index][0], missing):
                cache[index, row["sha256"]] = row
        return {(digest, key) for digest, key in cohort if not cache[index, digest][key]}

    probe_count = 0
    confirmed_base = probe(0, losses)
    confirmed_tip = probe(len(history) - 1, losses)
    reproducible = confirmed_tip - confirmed_base
    located, unresolved = first_losses(history, reproducible, probe)
    report = [{"sha256": digest, "lost_check": key, "last_good": history[index - 1][0],
               "first_bad": history[index][0], "description": history[index][1],
               "retest": [digest]} for (digest, key), index in sorted(located.items())]
    atomic(area / "regressions.json", {"first_losses": report,
                                      "unconfirmed": sorted(losses - reproducible | unresolved)})
    return int(bool(losses))


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("name")
    cli.add_argument("--baseline", required=True)
    cli.add_argument("--tip", required=True)
    cli.add_argument("--style-binary", required=True, type=lambda p: Path(p).resolve())
    cli.add_argument("--style-profile", required=True)
    cli.add_argument("--verifier-binary", required=True, type=lambda p: Path(p).resolve())
    cli.add_argument("--manifest", type=lambda p: Path(p).resolve(), default=CORPUS / "manifest.jsonl")
    cli.add_argument("--jobs", type=int, default=10)
    cli.add_argument("--plan", action="store_true", help="Validate and count without starting a benchmark")
    args = cli.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", args.name):
        cli.error("use a simple run name")
    base, tip = revision(args.baseline), revision(args.tip)
    history = linear_history(base, tip)
    early_fixes = {
        "Keep adjacent reference definitions stable and stream gate results",
        "Protect escaped inline and display LaTeX payloads",
    }
    fixes = sum(title in early_fixes or bool(re.match(r"^fix-\d+:", title)) for _, title in history[1:])
    print(f"{fixes} formatter fixes across {len(history) - 1} JJ commits", flush=True)
    if not args.plan:
        if fixes != 256:
            cli.error("set --tip to the 256th actual formatter fix after --baseline")
        raise SystemExit(run(args, base, tip, history))
