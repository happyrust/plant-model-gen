#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Summarize tracing-chrome output (Chrome trace JSON) by span name.

Usage:
  python scripts/profile/summarize_chrome_trace.py output/profile/chrome_trace_*.json
"""

from __future__ import annotations

import json
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Agg:
    cnt: int = 0
    dur_us: int = 0  # Chrome trace uses microseconds by default


def main() -> int:
    if len(sys.argv) != 2:
        print("Usage: python scripts/profile/summarize_chrome_trace.py <trace.json>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    # tracing-chrome 默认输出为一个 JSON 数组，每行一个 event，末尾可能因为进程异常退出而截断。
    # 这里采用流式解析：遇到不可解析的行时，允许容错并在错误过多后停止。
    spans: dict[str, Agg] = defaultdict(Agg)
    # Stack keyed by (pid, tid). Values are list of (name, ts_us_float).
    stacks: dict[tuple[int, int], list[tuple[str, float]]] = defaultdict(list)
    parsed = 0
    errors = 0

    with path.open("r", encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = raw.strip()
            if not line or line == "[" or line == "]":
                continue
            if line.endswith(","):
                line = line[:-1]
            try:
                ev = json.loads(line)
                parsed += 1
            except json.JSONDecodeError:
                errors += 1
                # 通常只会在文件尾部出现一次截断；容错少量错误后直接退出以节省时间。
                if errors > 50:
                    break
                continue

            ph = ev.get("ph")
            name = ev.get("name")

            # Complete events
            if ph == "X":
                if not name:
                    continue
                dur = ev.get("dur")
                if isinstance(dur, (int, float)):
                    spans[name].cnt += 1
                    spans[name].dur_us += int(dur)
                continue

            # Begin/End span events
            if ph in ("B", "E"):
                pid = ev.get("pid")
                tid = ev.get("tid")
                ts = ev.get("ts")
                if not isinstance(pid, int) or not isinstance(tid, int) or not isinstance(ts, (int, float)):
                    continue
                key = (pid, tid)

                if ph == "B":
                    if name:
                        stacks[key].append((name, float(ts)))
                    continue

                # ph == "E"
                if not stacks.get(key):
                    continue
                start_name, start_ts = stacks[key].pop()
                end_ts = float(ts)
                dur_us = int(max(0.0, end_ts - start_ts))
                spans[start_name].cnt += 1
                spans[start_name].dur_us += dur_us
                continue

    rows = []
    for name, agg in spans.items():
        rows.append((agg.dur_us, agg.cnt, name))
    rows.sort(reverse=True)

    print(f"Trace: {path}")
    print(f"Parsed events: {parsed} (errors: {errors})")
    print(f"Unique spans: {len(rows)}")
    print("")
    print(f"{'total_ms':>12} {'count':>8}  name")
    print("-" * 60)
    for dur_us, cnt, name in rows[:50]:
        print(f"{dur_us/1000.0:12.3f} {cnt:8d}  {name}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
