# Quickstart: 存量站点切换到统一 versioned 实例 (specs/022 + specs/024)

> **2026-07-20 Spec 024 硬切换**：PE/ATT 与模型历史统一由同一 RocksDB
> versioned 实例回答；DuckLake/release/history-replay 已退役。生产升级只允许
> 新数据目录全量重灌，不转换旧模型 ID 或旧锚点时间戳。

## Goal

把一个既有（非 versioned）managed 站点安全切换到 SUL_DB RocksDB `versioned=true` 存储：新建数据目录 → 全量重灌 → 首条锚点确认 → 当前态抽样比对 → 切流 → 增量回归。切换后当前态查询结果与旧库一致（SC-004），且增量落库 / 锚点固化 / 模型生成全链路无回归。

## 关键约束（务必先读）

- **versioned 是建库属性**：已存在的非 versioned 数据目录**不能**原地以 `versioned=true` 打开（RocksDB UDT comparator 不匹配会直接启动失败）。切换=**新建数据目录 + 重灌**，不是改开关就完事。
- **同一项目写入必须串行**：`watch-incremental` 是唯一常驻增量入口，并持有
  `output/<project>/incremental.lock`；手工 incremental/regen/full generation
  会争用同一把 OS 文件锁。
- **Version Commit 还会取得数据库 lease**：手工 CLI 与多个 watch 进程也不能并发提交同一 dbnum。失败会留下 `commit_pending` 并阻断后续 sesno。
- **锚点按 source 分流**：键为 `[dbnum,sesno,source]`。数据查询只解析
  `full/incremental`；模型查询与导出只解析 `model_gen`。数据 commit 锚点
  create-once；成功重跑模型生成会刷新同一 `model_gen` 锚点。
- **retention 默认 `0`（无限保留）**：可按站点改为 `90d`/`30d` 等；磁盘只增不减，需评估盘余量。仅改 retention 不需要重建库；但管理端对**已初始化**站点改 `versioned_storage` / `version_retention` 一律拒绝（避免与数据目录不一致）。
- **不用 `cargo test`**：全程 CLI `--json` + SurrealQL 脚本验证（仓库规则）。web_server 相关走 HTTP/POST。

## Prerequisites

- 部署环境使用本 fork 构建的 surreal 二进制（官方发行版无 RocksDB versioned 能力）。fork checkout：`D:\work\plant-code\surrealdb`（branch dev-3.1）。
- 一个既有站点（非 versioned），其 `DbOption.toml` 与旧数据目录、旧站点当前态查询结果可访问。
- Admin 认证 token（调用受保护端点时）。
- 参考脚本 / fixture：
  - `db-data/run_surrealkv_versioned.ps1`（起 versioned 实例）
  - `db-data/verify_versioned_pe_att.surql`（锚点 + VERSION 契约）
  - `db-data/DbOption-t020-history.toml`（history CLI fixture，ns=`test` db=`anchor_verify` port 8030）
- history CLI：数据使用 `snapshot/timeline/diff`，模型使用
  `model-snapshot/model-diff`；交付使用 `model-version export`（均需先连 Surreal）。

## Scenario 0: 管理端开关行为（T022）

| 场景 | 行为 |
|---|---|
| **新建站点** `POST .../sites` 带 `versioned_storage: true`（可选 `version_retention`） | 写入站点 `DbOption.toml`；首启必须以新目录建库 |
| **未初始化**（`parse_status` 为 Pending，且非 Running）改开关 | 允许写盘 |
| **已初始化**（`Parsed` / `Failed`）改 `versioned_storage` 或 `version_retention` | **拒绝**，错误含「需要重灌」/ quickstart 指引；**不静默改参数** |
| 其它字段更新触发 `write_site_files` | **保留**既有 `versioned_storage` / `version_retention`（不会被重建配置抹掉） |

示例（已初始化站点会被拒）：

```http
PATCH /api/admin/managed-sites/{site_id}
Content-Type: application/json

{ "versioned_storage": true, "version_retention": "0" }
```

**Expected**：HTTP 4xx + 文案提示按本手册新建数据目录并 `sync_pdms` 重灌；磁盘上 `DbOption.toml` 的 versioned 字段不变。

## Scenario 1: 新建 versioned 数据目录 + 站点配置开关

1. 为站点分配一个**全新**数据目录（不复用旧目录）；旧目录保留作对照。
2. 未初始化站点可通过管理端写入；已初始化站点请**手工**改 `DbOption.toml`（或新建站点）并同步改 `db_data_path` / 启动路径到新目录：

```toml
versioned_storage = true
version_retention = "0"
```

3. 站点启动串应拼成 `rocksdb://<新数据目录>?versioned=true&retention=0`。

**Expected**：站点进程 / systemd 模板 / nohup 模板三处启动命令均带 `?versioned=true&retention=0`（`?`/`&` 在 bash 下留在引号内）。`INFO FOR DB` 可连通；写读 + `SELECT ... VERSION time::now()` 冒烟通过。

## Scenario 2: 全量重灌 + 首条锚点确认

1. 对新 versioned 目录跑全量重灌（复用现有 `sync_pdms` 链路，**不写一次性迁移工具**）。
2. 重灌完成后查锚点表：

