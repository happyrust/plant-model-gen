-- ============================================================
-- specs/022+023 verify: version_commit_state 状态机 + per-dbnum lease + 不可变锚点
--（含 specs/023 新增 pe_owner_rows 计数字段与旧写入方 DEFAULT 0 兼容）
-- 前置：fork surreal 以 versioned=true 启动（run_version_commit_state_verify.ps1，端口 8034）
-- 连接：surreal sql --endpoint http://127.0.0.1:8034 --user root --pass root --ns test --db commit_verify
-- 目的：把 src/versioned_db/version_commit.rs 的 ensure_version_commit_schema DDL 与关键 SQL 串
--       （preparing→pending→committed / pending gate / lease THROW / CREATE ONLY 不可变锚点）
--       逐字放到 fork 上验证契约成立。深模块本身走 commit_version/recover_version_commit 单入口，
--       这里固化它内部发出的“确切 SQL”在引擎侧的行为，作为 SurrealQL 级证据。
-- 注意：交互式 shell 按行解析，故每条语句（含事务）写成单行。
-- ============================================================

USE NS test DB commit_verify;

-- ---- 0a. 锚点基表 schema（逐字复制自 database.rs ensure_sesno_version_anchor_schema；ensure_version_commit_schema 首先调用它）----
DEFINE TABLE IF NOT EXISTS sesno_version_anchor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE sesno_version_anchor TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS source ON TABLE sesno_version_anchor TYPE string ASSERT $value IN ['full', 'incremental'];
DEFINE FIELD IF NOT EXISTS note ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_sesno_version_anchor_dbnum_sesno ON TABLE sesno_version_anchor FIELDS dbnum, sesno UNIQUE;

-- ---- 0b. version_commit schema（逐字复制自 ensure_version_commit_schema，含 023 pe_owner_rows）----
DEFINE TABLE IF NOT EXISTS version_commit_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS to_sesno ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS source ON TABLE version_commit_state TYPE string ASSERT $value IN ['full', 'incremental'];
DEFINE FIELD IF NOT EXISTS fingerprint ON TABLE version_commit_state TYPE string;
DEFINE FIELD IF NOT EXISTS source_observation_hash ON TABLE version_commit_state TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status ON TABLE version_commit_state TYPE string ASSERT $value IN ['preparing', 'pending', 'committed'];
DEFINE FIELD IF NOT EXISTS pe_rows ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS att_rows ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS uda_rows ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS delete_count ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS dbnum_info_updates ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS pe_owner_rows ON TABLE version_commit_state TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE version_commit_state TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS last_error ON TABLE version_commit_state TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE version_commit_state TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE version_commit_state TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_version_commit_state_dbnum_status ON TABLE version_commit_state FIELDS dbnum, status;
DEFINE TABLE IF NOT EXISTS version_commit_lease SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE version_commit_lease TYPE int;
DEFINE FIELD IF NOT EXISTS owner ON TABLE version_commit_lease TYPE string;
DEFINE FIELD IF NOT EXISTS expires_at ON TABLE version_commit_lease TYPE datetime;
DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS fingerprint ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS source_observation_hash ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS pe_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS att_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS uda_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS delete_count ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS dbnum_info_updates ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS pe_owner_rows ON TABLE sesno_version_anchor TYPE option<int>;

-- fixture 可重复执行：清空当前态
DELETE version_commit_state;
DELETE version_commit_lease;
DELETE sesno_version_anchor;

