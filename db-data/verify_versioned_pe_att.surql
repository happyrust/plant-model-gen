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
DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS fingerprint ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS source_observation_hash ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS pe_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS att_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS uda_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS delete_count ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS dbnum_info_updates ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE INDEX IF NOT EXISTS idx_sesno_version_anchor_dbnum_sesno ON TABLE sesno_version_anchor FIELDS dbnum, sesno UNIQUE;

DEFINE TABLE IF NOT EXISTS pe TYPE NORMAL SCHEMALESS;

-- 专用 fixture DB 可重复执行：清空当前态；versioned 引擎仍保留删除前底层版本。
DELETE sesno_version_anchor;
DELETE pe:equi_001;

-- ---- 2. sesno=1：建 PE 初始态 + create-once 增量锚点 ----
RETURN "====== sesno=1 建初始态 ======";
UPSERT pe:equi_001 CONTENT { name: '/EQUI-PUMP-A01', noun: 'EQUI', dbnum: 7997, sesno: 1, attrs: { BORE: 150 } };
CREATE ONLY sesno_version_anchor:[7997, 1] SET dbnum = 7997, sesno = 1, from_sesno = 1, source = 'incremental', fingerprint = 'fixture-sesno-1', pe_rows = 1, att_rows = 0, uda_rows = 0, delete_count = 0, dbnum_info_updates = 1, anchored_at = time::now() RETURN anchored_at;

SLEEP 1500ms;

-- ---- 3. sesno=2：改 PE（BORE 150→200）+ 写第二个锚点 ----
RETURN "====== sesno=2 修改 BORE 150->200 ======";
UPSERT pe:equi_001 MERGE { sesno: 2, attrs: { BORE: 200 } };
CREATE ONLY sesno_version_anchor:[7997, 2] SET dbnum = 7997, sesno = 2, from_sesno = 2, source = 'incremental', fingerprint = 'fixture-sesno-2', pe_rows = 1, att_rows = 0, uda_rows = 0, delete_count = 0, dbnum_info_updates = 1, anchored_at = time::now() RETURN anchored_at;

SLEEP 1500ms;

-- ---- 4. sesno=3：硬 DELETE（验证删除前状态仍可 VERSION 查回）----
RETURN "====== sesno=3 硬 DELETE equi_001 + full 锚点 ======";
DELETE pe:equi_001;
CREATE ONLY sesno_version_anchor:[7997, 3] SET dbnum = 7997, sesno = 3, from_sesno = 3, source = 'full', fingerprint = 'fixture-sesno-3', pe_rows = 0, att_rows = 0, uda_rows = 0, delete_count = 1, dbnum_info_updates = 1, anchored_at = time::now() RETURN anchored_at;

-- ---- 5. 核对锚点表：应 3 行，(dbnum,sesno) 唯一，anchored_at 递增 ----
RETURN ">>> [ASSERT] 锚点表全部记录（期望 3 行，sesno 1/2/3，source incremental/incremental/full）:";
SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor ORDER BY sesno;

-- ---- 6. 时间旅行：锚点 create-once，不允许重写 anchored_at ----
-- ---- 5b. 库内查找函数（与 ensure_sesno_version_lookup_functions / resolve_anchor 对齐）----
DEFINE FUNCTION OVERWRITE fn::sesno_version($dbnum: number, $sesno: number) { LET $exact = (SELECT VALUE anchored_at FROM sesno_version_anchor WHERE dbnum = $dbnum AND sesno = $sesno LIMIT 1); IF array::len($exact) > 0 { RETURN $exact[0]; }; LET $fb = (SELECT VALUE anchored_at FROM sesno_version_anchor WHERE dbnum = $dbnum AND sesno <= $sesno ORDER BY sesno DESC LIMIT 1); RETURN IF array::len($fb) > 0 { $fb[0] } ELSE { NONE }; };

DEFINE FUNCTION OVERWRITE fn::sesno_version_hit($dbnum: number, $sesno: number) { LET $exact = (SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor WHERE dbnum = $dbnum AND sesno = $sesno LIMIT 1); IF array::len($exact) > 0 { LET $r = $exact[0]; RETURN { dbnum: $r.dbnum, sesno: $r.sesno, source: $r.source, anchored_at: $r.anchored_at, exact: true }; }; LET $fb = (SELECT dbnum, sesno, source, anchored_at FROM sesno_version_anchor WHERE dbnum = $dbnum AND sesno <= $sesno ORDER BY sesno DESC LIMIT 1); IF array::len($fb) > 0 { LET $r = $fb[0]; RETURN { dbnum: $r.dbnum, sesno: $r.sesno, source: $r.source, anchored_at: $r.anchored_at, exact: false }; }; RETURN NONE; };

RETURN ">>> [ASSERT] fn::sesno_version / hit（期望 exact=true，sesno=1）:";
RETURN fn::sesno_version_hit(7997, 1);
RETURN ">>> [ASSERT] fn::sesno_version 回退（请求 sesno=1 与精确应相同）:";
RETURN fn::sesno_version(7997, 1);

RETURN ">>> [ASSERT] 用 sesno=1 锚点时间戳 VERSION 查 PE（期望 BORE=150，改前态）:";
LET $t1 = fn::sesno_version(7997, 1);
SELECT name, sesno, attrs.BORE AS bore FROM pe:equi_001 VERSION $t1;

RETURN ">>> [ASSERT] 用 sesno=2 锚点时间戳 VERSION 查 PE（期望 BORE=200，删前态）:";
LET $t2 = fn::sesno_version(7997, 2);
SELECT name, sesno, attrs.BORE AS bore FROM pe:equi_001 VERSION $t2;

RETURN ">>> [ASSERT] 当前态 equi_001（期望空/已删除）:";
SELECT name, sesno, attrs.BORE AS bore FROM pe:equi_001;

-- ---- 7. 幂等：相同 fingerprint 读取既有锚点，不执行 UPDATE/UPSERT ----
RETURN ">>> [ASSERT] 幂等重跑 sesno=1（命中同 fingerprint，anchored_at 不变）:";
LET $existing = (SELECT fingerprint, anchored_at FROM sesno_version_anchor:[7997, 1]);
RETURN IF array::len($existing) > 0 AND $existing[0].fingerprint = 'fixture-sesno-1' { $existing[0] } ELSE { NONE };
RETURN ">>> 幂等后锚点行数（期望仍为 3）:";
SELECT count() FROM sesno_version_anchor GROUP ALL;

RETURN "====== 022 anchor + VERSION 验证脚本结束 ======";
