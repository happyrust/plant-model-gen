# specs/022 验证：不可变锚点 fingerprint 幂等/冲突语义

- 日期：2026-07-19
- 实例：`127.0.0.1:8030`（RocksDB `versioned=true&retention=30d`，`db-data/run_surrealkv_versioned.ps1`，surreal `3.2.0-nightly`）
- 隔离：独立 `ns=vc_verify db=anchor_fp`，dbnum 用假编号 `9201`，未触碰其它 ns/db
- 对照代码：`src/versioned_db/version_commit.rs` 的 `existing_idempotent_outcome` / `create_immutable_anchor`；schema 逐字取自 `database.rs::ensure_sesno_version_anchor_schema` + `version_commit.rs::ensure_version_commit_schema` 中 `sesno_version_anchor` 的 option 字段（含 specs/023 新增 `pe_owner_rows`）
- 脚本：`db-data/verify_022_anchor_fp.surql`（可重复执行，fixture 只 `DELETE ... WHERE dbnum = 9201`）
- 执行方式：`Get-Content -Raw db-data/verify_022_anchor_fp.surql | surreal sql --endpoint http://127.0.0.1:8030 --user root --pass root --ns vc_verify --db anchor_fp --hide-welcome`

## 结论总览

| # | 验证点 | 结果 |
|---|--------|------|
| 1 | create-once：`CREATE ONLY` 首次成功、同 id 再 CREATE 报错、锚点不可覆盖 | PASS |
| 2 | 幂等读回：`existing_idempotent_outcome` 投影读回完整 counts；不存在的 id 返回空 | PASS |
| 3 | legacy 锚点：无 fingerprint 字段读回 `NONE` → 对应 Rust `LegacyAnchor` 拒绝路径 | PASS |
| 4 | 唯一索引 `idx_sesno_version_anchor_dbnum_sesno` 存在且重复 `(dbnum,sesno)` 被拒 | PASS |

---

## [1] create-once（对照 `create_immutable_anchor`）

### 1a. 首次 CREATE ONLY（期望成功）

```surql
CREATE ONLY sesno_version_anchor:[9201, 10] SET dbnum = 9201, sesno = 10, from_sesno = 9,
  source = 'incremental', fingerprint = 'fp-A', source_observation_hash = NONE,
  pe_rows = 12, att_rows = 34, uda_rows = 5, delete_count = 2,
  dbnum_info_updates = 1, pe_owner_rows = 7, anchored_at = time::now() RETURN anchored_at;
```

返回：

```
[{ anchored_at: d'2026-07-19T15:10:35.858233300Z' }]
```

### 1b. 同 id 再 CREATE（fingerprint 换成 `fp-B`、counts 全改 99，期望报错）

```surql
CREATE ONLY sesno_version_anchor:[9201, 10] SET ... fingerprint = 'fp-B', pe_rows = 99, ... ;
```

返回（错误文案原文）：

```
['Database record `sesno_version_anchor:[9201, 10]` already exists']
```

### 1c. 锚点未被覆盖

```surql
SELECT dbnum, sesno, fingerprint, pe_rows, pe_owner_rows FROM sesno_version_anchor:[9201, 10];
```

返回：

```
[[{ dbnum: 9201, fingerprint: 'fp-A', pe_owner_rows: 7, pe_rows: 12, sesno: 10 }]]
```

**结论：PASS。** `CREATE ONLY` 是 create-once 语义：首次写入成功并 `RETURN anchored_at`（与 `create_immutable_anchor` 的返回值路径一致）；同 id 重放报 ``Database record `sesno_version_anchor:[9201, 10]` already exists``，且字段丝毫未被第二次请求污染（仍是 fp-A / 12 / 7，而非 fp-B / 99）——锚点不可覆盖。Rust 侧的 `FingerprintConflict` 正是建立在这一保证上：并发/重放者 CREATE 失败后回到 `existing_idempotent_outcome` 读回 `fp-A`，与自己请求的 `fp-B` 不等 → 抛 `immutable anchor conflict ... existing fingerprint=fp-A, requested fingerprint=fp-B`。

## [2] 幂等读回（照搬 `existing_idempotent_outcome` 的 SELECT）

### 2a. 命中已有锚点 `[9201, 10]`

```surql
SELECT fingerprint, type::string(anchored_at) AS anchored_at,
       pe_rows, att_rows, uda_rows, delete_count, dbnum_info_updates, pe_owner_rows
FROM sesno_version_anchor:[9201, 10];
```

返回：

```
[[{ anchored_at: '2026-07-19T15:10:35.858233300Z', att_rows: 34, dbnum_info_updates: 1,
    delete_count: 2, fingerprint: 'fp-A', pe_owner_rows: 7, pe_rows: 12, uda_rows: 5 }]]
```

### 2b. 不存在的 `[9201, 99]`

同一 SELECT 投影，返回：

```
[[]]
```

