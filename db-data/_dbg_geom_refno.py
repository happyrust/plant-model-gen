import json
import urllib.request
import base64

auth = "Basic " + base64.b64encode(b"root:root").decode()


def sq(q: str):
    req = urllib.request.Request(
        "http://127.0.0.1:8020/sql",
        data=q.encode("utf-8"),
        method="POST",
        headers={
            "Accept": "application/json",
            "Authorization": auth,
            "surreal-ns": "1516",
            "surreal-db": "AvevaMarineSample",
        },
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


print(
    json.dumps(
        sq(
            """
SELECT record::id(id) AS gid, record::id(out) AS geo, visible, record::id(trans) AS trans
FROM geo_relate:[24381, 145026, NONE]..=[24381, 145026, ..]
ORDER BY record::id(id)[2];
"""
        ),
        ensure_ascii=False,
        indent=2,
    )[:4000]
)