RETURN "====== [1] version_commit_state 状态机 preparing->pending->committed ======";
UPSERT version_commit_state:[7997, 5] SET dbnum = 7997, from_sesno = 4, to_sesno = 5, source = 'incremental', fingerprint = 'fp-dbnum7997-sesno5', source_observation_hash = NONE, status = 'preparing', pe_rows = 0, att_rows = 0, uda_rows = 0, delete_count = 0, dbnum_info_updates = 0, pe_owner_rows = 0, anchored_at = NONE, last_error = NONE, updated_at = time::now();
RETURN ">>> [ASSERT] 期望 status=preparing, pe_owner_rows=0:";
SELECT dbnum, to_sesno, status, fingerprint, pe_owner_rows FROM version_commit_state:[7997, 5];
UPDATE version_commit_state:[7997, 5] SET status = 'pending', last_error = 'apply failed: injected', updated_at = time::now();
RETURN ">>> [ASSERT] 期望 status=pending, last_error 非空:";
SELECT dbnum, to_sesno, status, last_error FROM version_commit_state:[7997, 5];
UPSERT version_commit_state:[7997, 5] SET dbnum = 7997, from_sesno = 4, to_sesno = 5, source = 'incremental', fingerprint = 'fp-dbnum7997-sesno5', source_observation_hash = NONE, status = 'committed', pe_rows = 2, att_rows = 2, uda_rows = 1, delete_count = 0, dbnum_info_updates = 1, pe_owner_rows = 3, anchored_at = <datetime>'2026-07-19T00:00:00Z', last_error = NONE, updated_at = time::now();
RETURN ">>> [ASSERT] 期望 status=committed, pe_rows=2, pe_owner_rows=3, last_error=NONE:";
SELECT dbnum, to_sesno, status, pe_rows, att_rows, uda_rows, pe_owner_rows, last_error FROM version_commit_state:[7997, 5];
RETURN ">>> [ASSERT-ERR] 写入非法 status='booted' 应被 ASSERT 拒绝（期望报错）:";
UPDATE version_commit_state:[7997, 5] SET status = 'booted';
RETURN ">>> [ASSERT] 023 兼容：旧写入方 UPSERT 不带 pe_owner_rows（DEFAULT 0 兜底，期望 pe_owner_rows=0）:";
UPSERT version_commit_state:[7997, 99] SET dbnum = 7997, from_sesno = 99, to_sesno = 99, source = 'incremental', fingerprint = 'fp-legacy-writer', source_observation_hash = NONE, status = 'preparing', pe_rows = 0, att_rows = 0, uda_rows = 0, delete_count = 0, dbnum_info_updates = 0, anchored_at = NONE, last_error = NONE, updated_at = time::now();
SELECT to_sesno, status, pe_owner_rows FROM version_commit_state:[7997, 99];
DELETE version_commit_state:[7997, 99];

RETURN "====== [2] pending gate（reject_pending_commit 查询）======";
UPSERT version_commit_state:[7997, 6] SET dbnum = 7997, from_sesno = 6, to_sesno = 6, source = 'incremental', fingerprint = 'fp-dbnum7997-sesno6', source_observation_hash = NONE, status = 'preparing', pe_rows = 0, att_rows = 0, uda_rows = 0, delete_count = 0, dbnum_info_updates = 0, pe_owner_rows = 0, anchored_at = NONE, last_error = NONE, updated_at = time::now();
RETURN ">>> [ASSERT] 非 recover 路径 gate（committed 的 5 不算，pending/preparing 只剩 6；期望命中 to_sesno=6，说明请求 sesno=7 被阻断）:";
SELECT to_sesno FROM version_commit_state WHERE dbnum = 7997 AND status IN ['preparing', 'pending'] ORDER BY to_sesno ASC LIMIT 1;
RETURN ">>> [ASSERT] recover 路径（AND fingerprint != 本次）——用 sesno=6 自身 fingerprint 应放行（期望空数组）:";
SELECT to_sesno FROM version_commit_state WHERE dbnum = 7997 AND status IN ['preparing', 'pending'] AND fingerprint != 'fp-dbnum7997-sesno6' ORDER BY to_sesno ASC LIMIT 1;
RETURN ">>> [ASSERT] require_matching_pending 恢复查找（期望 1 行 fingerprint=fp-dbnum7997-sesno6, status=preparing）:";
SELECT fingerprint, status FROM version_commit_state:[7997, 6];
UPDATE version_commit_state:[7997, 6] SET status = 'committed', anchored_at = <datetime>'2026-07-19T00:00:01Z';