**结论：PASS。** 投影完整读回 fingerprint + 全部 counts（`pe_rows/att_rows/uda_rows/delete_count/dbnum_info_updates`，含 specs/023 新增的 `pe_owner_rows` option 字段，值 7 原样返回）；`type::string(anchored_at)` 把 datetime 转成纯字符串（无 `d''` 前缀），与 Rust `ExistingAnchor.anchored_at: String` 的反序列化契约吻合。不存在的 id 返回空数组 → Rust 侧 `rows.into_iter().next()` 为 `None` → 返回 `Ok(None)` 走全新提交路径；命中且 fingerprint 相等时即可拼出 `idempotent: true` 的 outcome，无需重做落库。

## [3] legacy 锚点（无 fingerprint → `LegacyAnchor` 拒绝路径）

### 3a. 造一条早于锚定机制字段扩展的锚点（只 SET 基础四字段）

```surql
CREATE ONLY sesno_version_anchor:[9201, 20] SET dbnum = 9201, sesno = 20, anchored_at = time::now(), source = 'full';
```

返回（成功，记录本体无 fingerprint 字段）：

```
[{ anchored_at: d'2026-07-19T15:10:35.880489600Z', dbnum: 9201, id: sesno_version_anchor:[9201, 20], sesno: 20, source: 'full' }]
```

### 3b. 用 `existing_idempotent_outcome` 投影读回

```
[[{ anchored_at: '2026-07-19T15:10:35.880489600Z', att_rows: NONE, dbnum_info_updates: NONE,
    delete_count: NONE, fingerprint: NONE, pe_owner_rows: NONE, pe_rows: NONE, uda_rows: NONE }]]
```

### 3c. 显式判空

```surql
RETURN sesno_version_anchor:[9201, 20].fingerprint ?? 'FINGERPRINT_IS_NONE';
```

返回：`['FINGERPRINT_IS_NONE']`

**结论：PASS。** legacy 锚点读回时 `fingerprint` 为 `NONE`（counts 亦全 `NONE`），对应 Rust `ExistingAnchor.fingerprint: Option<String>` 反序列化为 `None`，命中 `let Some(fingerprint) = existing.fingerprint else { ... }` 的 else 分支 → 抛 `LegacyAnchor`（"legacy anchor for dbnum=.. sesno=.. has no fingerprint and is read-only"）：旧锚点只读，不允许任何新提交冒充其幂等重放。

## [4] 唯一索引 `idx_sesno_version_anchor_dbnum_sesno`

### 4a. `INFO FOR TABLE sesno_version_anchor` 的 indexes 段

```
indexes: { idx_sesno_version_anchor_dbnum_sesno:
  'DEFINE INDEX idx_sesno_version_anchor_dbnum_sesno ON sesno_version_anchor FIELDS dbnum, sesno UNIQUE' }
```

（fields 段同时确认 `fingerprint/pe_rows/att_rows/uda_rows/delete_count/dbnum_info_updates/pe_owner_rows` 均为 `none | int` / `none | string` 的 option 类型，`source` 带 `ASSERT $value INSIDE ['full', 'incremental']`。）

### 4b. 换 record id 但字段对重复：`[9201, 21]` SET `(dbnum,sesno)=(9201,20)`（期望被拒）

```surql
CREATE sesno_version_anchor:[9201, 21] SET dbnum = 9201, sesno = 20, source = 'full', fingerprint = 'fp-dup', anchored_at = time::now();
```

返回（错误文案原文）：

```
['Database index `idx_sesno_version_anchor_dbnum_sesno` already contains [9201, 20], with record `sesno_version_anchor:[9201, 20]`']
```

### 4c. 终态核对

```surql
SELECT sesno, fingerprint FROM sesno_version_anchor WHERE dbnum = 9201 ORDER BY sesno ASC;
```

返回：

```
[[{ fingerprint: 'fp-A', sesno: 10 }, { fingerprint: NONE, sesno: 20 }]]
```

**结论：PASS。** 唯一索引存在且生效：即便绕开复合 record id 用不同 id 写入，重复 `(dbnum, sesno)` 字段对也会被索引拒绝——锚点"一个 (dbnum,sesno) 只此一条"由 record-id create-once 和唯一索引双保险兜底。终态恰好 2 行（sesno 10 / 20），无脏数据。

---

## 与代码语义的对应小结

- `CREATE ONLY` 已存在即报错 → `create_immutable_anchor` 失败后走 `mark_commit_pending`，等待恢复或人工处理；锚点本体永不被 UPDATE/UPSERT。
- `existing_idempotent_outcome` 的三分支在引擎侧的证据链：空数组（→ `None` 全新提交）、`fingerprint: NONE`（→ `LegacyAnchor`）、`fingerprint` 相等（→ `idempotent: true` 直接返回既有 counts）/不等（→ `FingerprintConflict`）。
- specs/023 的 `pe_owner_rows` 作为 `option<int>` 已进 schema、投影与读回，legacy 行读为 `NONE` 不炸投影。
