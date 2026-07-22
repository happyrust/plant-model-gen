-- ============================================================
-- specs/022 verify: 不可变锚点 fingerprint 幂等/冲突语义
-- 对照 src/versioned_db/version_commit.rs 的 existing_idempotent_outcome / create_immutable_anchor
-- 前置：127.0.0.1:8030 versioned 实例（db-data/run_surrealkv_versioned.ps1）
-- 连接：surreal sql --endpoint http://127.0.0.1:8030 --user root --pass root --ns vc_verify --db anchor_fp
-- 隔离：独立 ns=vc_verify db=anchor_fp，dbnum 用 9201 假编号，不碰其它 db
-- 注意：交互式 shell 按行解析，每条语句写成单行；RETURN 标签保持 ASCII 避免控制台编码问题
-- ============================================================

USE NS vc_verify DB anchor_fp;

RETURN "====== [0] schema: verbatim ensure_sesno_version_anchor_schema (database.rs) + sesno_version_anchor option fields from ensure_version_commit_schema (version_commit.rs, incl. specs/023 pe_owner_rows) ======";
DEFINE TABLE IF NOT EXISTS sesno_version_anchor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE sesno_version_anchor TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS source ON TABLE sesno_version_anchor TYPE string ASSERT $value IN ['full', 'incremental'];
DEFINE FIELD IF NOT EXISTS note ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE INDEX IF NOT EXISTS idx_sesno_version_anchor_dbnum_sesno ON TABLE sesno_version_anchor FIELDS dbnum, sesno UNIQUE;
DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS fingerprint ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS source_observation_hash ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS pe_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS att_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS uda_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS delete_count ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS dbnum_info_updates ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS pe_owner_rows ON TABLE sesno_version_anchor TYPE option<int>;

-- fixture 可重复执行：只清本验证专用的 9201 假 dbnum
DELETE sesno_version_anchor WHERE dbnum = 9201;

RETURN "====== [1] create-once: CREATE ONLY first succeeds, same id re-CREATE errors (anchor is immutable) ======";
RETURN ">>> [1a] first CREATE ONLY sesno_version_anchor:[9201,10] fingerprint='fp-A' (expect success, RETURN anchored_at):";
CREATE ONLY sesno_version_anchor:[9201, 10] SET dbnum = 9201, sesno = 10, from_sesno = 9, source = 'incremental', fingerprint = 'fp-A', source_observation_hash = NONE, pe_rows = 12, att_rows = 34, uda_rows = 5, delete_count = 2, dbnum_info_updates = 1, pe_owner_rows = 7, anchored_at = time::now() RETURN anchored_at;
RETURN ">>> [1b-ERR] re-CREATE ONLY same id with fingerprint='fp-B' (expect error: record already exists => anchor cannot be overwritten):";
CREATE ONLY sesno_version_anchor:[9201, 10] SET dbnum = 9201, sesno = 10, from_sesno = 9, source = 'incremental', fingerprint = 'fp-B', source_observation_hash = NONE, pe_rows = 99, att_rows = 99, uda_rows = 99, delete_count = 99, dbnum_info_updates = 99, pe_owner_rows = 99, anchored_at = time::now() RETURN anchored_at;
RETURN ">>> [1c] anchor not overwritten (expect still fingerprint='fp-A', pe_rows=12, pe_owner_rows=7):";
SELECT dbnum, sesno, fingerprint, pe_rows, pe_owner_rows FROM sesno_version_anchor:[9201, 10];

RETURN "====== [2] idempotent read-back: verbatim SELECT projection of existing_idempotent_outcome ======";
RETURN ">>> [2a] SELECT on [9201,10] (expect 1 row: fingerprint='fp-A', anchored_at as string, counts 12/34/5/2/1/7 incl. specs/023 pe_owner_rows):";
SELECT fingerprint, type::string(anchored_at) AS anchored_at, pe_rows, att_rows, uda_rows, delete_count, dbnum_info_updates, pe_owner_rows FROM sesno_version_anchor:[9201, 10];
RETURN ">>> [2b] same SELECT on missing [9201,99] (expect empty array => Rust side returns None, proceeds as fresh commit):";
SELECT fingerprint, type::string(anchored_at) AS anchored_at, pe_rows, att_rows, uda_rows, delete_count, dbnum_info_updates, pe_owner_rows FROM sesno_version_anchor:[9201, 99];

RETURN "====== [3] legacy anchor: no fingerprint field => read-back NONE => LegacyAnchor rejection path ======";
RETURN ">>> [3a] create legacy anchor [9201,20] with only dbnum/sesno/anchored_at/source (expect success, record has no fingerprint):";
CREATE ONLY sesno_version_anchor:[9201, 20] SET dbnum = 9201, sesno = 20, anchored_at = time::now(), source = 'full';
RETURN ">>> [3b] existing_idempotent_outcome projection on [9201,20] (expect fingerprint NONE/absent, counts all NONE):";
SELECT fingerprint, type::string(anchored_at) AS anchored_at, pe_rows, att_rows, uda_rows, delete_count, dbnum_info_updates, pe_owner_rows FROM sesno_version_anchor:[9201, 20];
RETURN ">>> [3c] explicit NONE check via ?? (expect 'FINGERPRINT_IS_NONE'):";
RETURN sesno_version_anchor:[9201, 20].fingerprint ?? 'FINGERPRINT_IS_NONE';

RETURN "====== [4] unique index idx_sesno_version_anchor_dbnum_sesno ======";
RETURN ">>> [4a] INFO FOR TABLE (expect indexes to contain idx_sesno_version_anchor_dbnum_sesno FIELDS dbnum, sesno UNIQUE):";
INFO FOR TABLE sesno_version_anchor;
RETURN ">>> [4b-ERR] different record id [9201,21] but duplicate field pair (dbnum,sesno)=(9201,20) (expect rejected by unique index):";
CREATE sesno_version_anchor:[9201, 21] SET dbnum = 9201, sesno = 20, source = 'full', fingerprint = 'fp-dup', anchored_at = time::now();
RETURN ">>> [4c] duplicate did not land (expect exactly 2 rows for dbnum=9201: sesno 10 and 20):";
SELECT sesno, fingerprint FROM sesno_version_anchor WHERE dbnum = 9201 ORDER BY sesno ASC;

RETURN "====== verify_022_anchor_fp end ======";
