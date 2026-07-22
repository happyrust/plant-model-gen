# verify_022_lease_gate — dbnum lease 串行 + PendingCommit 门禁（SurrealQL 实测证据）

- 日期：2026-07-19
- 实例：`http://127.0.0.1:8030`（`db-data/run_surrealkv_versioned.ps1` 启动的 versioned 实例，`GET /version` → `surrealdb-3.2.0-nightly`）
- 隔离环境：`ns=vc_verify`，主验证 `db=lease_gate`；schema 幂等补测 `db=lease_gate_fresh2`。dbnum 用假编号 9101/9102，不触碰其它成员在用的 db。
- 方法：HTTP `POST /sql`（Basic root/root + `surreal-ns`/`surreal-db` 头），SQL 逐字照搬 `src/versioned_db/version_commit.rs`（`acquire_dbnum_lease` / `reject_pending_commit` / `ensure_version_commit_schema`）与 `src/versioned_db/database.rs`（`ensure_sesno_version_anchor_schema` / `ensure_sesno_version_lookup_functions`）。
- 下文"返回"为服务端 JSON 的 status/result 摘录（略去 time 字段）；批量 DDL 只给逐语句 OK/ERR 计数与错误原文。

---

## 1. lease 互斥（对照 `acquire_dbnum_lease`，version_commit.rs L613-645）

### 1.1 fixture：owner='A' 持有未过期租约

```sql
DELETE version_commit_lease;
DELETE version_commit_state;
UPSERT version_commit_lease:9101 SET dbnum = 9101, owner = 'A', expires_at = time::now() + duration::from_secs(900);
SELECT dbnum, owner, type::string(expires_at) AS expires_at FROM version_commit_lease:9101;
```

返回（第 4 条）：

```json
[{"dbnum":9101,"expires_at":"2026-07-19T15:25:23.002091100Z","owner":"A"}]
```

### 1.2 owner='B' 模拟 acquire（照搬 acquire_dbnum_lease 事务，仅把绑定参数 $owner 改为事务内 LET）→ 预期 THROW

```sql
BEGIN TRANSACTION;
LET $owner = 'B';
LET $current = (SELECT owner, expires_at FROM version_commit_lease:9101);
IF array::len($current) > 0
    AND $current[0].expires_at > time::now()
    AND $current[0].owner != $owner {
    THROW "VERSION_COMMIT_LEASE_BUSY";
};
UPSERT version_commit_lease:9101 SET
    dbnum = 9101,
    owner = $owner,
    expires_at = time::now() + duration::from_secs(900);
COMMIT TRANSACTION;
```

返回（逐语句 status/result）：

```json
[
 {"status":"OK","result":null},
 {"status":"ERR","result":"The query was not executed due to a failed transaction"},
 {"status":"ERR","result":"The query was not executed due to a failed transaction"},
 {"status":"ERR","kind":"Thrown","result":"An error occurred: VERSION_COMMIT_LEASE_BUSY"},
 {"status":"ERR","result":"The query was not executed due to a cancelled transaction"},
 {"status":"ERR","result":"Cannot COMMIT: the transaction was aborted due to a prior error"}
]
```

事务被 THROW 中止，UPSERT 未执行。随查确认租约未被抢走：

```sql
SELECT dbnum, owner FROM version_commit_lease:9101;
-- => [{"dbnum":9101,"owner":"A"}]   (status OK)
```

Rust 侧 `acquire_dbnum_lease` 正是靠错误信息包含 `VERSION_COMMIT_LEASE_BUSY` 映射为 `VersionCommitError::LeaseBusy`（L637-641），与此处引擎行为一致。

### 1.3 同 owner='A' 重入 → 预期放行（幂等续租）

同一事务 SQL、`LET $owner = 'A'`。返回全部 OK，UPSERT 生效并刷新 expires_at：

```json
[..., {"status":"OK","result":[{"dbnum":9101,"expires_at":"2026-07-19T15:25:23.101401600Z","id":"version_commit_lease:9101","owner":"A"}]}, {"status":"OK"}]
```

（`$current[0].owner != $owner` 为 false → 不 THROW，验证互斥条件只挡"别的 owner"。）

### 1.4 过期后接管 → 预期成功

```sql
UPDATE version_commit_lease:9101 SET expires_at = <datetime>'2000-01-01T00:00:00Z';
```

返回 OK（owner 仍为 A，expires_at 已成过去时刻）。再执行 1.2 完全相同的 owner='B' 事务：

```json
[..., {"status":"OK","result":[{"dbnum":9101,"expires_at":"2026-07-19T15:25:23.157709900Z","id":"version_commit_lease:9101","owner":"B"}]}, {"status":"OK"}]
SELECT dbnum, owner FROM version_commit_lease:9101;
-- => [{"dbnum":9101,"owner":"B"}]
```

（`$current[0].expires_at > time::now()` 为 false → 不 THROW，B 接管成功。）

**结论 1**：lease 互斥成立——未过期时异 owner acquire 被 `THROW VERSION_COMMIT_LEASE_BUSY` 拒绝且事务回滚（租约不变）；同 owner 重入放行；过期后异 owner 可接管。与 `acquire_dbnum_lease` 的 SQL 契约完全一致。

---

## 2. PendingCommit 门禁（对照 `reject_pending_commit`，version_commit.rs L450-484）

### 2.1 fixture：`version_commit_state:[9101, 5]` status='pending'

