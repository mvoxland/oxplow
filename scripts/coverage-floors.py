#!/usr/bin/env python3
"""Enforce per-crate line-coverage floors.

Reads `cargo llvm-cov report --json` from stdin and fails (exit 1)
if any crate's aggregated line coverage is below its floor.

Tracking per-crate floors (in addition to the workspace floor in
ci.yml) keeps a regression in one crate from hiding behind a high
average. Floors are deliberately conservative — set just below the
current measured coverage so a real drop fails CI but normal
fluctuation doesn't. Raise them as crate coverage grows.
"""
from __future__ import annotations

import json
import re
import sys
from collections import defaultdict

# Floors are line-coverage percentages. Crates not listed are not
# enforced individually (still counted in the workspace total).
FLOORS = {
    "oxplow-app": 75.0,
    "oxplow-config": 70.0,
    "oxplow-db": 70.0,
    "oxplow-domain": 70.0,
    "oxplow-fs-watch": 60.0,
    "oxplow-git": 80.0,
    "oxplow-lsp": 75.0,
    "oxplow-runtime": 90.0,
    "oxplow-session": 70.0,
    # Subprocess-heavy crates exercised through oxplow-app integration:
    "oxplow-pty": 80.0,
    "oxplow-tmux": 70.0,
    # Adapter crates currently with light coverage. Floors are set at
    # the current baseline and should be raised as integration tests
    # land. See task #78 for the harness that will move these up.
    "oxplow-mcp": 25.0,
    "oxplow-tauri-ipc": 12.0,
}

CRATE_RE = re.compile(r"/crates/([^/]+)/")


def crate_of(path: str) -> str | None:
    m = CRATE_RE.search(path)
    return m.group(1) if m else None


def main() -> int:
    data = json.load(sys.stdin)
    by_crate: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for export in data.get("data", []):
        for f in export.get("files", []):
            crate = crate_of(f["filename"])
            if not crate:
                continue
            lines = f["summary"]["lines"]
            by_crate[crate][0] += lines["count"]
            by_crate[crate][1] += lines["covered"]

    failures: list[str] = []
    print(f"{'crate':<24} {'lines':>10} {'covered':>10} {'pct':>8} {'floor':>8} {'ok'}")
    for crate, floor in sorted(FLOORS.items()):
        total, covered = by_crate.get(crate, [0, 0])
        pct = (covered / total * 100.0) if total else 0.0
        ok = pct >= floor
        flag = "OK" if ok else "FAIL"
        print(f"{crate:<24} {total:>10} {covered:>10} {pct:>7.2f}% {floor:>7.2f}% {flag}")
        if not ok:
            failures.append(f"{crate}: {pct:.2f}% < floor {floor:.2f}%")

    if failures:
        print("\nPer-crate coverage floor failures:", file=sys.stderr)
        for line in failures:
            print(f"  - {line}", file=sys.stderr)
        return 1
    print("\nAll per-crate floors OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
