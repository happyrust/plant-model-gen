"""spec 006 T308 前置：修正 quicktest-7997-8080 的 projects 配置。

站点 projects 里混入了一条坏记录 {name: acp000, path: ...\AvevaCatalogue\acp000,
is_primary: true}：path 指向数据目录而非项目根（解析器会在其下再找 *000 目录，
报 "项目目录下未找到 000 数据目录" 退出 1），且与 AvevaCatalogue 重复并抢走主工程标记。
"""

import json
import urllib.request

TOKEN = "864010fd-8e8c-4813-a13b-847d785e8290"
BASE = "http://127.0.0.1:3111"
SITE = "quicktest-7997-8080"

payload = {
    "projects": [
        {
            "name": "AvevaMarineSample",
            "path": "\\\\?\\D:\\AVEVA\\Projects\\E3D2.1\\AvevaMarineSample",
            "role": "design",
            "is_primary": True,
            "sort_order": 0,
        },
        {
            "name": "AvevaCatalogue",
            "path": "\\\\?\\D:\\AVEVA\\Projects\\E3D2.1\\AvevaCatalogue",
            "role": "library",
            "is_primary": False,
            "sort_order": 1,
        },
    ]
}

req = urllib.request.Request(
    f"{BASE}/api/admin/sites/{SITE}",
    data=json.dumps(payload).encode("utf-8"),
    headers={
        "Authorization": f"Bearer {TOKEN}",
        "Content-Type": "application/json",
    },
    method="PUT",
)
with urllib.request.urlopen(req) as resp:
    body = json.loads(resp.read())
print(json.dumps(body.get("message"), ensure_ascii=False))
projects = body.get("data", {}).get("projects", [])
for p in projects:
    print(p.get("name"), p.get("role"), p.get("is_primary"), p.get("path"))