RETURN "====== [3] per-dbnum lease（acquire_dbnum_lease 事务）======";
RETURN ">>> [ASSERT] ownerA 首次获取租约（无既有）→ 期望 owner=ownerA-1:";
BEGIN TRANSACTION; LET $ownerA = "ownerA-1"; LET $current = (SELECT owner, expires_at FROM version_commit_lease:7997); IF array::len($current) > 0 AND $current[0].expires_at > time::now() AND $current[0].owner != $ownerA { THROW "VERSION_COMMIT_LEASE_BUSY"; }; UPSERT version_commit_lease:7997 SET dbnum = 7997, owner = $ownerA, expires_at = time::now() + duration::from_secs(900); COMMIT TRANSACTION;
SELECT dbnum, owner FROM version_commit_lease:7997;
RETURN ">>> [ASSERT-ERR] ownerB 抢占未过期租约（期望 THROW VERSION_COMMIT_LEASE_BUSY）:";
BEGIN TRANSACTION; LET $ownerB = "ownerB-2"; LET $current = (SELECT owner, expires_at FROM version_commit_lease:7997); IF array::len($current) > 0 AND $current[0].expires_at > time::now() AND $current[0].owner != $ownerB { THROW "VERSION_COMMIT_LEASE_BUSY"; }; UPSERT version_commit_lease:7997 SET dbnum = 7997, owner = $ownerB, expires_at = time::now() + duration::from_secs(900); COMMIT TRANSACTION;
RETURN ">>> [ASSERT] 抢占失败后仍是 ownerA-1:";
SELECT dbnum, owner FROM version_commit_lease:7997;
RETURN ">>> [ASSERT] ownerA 重入（幂等成功，仍 ownerA-1）:";
BEGIN TRANSACTION; LET $ownerA = "ownerA-1"; LET $current = (SELECT owner, expires_at FROM version_commit_lease:7997); IF array::len($current) > 0 AND $current[0].expires_at > time::now() AND $current[0].owner != $ownerA { THROW "VERSION_COMMIT_LEASE_BUSY"; }; UPSERT version_commit_lease:7997 SET dbnum = 7997, owner = $ownerA, expires_at = time::now() + duration::from_secs(900); COMMIT TRANSACTION;
SELECT dbnum, owner FROM version_commit_lease:7997;
UPDATE version_commit_lease:7997 SET expires_at = <datetime>'2000-01-01T00:00:00Z';
RETURN ">>> [ASSERT] 租约过期后 ownerB 接管（期望 owner=ownerB-2）:";
BEGIN TRANSACTION; LET $ownerB = "ownerB-2"; LET $current = (SELECT owner, expires_at FROM version_commit_lease:7997); IF array::len($current) > 0 AND $current[0].expires_at > time::now() AND $current[0].owner != $ownerB { THROW "VERSION_COMMIT_LEASE_BUSY"; }; UPSERT version_commit_lease:7997 SET dbnum = 7997, owner = $ownerB, expires_at = time::now() + duration::from_secs(900); COMMIT TRANSACTION;
SELECT dbnum, owner FROM version_commit_lease:7997;
RETURN ">>> [ASSERT] 非持有者 ownerA 释放（DELETE ... WHERE owner；期望租约仍在=ownerB-2）:";
DELETE version_commit_lease:7997 WHERE owner = 'ownerA-1';
SELECT dbnum, owner FROM version_commit_lease:7997;
RETURN ">>> [ASSERT] 持有者 ownerB 释放（期望租约清空，count=0/空）:";
DELETE version_commit_lease:7997 WHERE owner = 'ownerB-2';
SELECT count() FROM version_commit_lease GROUP ALL;

RETURN "====== [4] 不可变锚点 CREATE ONLY create-once（create_immutable_anchor）======";
RETURN ">>> [ASSERT] 首次 CREATE ONLY 锚点（期望成功、返回 anchored_at）:";
CREATE ONLY sesno_version_anchor:[7997, 5] SET dbnum = 7997, sesno = 5, from_sesno = 4, source = 'incremental', fingerprint = 'fp-dbnum7997-sesno5', source_observation_hash = NONE, pe_rows = 2, att_rows = 2, uda_rows = 1, delete_count = 0, dbnum_info_updates = 1, pe_owner_rows = 3, anchored_at = time::now() RETURN anchored_at;
RETURN ">>> [ASSERT-ERR] 重复 CREATE ONLY 同一 (dbnum,sesno)（期望报错：已存在，不可变）:";
CREATE ONLY sesno_version_anchor:[7997, 5] SET dbnum = 7997, sesno = 5, from_sesno = 4, source = 'incremental', fingerprint = 'fp-different-attempt', source_observation_hash = NONE, pe_rows = 9, att_rows = 9, uda_rows = 9, delete_count = 9, dbnum_info_updates = 9, pe_owner_rows = 9, anchored_at = time::now() RETURN anchored_at;
RETURN ">>> [ASSERT] 锚点仍是首个 fingerprint（未被第二次覆盖，pe_rows=2/pe_owner_rows=3 而非 9）:";
SELECT dbnum, sesno, fingerprint, pe_rows, pe_owner_rows FROM sesno_version_anchor:[7997, 5];
RETURN ">>> [ASSERT] existing_idempotent_outcome 读取（期望 1 行、fingerprint=fp-dbnum7997-sesno5、pe_owner_rows=3）:";
SELECT fingerprint, type::string(anchored_at) AS anchored_at, pe_rows, att_rows, uda_rows, delete_count, dbnum_info_updates, pe_owner_rows FROM sesno_version_anchor:[7997, 5];

RETURN "====== version_commit_state / lease / 不可变锚点 契约验证脚本结束 ======";
