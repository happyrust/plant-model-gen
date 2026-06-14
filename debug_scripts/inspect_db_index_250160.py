import sqlite3

DB = (
    r"D:\work\plant-code\plant-model-gen-cata-closure\dist\package"
    r"\Plant3D-AIOS-win-x64\release\runtime\admin_sites\quicktest-250160-8080"
    r"\output\AvevaPlantSample\scene_tree\db_index.sqlite"
)

con = sqlite3.connect(DB)
tables = [r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")]
print("tables:", tables)
for t in tables:
    cols = [c[1] for c in con.execute(f"PRAGMA table_info({t})")]
    cnt = con.execute(f"SELECT count(*) FROM {t}").fetchone()[0]
    print(f"-- {t} ({cnt} rows) cols={cols}")
    for row in con.execute(f"SELECT * FROM {t} LIMIT 5"):
        print("   ", row)

# 查 ref0=15192 的映射
for t in tables:
    cols = [c[1] for c in con.execute(f"PRAGMA table_info({t})")]
    if "ref0" in cols:
        rows = list(con.execute(f"SELECT * FROM {t} WHERE ref0=15192"))
        print(f"ref0=15192 in {t}:", rows)
