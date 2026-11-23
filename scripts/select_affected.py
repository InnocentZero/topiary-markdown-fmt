#!/usr/bin/env python3
"""Build a reproducible affected set by matching a raw input feature."""

import argparse
from pathlib import Path
import re

from bench import CORPUS, records

PATTERNS = {
    "refs": rb"^ {0,3}\[(?:\\.|[^\]\r\n])+\]:",
    "footnote": rb"\[\^[^\r\n]*\]:",
    "latex": rb"\\[([]",
    "emphasis": rb"[*]|__",
    "emphasis-edge-underscore": rb"[*]_|_[*]",
    "table-escaped-pipe": rb"[\\]{2,}[|]",
    "list": rb"(?:^|[\s>])(?:[-+*]|[0-9]{1,9}[.)])(?=[ \t\r\n]|$)",
    "code-boundary": rb"[`]{3,}|[~]{3,}|^[ \t>]*(?:[ ]{4}|\t)",
    "rule": rb"(?:[-][ \t]*){3,}|(?:[*][ \t]*){3,}|(?:[_][ \t]*){3,}",
    "table": rb"^[ \t:-]*-[ \t:-]*[|]|^[ \t]*[|][ \t:-]*-|^[^\r\n]*[|][^\r\n]*\r?\n[ \t]*:?-+:?[ \t]*\r?$",
}

if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("regex")
    cli.add_argument("output", type=Path)
    cli.add_argument("--manifest", type=Path, default=CORPUS / "manifest.jsonl")
    cli.add_argument("--occurrences", type=int, default=1)
    args = cli.parse_args()
    regex = re.compile(PATTERNS.get(args.regex, args.regex.encode()), re.MULTILINE)
    matched = 0
    temporary = args.output.with_suffix(".selecting")
    with temporary.open("w") as group:
        for row in records(args.manifest):
            source = (CORPUS / row["blob"]).read_bytes()
            found = 0
            for _ in regex.finditer(source):
                found += 1
                if found == args.occurrences:
                    group.write(row["sha256"] + "\n")
                    matched += 1
                    break
    temporary.replace(args.output)
    print(f"{matched:,} matching corpus files")
