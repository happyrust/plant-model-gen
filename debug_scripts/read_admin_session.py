import sqlite3

con = sqlite3.connect(r"D:\work\plant-code\plant-model-gen-cata-closure\deployment_sites.sqlite")
rows = list(
    con.execute(
        "SELECT token, username, role, expires_at FROM admin_sessions ORDER BY expires_at DESC LIMIT 3"
    )
)
for r in rows:
    print(r)
