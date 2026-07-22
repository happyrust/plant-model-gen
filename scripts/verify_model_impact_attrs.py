#!/usr/bin/env python3
"""只读校验：model_impact.rs 的几何影响属性白名单 vs 运行库权威属性字典 att_meta。

用途（对齐 AGENTS.md：aios-database 用 CLI+JSON 验证，不写 cargo test）：
  1. 从 src/version_management/model_impact.rs 解析 attribute_affects_model 的白名单；
  2. 查询运行中的 SurrealDB att_meta（权威属性字典，名+dabacon 哈希）；
  3. 报告：白名单里"形似 dabacon 名却不在 att_meta"的项（typo/伪属性守卫）、
     §13.2 几何输入属性对白名单的覆盖率（应 100%）、本次补入缺口在 att_meta 的命中。
  输出 JSON 到 stdout（可 > 存证）。

环境变量（可选）：SURREAL_URL(默认 http://127.0.0.1:8020/sql)、SURREAL_NS(1516)、
  SURREAL_DB(AvevaMarineSample)、SURREAL_AUTH_B64(root:root 的 base64=cm9vdDpyb290)。
"""
import base64
import json
import os
import re
import sys
from pathlib import Path

try:
    import requests
except Exception:
    print(json.dumps({"error": "requests not available"}, ensure_ascii=False))
    sys.exit(2)

REPO = Path(__file__).resolve().parents[1]
SRC = REPO / "src" / "version_management" / "model_impact.rs"

# §13.2 几何输入属性（逆向文档，post-correction；派生项 XDIR/RAD 已剔除）
GEOM_13_2 = set("""POS POSL POSS POSE NPOS CPOS ORI YDIR ZDIR PAXI PZAXI PLAX ARRI LEAV BANG
CATR SPRE CREF HREF TREF PSPE NGMR GTYP CTYP DESP DELP PARA RINS OPDI UNIPAR
HEIG ANGL RADI DIAM PRAD PWID PHEI PDIA PBDM PTDM PDIS PBDI PTDI PXTS PYTS PXBS PYBS PXLE PX PY PZ DX DY
ZDIS ROUT DRNS DRNE CURD CURTYP DETR JUSL SJUS JLIN JFRE DTRE DKEY DPRO PPRO PTYP PSTR PKEY PKDI""".split())

# 本次 M1 补入的缺口（用于确认命中 att_meta）
NEW_GAPS = "PX PY PZ DX DY POSL POSS POSE NPOS CPOS BANG ZDIS LEAV CURD CURTYP OPDI ROUT DRNS DRNE PSPE CTYP JFRE JLIN DELP RINS PKDI".split()


def parse_allowlist(src_text: str) -> set:
    """提取 attribute_affects_model 中 matches!(name.as_str(), ...) 块内的大写字面量。"""
    m = re.search(r"matches!\(\s*name\.as_str\(\),(.*?)\n\s*\)\s*\|\|", src_text, re.S)
    if not m:
        raise RuntimeError("未能定位 attribute_affects_model 的 matches! 块")
    block = m.group(1)
    return set(re.findall(r'"([A-Z][A-Z0-9]{1,})"', block))


def fetch_att_meta():
    url = os.environ.get("SURREAL_URL", "http://127.0.0.1:8020/sql")
    ns = os.environ.get("SURREAL_NS", "1516")
    db = os.environ.get("SURREAL_DB", "AvevaMarineSample")
    auth = os.environ.get("SURREAL_AUTH_B64", base64.b64encode(b"root:root").decode())
    headers = {"Accept": "application/json", "Authorization": f"Basic {auth}",
               "surreal-ns": ns, "surreal-db": db, "NS": ns, "DB": db,
               "Content-Type": "text/plain"}
    r = requests.post(url, headers=headers,
                      data=b"SELECT record::id(id) AS name, hash FROM att_meta;", timeout=60)
    r.raise_for_status()
    rows = r.json()[0].get("result", [])
    return {str(x["name"]): x.get("hash") for x in rows if x.get("name")}


def main():
    src_text = SRC.read_text(encoding="utf-8")
    allow = parse_allowlist(src_text)

    report = {"allowlist_count": len(allow)}
    try:
        name2hash = fetch_att_meta()
        report["att_meta_count"] = len(name2hash)
        report["att_meta_available"] = True
    except Exception as e:
        report["att_meta_available"] = False
        report["att_meta_error"] = f"{type(e).__name__}: {e}"
        name2hash = {}

    if name2hash:
        # 白名单里不在 att_meta 的项（结构/伪属性 or 潜在 typo；供人工判读）
        not_in = sorted(a for a in allow if a not in name2hash)
        report["allowlist_not_in_att_meta"] = not_in
        report["allowlist_in_att_meta_count"] = len(allow) - len(not_in)
        # §13.2 覆盖：应被白名单 100% 覆盖
        missing_13_2 = sorted(a for a in GEOM_13_2 if a not in allow)
        report["geom_13_2_not_in_allowlist"] = missing_13_2
        report["geom_13_2_coverage_ok"] = (len(missing_13_2) == 0)
        # 本次补入缺口命中 att_meta（真实 dabacon 属性带 hash）
        gaps = {}
        for a in NEW_GAPS:
            h = name2hash.get(a)
            gaps[a] = {"in_allowlist": a in allow,
                       "in_att_meta": h is not None,
                       "hash": (hex(h) if isinstance(h, int) else None)}
        report["new_gaps"] = gaps
        report["new_gaps_all_in_allowlist"] = all(a in allow for a in NEW_GAPS)

    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
