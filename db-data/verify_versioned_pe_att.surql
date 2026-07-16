-- ============================================================
-- specs/022 verify: sesno_version_anchor + PE VERSION time-travel
-- 前置：fork surreal 以 versioned=true 启动（见 run_surrealkv_versioned.ps1）
-- 连接：surreal sql --endpoint http://127.0.0.1:8030 --user root --pass root --ns test --db anchor_verify
-- 目的：验证 write_sesno_version_anchor 发出的“确切 SQL 串”与
--       ensure_sesno_version_anchor_schema 的 DDL 在 fork 上成立，且锚点时间戳
--       能驱动 VERSION 时间旅行拿到改前快照（T012/T019 证据）。
-- ============================================================

USE NS test DB anchor_verify;

-- ---- 1. 锚点表 schema（逐字复制自 ensure_sesno_version_anchor_schema）----
DEFINE TABLE IF NOT EXISTS sesno_version_anchor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE sesno_version_anchor TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS source ON TABLE sesno_version_anchor TYPE string ASSERT $value IN ['full', 'incremental'];
DEFINE FIELD IF NOT EXISTS note ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_sesno_version_anchor_dbnum_sesno ON TABLE sesno_version_anchor FIELDS dbnum, sesno UNIQUE;

DEFINE TABLE IF NOT EXISTS pe TYPE NORMAL SCHEMALESS;

-- ---- 2. sesno=1：建 PE 初始态 + 写增量锚点（逐字复制 write_sesno_version_anchor 的 UPSERT）----
RETURN "====== sesno=1 建初始态 ======";
UPSERT pe:equi_001 CONTENT { name: '/EQUI-PUMP-A01', noun: 'EQUI', dbnum: 7997, sesno: 1, attrs: { BORE: 150 } };
UPSERT sesno_version_anchor:[7997, 1] SET dbnum = 7997, sesno = 1, source = 'incremental', anchored_at = time::now() RETURN anchored_at;

SLEEP 1500ms;

-- ---- 3. sesno=2：改 PE（BORE 150→200）+ 写第二个锚点 ----
RETURN "====== sesno=2 修改 BORE 150->200 ======";
UPSERT pe:equi_001 MERGE { sesno: 2, attrs: { BORE: 200 } };
UPSERT sesno_version_anchor:[7997, 2] SET dbnum = 7997, sesno = 2, source = 'incremental', anchored_at = time::now() RETURN anchored_at;

SLEEP 1500ms;

-- ---- 4. sesno=3：硬 DELETE（验证删除前状态仍可 VERSION 查回）----
RETURN "====== sesno=3 硬 DELETE equi_001 + full 锚点 ======";
DELETE pe:equi_001;
UPSERT sesno_version_anchor:[7997, 3] SET dbnum = 7997, sesno = 3, source = 'full', anchored_at = time::now() RETURN anchored_at;

-- ---- 5. 核对锚点表：应 3 行，(dbnum,sesno) 唯一，anchored_at 递增 ----
RETURN ">>> [ASSERT] 锚点表全部记录（期望 3 行，sesno 1/2/3，source incremental/incremental/full）:";
SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor ORDER BY sesno;

-- ---- 6. 时间旅行：必须在任何锚点“覆盖重写”之前执行 ----
-- ⚠️ 顺序陷阱：锚点是定长 record id 的 UPSERT，重跑会把 anchored_at 刷新到“当下”。
-- 若在硬删除之后重跑 sesno=1 锚点，其时间戳会被推到删除之后，用它做 VERSION 反查
-- 就会落在删除态而返回空——这是脚本编排问题，不是引擎/代码问题。故时间旅行读取
-- 必须先做（此时各锚点时间戳仍是各自 sesno 落库当刻）。
RETURN ">>> [ASSERT] 用 sesno=1 锚点时间戳 VERSION 查 PE（期望 BORE=150，改前态）:";
LET $t1 = (SELECT VALUE anchored_at FROM ONLY sesno_version_anchor:[7997, 1]);
SELECT name, sesno, attrs.BORE AS bore FROM pe:equi_001 VERSION $t1;

RETURN ">>> [ASSERT] 用 sesno=2 锚点时间戳 VERSION 查 PE（期望 BORE=200，删前态）:";
LET $t2 = (SELECT VALUE anchored_at FROM ONLY sesno_version_anchor:[7997, 2]);
SELECT name, sesno, attrs.BORE AS bore FROM pe:equi_001 VERSION $t2;

RETURN ">>> [ASSERT] 当前态 equi_001（期望空/已删除）:";
SELECT name, sesno, attrs.BORE AS bore FROM pe:equi_001;

-- ---- 7. 幂等：最后再重跑 sesno=1 锚点，行数仍应为 3（UPSERT 到定长 id 覆盖，不新增）----
-- 放在时间旅行之后，避免刷新 sesno=1 的 anchored_at 干扰上面的 VERSION 读取。
RETURN ">>> [ASSERT] 幂等重跑 sesno=1（覆盖，不新增；放在时间旅行之后）:";
UPSERT sesno_version_anchor:[7997, 1] SET dbnum = 7997, sesno = 1, source = 'incremental', anchored_at = time::now() RETURN anchored_at;
RETURN ">>> 幂等后锚点行数（期望仍为 3）:";
SELECT count() FROM sesno_version_anchor GROUP ALL;

RETURN "====== 022 anchor + VERSION 验证脚本结束 ======";
