#!/usr/bin/env python3
"""按 TOML 区块改写 DbOption 中与部署相关的路径（由环境变量/CLI 传入），不做 Mac 硬编码 sed。"""

from __future__ import annotations

import argparse
import json
import re
import sys

# 区块头（整行以 [name] 为准，忽略行内注释形式的区块）
SECTION_RE = re.compile(r"^\s*\[([^\]]+)\]\s*(#.*)?$")


def _assignment_line(key: str, value: str) -> str:
    return f"{key} = {json.dumps(value)}\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("input", help="源 DbOption.toml")
    ap.add_argument("output", help="输出路径")
    ap.add_argument("--project-path", dest="project_path", help="顶层 project_path")
    ap.add_argument("--meshes-path", dest="meshes_path", help="顶层 meshes_path")
    ap.add_argument(
        "--surreal-script-dir",
        dest="surreal_script_dir",
        help="顶层 surreal_script_dir（远端绝对路径）",
    )
    ap.add_argument(
        "--surreal-data-path",
        dest="surreal_data_path",
        help="[web_server].surreal_data_path 与 [surrealdb].path",
    )
    ap.add_argument(
        "--surrealkv-path",
        dest="surrealkv_path",
        help="[surrealkv].path",
    )
    args = ap.parse_args()

    section = "_root_"
    out_lines: list[str] = []

    with open(args.input, encoding="utf-8") as f:
        for line in f:
            stripped = line.lstrip()
            if stripped.startswith("#") or not stripped.strip():
                out_lines.append(line)
                continue

            msec = SECTION_RE.match(line)
            if msec:
                section = msec.group(1).strip()
                out_lines.append(line)
                continue

            replaced = False
            if section == "_root_":
                pairs = [
                    ("project_path", args.project_path),
                    ("meshes_path", args.meshes_path),
                    ("surreal_script_dir", args.surreal_script_dir),
                ]
                for key, val in pairs:
                    if val is None:
                        continue
                    if re.match(rf"^\s*{re.escape(key)}\s*=", line):
                        out_lines.append(_assignment_line(key, val))
                        replaced = True
                        break
            elif section == "web_server" and args.surreal_data_path:
                if re.match(r"^\s*surreal_data_path\s*=", line):
                    out_lines.append(
                        _assignment_line("surreal_data_path", args.surreal_data_path)
                    )
                    replaced = True
            elif section == "surrealdb" and args.surreal_data_path:
                if re.match(r"^\s*path\s*=", line):
                    out_lines.append(_assignment_line("path", args.surreal_data_path))
                    replaced = True
            elif section == "surrealkv" and args.surrealkv_path:
                if re.match(r"^\s*path\s*=", line):
                    out_lines.append(_assignment_line("path", args.surrealkv_path))
                    replaced = True

            if not replaced:
                out_lines.append(line)

    with open(args.output, "w", encoding="utf-8") as fo:
        fo.writelines(out_lines)
    return 0


if __name__ == "__main__":
    sys.exit(main())
