"""BRAN tubi 缺失修复：给 quicktest-250160-8080 挂 AvevaCatalogue library 工程。

根因：SPRE 引用指向 AvevaCatalogue 的 acp7000_0001 等元件库（ref0 15192=dbnum 7000），
单工程配置下 db_index 未扫描 acp 文件 → 闭包 missing=44 → LSTU→CATR axis_candidates=0
→ 全站 16 个 BRAN 零 tubi。
"""

import json
import sqlite3
import urllib.request

con = sqlite3.connect(
    r"D:\work\plant-code\plant-model-gen-cata-closure\dist\package"
    r"\Plant3D-AIOS-win-x64\release\deployment_sites.sqlite"
)
token = next(
    con.execute("SELECT token FROM admin_sessions ORDER BY expires_at DESC LIMIT 1")
)[0]

payload = {
    "projects": [
        {
            "name": "AvevaPlantSample",
            "path": "\\\\?\\D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample",
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
    "http://127.0.0.1:3101/api/admin/sites/quicktest-250160-8080",
    data=json.dumps(payload).encode("utf-8"),
    headers={
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/json",
    },
    method="PUT",
)
with urllib.request.urlopen(req) as resp:
    body = json.loads(resp.read())
print(body.get("message"))
for p in body.get("data", {}).get("projects", []):
    print(p.get("name"), p.get("role"), p.get("is_primary"), p.get("path"))
