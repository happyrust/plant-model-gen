"""BRAN 模型生成依赖数据的全量字段级核对（金标准 = 8020 老库）。

范围：BRAN 24381_145018 + 17 子件（DESI 侧）+ 其 precise 口径 CATA 依赖闭包
（SPCO/SPEC/SCOM/GMSE 基元/PTSE 点/文本）。
逐元素逐字段对比 8020 与按需库：
- 缺失元素（按需库无此 pe）
- 字段丢失（8020 有值、按需为 None/空/截断）按 noun.FIELD 聚合
- 无害差异（dbnum 语义修正、DBNUM 补充等）单独归类
"""

import json
import re
import base64
import urllib.request
from collections import Counter, defaultdict

OLD = {"url": "http://127.0.0.1:8020/sql", "ns": "1516", "db": "AvevaMarineSample", "user": "root", "pass": "root"}
NEW = {"url": "http://127.0.0.1:8031/sql", "ns": "1", "db": "AvevaMarineSample", "user": "root", "pass": "root"}

BRAN = "24381_145018"
CONTAINER_NOUNS = {"GMSE", "NGMS", "PTSE", "PSTR", "SPRO", "DTSE"}
REF_RE = re.compile(r"^(?:\w+):[`⟨]?(\d+_\d+)[`⟩]?$")
# 无害/预期差异字段（语义修正而非丢失）
BENIGN_FIELDS = {"DBNUM", "dbnum", "SESNO", "id"}


def sql(ep, query):
    req = urllib.request.Request(ep["url"], data=query.encode(), method="POST")
    req.add_header(
        "Authorization",
        "Basic " + base64.b64encode((ep["user"] + ":" + ep["pass"]).encode()).decode(),
    )
    req.add_header("Accept", "application/json")
    req.add_header("surreal-ns", ep["ns"])
    req.add_header("surreal-db", ep["db"])
    req.add_header("NS", ep["ns"])
    req.add_header("DB", ep["db"])
    with urllib.request.urlopen(req, timeout=120) as r:
        body = json.load(r)
    f = body[0] if isinstance(body, list) else body
    if f.get("status") not in (None, "OK"):
        raise RuntimeError(f"SQL failed: {f}")
    return f.get("result") or []


def extract_refs(value, out):
    if isinstance(value, str):
        m = REF_RE.match(value)
        if m and m.group(1) != "0_0":
            out.add(m.group(1))
    elif isinstance(value, list):
        for v in value:
            extract_refs(v, out)
    elif isinstance(value, dict):
        for v in value.values():
            extract_refs(v, out)


def fetch_docs(ep, ids):
    docs = {}
    for i in range(0, len(ids), 150):
        chunk = ids[i : i + 150]
        keys = ",".join(f"pe:`{r}`" for r in chunk)
        rows = sql(ep, f"SELECT <string>id AS id, <string>noun AS noun, children, refno.* AS att FROM [{keys}];")
        for row in rows:
            m = REF_RE.match(row.get("id") or "")
            if m:
                docs[m.group(1)] = row
    return docs


# ── 1. 8020 构建 BRAN 生成依赖闭包（DESI 子件 + precise CATA 闭包）──
visited = {}
# 种子：BRAN + 直接子件
bran_doc = fetch_docs(OLD, [BRAN])[BRAN]
children = set()
extract_refs(bran_doc.get("children"), children)
seeds = [BRAN] + sorted(children)
frontier = list(seeds)
while frontier:
    batch = [r for r in frontier if r not in visited]
    if not batch:
        break
    docs = fetch_docs(OLD, batch)
    frontier = []
    for rid in batch:
        row = docs.get(rid)
        visited[rid] = row
        if row is None:
            continue
        refs = set()
        for field, value in (row.get("att") or {}).items():
            if field in ("REFNO", "OWNER"):
                continue
            extract_refs(value, refs)
        if (row.get("noun") or "").upper() in CONTAINER_NOUNS:
            extract_refs(row.get("children"), refs)
        # 跟 CATA 库引用；DESI 子件本身已在种子
        frontier.extend(r for r in refs if r.startswith("13246_"))

deps = sorted(r for r, v in visited.items() if v is not None)
noun_of = {r: (visited[r].get("noun") or "?") for r in deps}
print(f"8020 金标准：BRAN 生成依赖闭包 = {len(deps)} 个元素")
print(f"  noun 分布: {dict(Counter(noun_of.values()).most_common(15))}")

# ── 2. 按需库核对 ──────────────────────────────────────────────────────
new_docs = fetch_docs(NEW, deps)
missing = [r for r in deps if r not in new_docs]
print(f"\n[元素级] 按需库缺失: {len(missing)}")
for r in missing[:10]:
    print(f"  - {r} ({noun_of[r]})")


def is_empty(v):
    return v is None or v == "" or v == [] or v == {}


# ── 3. 字段级聚合对比 ──────────────────────────────────────────────────
lost = defaultdict(list)  # (noun, field) -> [(rid, old_val)]
typed = defaultdict(list)  # 类型/值改变（双方都有值但不同）
for r in deps:
    if r not in new_docs:
        continue
    a = visited[r].get("att") or {}
    b = new_docs[r].get("att") or {}
    noun = noun_of[r]
    for field in set(a) | set(b):
        if field in BENIGN_FIELDS:
            continue
        va, vb = a.get(field), b.get(field)
        if va == vb:
            continue
        if not is_empty(va) and is_empty(vb):
            lost[(noun, field)].append((r, va))
        elif isinstance(va, list) and isinstance(vb, list) and len(vb) < len(va):
            lost[(noun, field)].append((r, f"{va} -> 截断 {vb}"))
        else:
            typed[(noun, field)].append((r, va, vb))

print(f"\n[字段级] 丢失/截断（8020 有值 → 按需为空或截断）: {len(lost)} 类")
for (noun, field), items in sorted(lost.items(), key=lambda kv: -len(kv[1])):
    sample_r, sample_v = items[0]
    print(f"  {noun}.{field}: {len(items)} 个元素  样例 {sample_r} = {sample_v!r}")

print(f"\n[字段级] 值/类型差异（双方有值但不同）: {len(typed)} 类")
for (noun, field), items in sorted(typed.items(), key=lambda kv: -len(kv[1]))[:15]:
    sample_r, va, vb = items[0]
    print(f"  {noun}.{field}: {len(items)} 个  样例 {sample_r}: 8020={va!r} ondemand={vb!r}")

total_lost = sum(len(v) for v in lost.values())
print(f"\n总计：元素缺失 {len(missing)}，字段丢失 {total_lost} 处（{len(lost)} 类）")
print("结论:", "PASS — 元素与字段均完整" if not missing and not lost else "FAIL — 存在解析缺口（字段级）")
