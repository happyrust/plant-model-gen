import sqlite3

con = sqlite3.connect(
    r"D:\work\plant-code\plant-model-gen-cata-closure\dist\package\Plant3D-AIOS-win-x64\release\deployment_sites.sqlite"
)
rows = list(
    con.execute(
        "SELECT token, expires_at FROM admin_sessions ORDER BY expires_at DESC LIMIT 2"
    )
)
for r in rows:
    print(r[0], r[1])