```sql
UPSERT version_commit_state:[9101, 5] SET dbnum = 9101, from_sesno = 4, to_sesno = 5, source = 'incremental', fingerprint = 'fp-9101-5', source_observation_hash = NONE, status = 'pending', pe_rows = 0, att_rows = 0, uda_rows = 0, delete_count = 0, dbnum_info_updates = 0, pe_owner_rows = 0, anchored_at = NONE, last_error = 'apply failed: injected-for-verify', updated_at = time::now();
SELECT dbnum, to_sesno, status, pe_owner_rows, last_error FROM version_commit_state:[9101, 5];
```

返回（第 2 条）：

```json
[{"dbnum":9101,"last_error":"apply failed: injected-for-verify","pe_owner_rows":0,"status":"pending","to_sesno":5}]
```

### 2.2 非 recover 路径门禁查询（照搬 reject_pending_commit，recovering=false 时无 fingerprint 子句）→ 预期命中

```sql
SELECT to_sesno FROM version_commit_state WHERE dbnum = 9101 AND status IN ['preparing', 'pending'] ORDER BY to_sesno ASC LIMIT 1;
-- => [{"to_sesno":5}]   (status OK)
```

命中 `to_sesno=5` → Rust 侧 `rows.into_iter().next()` 为 Some，任何该 dbnum 的新提交（如更高 sesno=6）都会返回 `VersionCommitError::PendingCommit { dbnum: 9101, pending_sesno: 5, requested_sesno: 6 }` 被阻断。

旁证（per-dbnum 隔离，同 SQL 换 dbnum=9102）：

```sql
SELECT to_sesno FROM version_commit_state WHERE dbnum = 9102 AND status IN ['preparing', 'pending'] ORDER BY to_sesno ASC LIMIT 1;
-- => []   (status OK；9101 的 pending 不影响其它 dbnum)
```

### 2.3 'committed' 状态不触发 → 预期查空

```sql
UPDATE version_commit_state:[9101, 5] SET status = 'committed', anchored_at = <datetime>'2026-07-19T00:00:00Z', last_error = NONE, updated_at = time::now();
SELECT to_sesno FROM version_commit_state WHERE dbnum = 9101 AND status IN ['preparing', 'pending'] ORDER BY to_sesno ASC LIMIT 1;
```

返回：UPDATE OK（status 变 committed），门禁 SELECT → `[]`（status OK）。

**结论 2**：PendingCommit 门禁成立——存在 `status IN ['preparing','pending']` 记录时门禁查询命中（更高 sesno 提交会被 `PendingCommit` 错误阻断）；转为 'committed' 后同查询为空、不再触发；且门禁按 dbnum 隔离。

---

## 3. schema 幂等（对照 `ensure_version_commit_schema`，version_commit.rs L314-361；含 specs/023 新增 `pe_owner_rows`）

执行体 = `ensure_sesno_version_anchor_schema` 基础 DDL（database.rs L578-586）+ `fn::sesno_version` / `fn::sesno_version_hit`（DEFINE FUNCTION OVERWRITE，database.rs L608-661）+ `ensure_version_commit_schema` 自身 DDL（version_commit.rs L318-353，含两处 `pe_owner_rows`），共 41 条语句，与代码逐字一致（此处不重复粘贴，语句见上述源文件行号）。

| 步骤 | 环境 | 结果 |
|---|---|---|
| 第 1 次执行 | `lease_gate`（首次访问的新 db） | 40 OK / 1 ERR：首条语句报 `The namespace 'vc_verify' does not exist`，其余 40 条 OK |
| 第 2 次执行 | 同上 | **41 OK / 0 ERR** |
| 预建 ns/db 后第 1 次 | `lease_gate_fresh2`（`DEFINE NAMESPACE/DATABASE IF NOT EXISTS` 预建的空库） | **41 OK / 0 ERR** |
| 预建 ns/db 后第 2 次 | 同上 | **41 OK / 0 ERR** |

说明：那条唯一 ERR 是 HTTP 头指定 ns/db 时"首条语句早于 lazy 建库"的边界产物，与 DDL 本身无关——预建 ns/db（空库）后同一 DDL 两次执行均 41/41 全 OK。真实代码路径中 SDK 先 `use_ns/use_db` 建好会话才执行 schema，不会遇到该边界；`IF NOT EXISTS` + `OVERWRITE` 语义下重复执行零报错，幂等成立。

`INFO FOR TABLE` 证据（两个表都已带 `pe_owner_rows`）：

```json
version_commit_state.fields.pe_owner_rows  = "DEFINE FIELD pe_owner_rows ON version_commit_state TYPE int DEFAULT 0 PERMISSIONS FULL"
sesno_version_anchor.fields.pe_owner_rows = "DEFINE FIELD pe_owner_rows ON sesno_version_anchor TYPE none | int PERMISSIONS FULL"
```

（`version_commit_state` 上 `TYPE int DEFAULT 0`、`sesno_version_anchor` 上 `option<int>`，与源码及 serde default 兼容策略一致。）

**结论 3**：`ensure_version_commit_schema` 全套 DDL（含 `pe_owner_rows`）在空库可重复执行不报错，`IF NOT EXISTS` / `OVERWRITE` 幂等成立；字段落库形态与源码声明一致。

---

## 复现方式

对 `http://127.0.0.1:8030/sql` 发 HTTP POST（Basic root/root，头 `surreal-ns: vc_verify` / `surreal-db: lease_gate`），按本文各节顺序提交 SQL 即可；fixture 开头的 `DELETE version_commit_lease; DELETE version_commit_state;` 保证可重复运行。验证数据保留在 `ns=vc_verify` 下（`lease_gate` / `lease_gate_fresh2`），与业务 db 隔离，可随时删除。
