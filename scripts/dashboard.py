#!/usr/bin/env python3
"""Serve benchmark progress and corpus collection counts on localhost."""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from functools import lru_cache
import json
from pathlib import Path
import re
from urllib.parse import urlsplit, parse_qs

ROOT = Path(__file__).resolve().parent.parent


@lru_cache(maxsize=128)
def cached_count(path, _mtime, _size):
    return path.read_bytes().count(b"\n")


def count(path):
    stat = path.stat()
    return cached_count(path, stat.st_mtime_ns, stat.st_size)


class Dashboard(BaseHTTPRequestHandler):
    def do_GET(self):
        url = urlsplit(self.path)
        if url.path == "/":
            data = Path(__file__).with_suffix(".html").read_bytes()
            content_type = "text/html; charset=utf-8"
        elif url.path == "/api/status":
            runs = []
            for path in (ROOT / "measurements").glob("*/status.json"):
                try:
                    run = json.loads(path.read_bytes())
                    run["affected_total"] = len(run.pop("affected", []))
                    run["sample_total"] = len(run.pop("sample_sha256", []))
                    runs.append(run)
                except (OSError, ValueError):
                    continue
            shards = {}
            for path in sorted((ROOT / "corpus/shards").glob("*/manifest.jsonl")):
                try:
                    shards[path.parent.name] = count(path)
                except OSError:
                    continue
            pin = ROOT / "corpus/manifest.jsonl"
            data = json.dumps({"runs": sorted(runs, key=lambda r: r["started"], reverse=True), "shards": shards,
                                "pinned": count(pin) if pin.exists() else None}).encode()
            content_type = "application/json"
        elif url.path == "/api/grid":
            run = parse_qs(url.query).get("run", [""])[0]
            if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", run):
                self.send_error(400)
                return
            try:
                data = (ROOT / "measurements" / run / "grid.txt").read_bytes()
            except FileNotFoundError:
                self.send_error(404)
                return
            content_type = "text/plain; charset=utf-8"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Security-Policy", "default-src 'none'; connect-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--port", type=int, default=56809)
    args = cli.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Dashboard)
    print(f"http://127.0.0.1:{server.server_port}/", flush=True)
    server.serve_forever()
