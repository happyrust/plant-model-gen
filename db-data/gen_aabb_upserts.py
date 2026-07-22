import json
import re

rows = json.load(
    open(r"d:\work\plant-code\plant-model-gen\db-data\list_aabb_bran.json", encoding="utf-8")
)[0]["result"] or []
print("rows", len(rows))
seen = {}
for r in rows:
    rid = str(r["id"])
    ref = str(r["refno"])
    aabb = str(r["aabb_id"])
    m = re.search(r"pe:`([^`]+)`", ref)
    if not m:
        continue
    key = m.group(1)
    if rid.startswith("inst_relate_aabb:[") and rid.count(",") == 1:
        score = 2
    elif rid.startswith("inst_relate_aabb:["):
        score = 1
    else:
        score = 0
    if key not in seen or score > seen[key][2]:
        seen[key] = (aabb, rid, score)

out = []
for key, (aabb, src, _) in sorted(seen.items()):
    am = re.search(r"aabb:`([^`]+)`", aabb)
    if not am:
        continue
    ah = am.group(1)
    out.append(
        f"UPSERT inst_relate_aabb:`{key}` SET aabb_id = aabb:`{ah}`, aabb = aabb:`{ah}`, refno = pe:`{key}`;"
    )
    print(key, "<-", src)

path = r"d:\work\plant-code\plant-model-gen\db-data\upsert_aabb_string.surql"
open(path, "w", encoding="utf-8").write("\n".join(out) + "\n")
print("wrote", len(out), "upserts")