```surql
SELECT * FROM sesno_version_anchor WHERE source = 'full' ORDER BY dbnum;
```

**Expected**：每个成功解析的 dbnum 有一条 `source='full'` 数据锚点；随后完整
模型生成成功时出现同 sesno 的 `source='model_gen'` 锚点，且
`model_gen.anchored_at` 晚于该批全部模型写入。生成失败不得写 model_gen。

契约级对照（无项目数据时）：对 fixture 跑 `db-data/verify_versioned_pe_att.surql`（见 T012）。

## Scenario 3: 当前态抽样比对（旧库 vs 新库）

1. 选一组代表性 refno（覆盖多种 noun、含最近改过的元素）。
2. 对旧库与新 versioned 库分别查当前态 PE + ATT，逐字段比对。

**Expected**：抽样 refno 集合当前态逐字段一致（SC-004）。差异清单记入本文附录 A。

## Scenario 4: 切流 + 增量回归 + history CLI

1. 将站点流量切到新 versioned 实例（入口 / 端口 / 数据目录均指向新库）。
2. 跑一次 `incremental-sesno --file <...> --json`（或 `--dbnum`，需 sqlite-index feature）。
3. 用 history CLI 抽检（配置指向新库 / 或 fixture）：

```text
model-version history snapshot --refno <E> --sesno <N> --json
model-version history timeline --refno <E> --from-sesno <A> --to-sesno <B> --json
model-version history diff --refno <E> --from-sesno <A> --to-sesno <B> --json
model-version history model-snapshot --refno <E> --sesno <N> --dbnum <D> --json
model-version history model-diff --refnos <E1,E2> --from-sesno <A> --to-sesno <B> --dbnum <D> --json
model-version export --dbnum <D> --sesno <N> --format v3-json --json
```

库内快速换算（`ensure_sesno_version_anchor_schema` 会 `DEFINE FUNCTION OVERWRITE`，与 rs-core `resolve_anchor` 同语义）：

```surql
-- 只要时刻
LET $t = fn::data_sesno_version(<DBNUM>, <SESNO>);
SELECT * FROM pe:<id> VERSION $t;

-- 要元数据（含 exact / 实际命中 sesno）
RETURN fn::data_sesno_version_hit(<DBNUM>, <SESNO>);

-- 模型锚点（只认 source='model_gen'）
RETURN fn::model_sesno_version_hit(<DBNUM>, <SESNO>);
```

CLI 一键查锚点：

```text
model-version resolve-anchor --dbnum <D> --sesno <N> --json
model-version resolve-anchor --dbnum <D> --sesno <N> --exact-only
```

**Expected**：
- 增量落库成功，汇总 JSON 含 `version_anchor` 字段（本批 dbnum/sesno/anchored_at）。
- 锚点表出现对应 `source='incremental'` 记录。
- 模型生成开启且成功时出现对应 `source='model_gen'` 记录；persist-only 或生成失败时不出现。
- `version_commit_state` 对应记录为 `status='committed'`，锚点含非空 fingerprint。
- 模型生成、导出全链路无回归。
- `snapshot` 能返回改前属性；过期窗口外返回明确「历史已过期」类错误（不 panic、不空冒充）。

若命令报告 `commit_pending`，不得直接推进更高 sesno。先确认源文件/manifest 未变化，再用完全相同的 file/dbnum、from/to 与 source-observation 参数重放：

```text
incremental-sesno ... --recover-pending --json
```

恢复成功后应复用相同 fingerprint、发布一次不可变数据锚点，并把状态改为
`committed`。若 fingerprint 不同，停止并重新核对源文件与区间，不得覆盖旧锚点。

## Spec 024 存量测试站清理（禁止生产使用）

存量 versioned **测试站**若需要复用目录，可先显式创建安全门记录，再导入
`db-data/migrate_024_reset_versioned_model.surql`。脚本会删除旧形状模型表、
锚点与 commit metadata；随后必须重新 `sync_pdms` 并完整 regen。

生产站不运行该脚本：创建全新 RocksDB 目录，完整重灌后切流；回滚就是停止新站、
恢复旧站流量。有限 retention 窗口外的历史无法迁移或补写，只能重扫源 DB。

## 附录 A: 切换执行记录（T023 填写）

| 站点 | 切换日期 | dbnum 集合 | 首条 full 锚点 | 抽样比对结果 | 增量回归结果 | 备注 |
|---|---|---|---|---|---|---|
| _(待填 — 阻塞：需真实 managed 测试站点)_ | | | | | | |

## 附录 B: 已知回退与兜底

- retention 窗口外的 sesno 历史：CLI 返回明确「历史已过期」错误（不 panic、不返回空冒充）。此时改用 PDMS 源 db 文件重扫。
- 模型表与 PE/ATT 同库一并版本化（SurrealKV/MODEL_KV 分离已移除），磁盘增长由 retention 兜底；默认 `0` 时务必评估盘余量，必要时改为有限窗口。
- 管理端已初始化站点改开关被拒时：按 Scenario 1 新建目录 + 手工配置 + Scenario 2 重灌，**不要**试图原地改 comparator。
