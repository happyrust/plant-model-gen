# Quickstart: 存量站点切换到 versioned 实例 (specs/022 M4)

**状态**: T021 手册已刷新（2026-07-16）。T022 站点编辑防护已落地；T023 全站切换演练待有真实测试站点后补附录。

## Goal

把一个既有（非 versioned）managed 站点安全切换到 SUL_DB RocksDB `versioned=true` 存储：新建数据目录 → 全量重灌 → 首条锚点确认 → 当前态抽样比对 → 切流 → 增量回归。切换后当前态查询结果与旧库一致（SC-004），且增量落库 / 锚点固化 / 模型生成全链路无回归。

## 关键约束（务必先读）

- **versioned 是建库属性**：已存在的非 versioned 数据目录**不能**原地以 `versioned=true` 打开（RocksDB UDT comparator 不匹配会直接启动失败）。切换=**新建数据目录 + 重灌**，不是改开关就完事。
- **同一 dbnum 增量必须串行**：现有 watch-incremental 单队列已满足，切换后不要引入并发增量。
- **retention 默认 90d**：`retention=0` 为无限保留（磁盘风险，需显式确认）。仅改 retention 不需要重建库；但管理端对**已初始化**站点改 `versioned_storage` / `version_retention` 一律拒绝（避免与数据目录不一致）。
- **不用 `cargo test`**：全程 CLI `--json` + SurrealQL 脚本验证（仓库规则）。web_server 相关走 HTTP/POST。

## Prerequisites

- 部署环境使用本 fork 构建的 surreal 二进制（官方发行版无 RocksDB versioned 能力）。fork checkout：`D:\work\plant-code\surrealdb`（branch dev-3.1）。
- 一个既有站点（非 versioned），其 `DbOption.toml` 与旧数据目录、旧站点当前态查询结果可访问。
- Admin 认证 token（调用受保护端点时）。
- 参考脚本 / fixture：
  - `db-data/run_surrealkv_versioned.ps1`（起 versioned 实例）
  - `db-data/verify_versioned_pe_att.surql`（锚点 + VERSION 契约）
  - `db-data/DbOption-t020-history.toml`（history CLI fixture，ns=`test` db=`anchor_verify` port 8030）
- M3 history CLI 已可用：`model-version history {snapshot,timeline,diff}`（需先连 Surreal）。

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

{ "versioned_storage": true, "version_retention": "90d" }
```

**Expected**：HTTP 4xx + 文案提示按本手册新建数据目录并 `sync_pdms` 重灌；磁盘上 `DbOption.toml` 的 versioned 字段不变。

## Scenario 1: 新建 versioned 数据目录 + 站点配置开关

1. 为站点分配一个**全新**数据目录（不复用旧目录）；旧目录保留作对照。
2. 未初始化站点可通过管理端写入；已初始化站点请**手工**改 `DbOption.toml`（或新建站点）并同步改 `db_data_path` / 启动路径到新目录：

```toml
versioned_storage = true
version_retention = "90d"
```

3. 站点启动串应拼成 `rocksdb://<新数据目录>?versioned=true&retention=90d`。

**Expected**：站点进程 / systemd 模板 / nohup 模板三处启动命令均带 `?versioned=true&retention=90d`（`?`/`&` 在 bash 下留在引号内）。`INFO FOR DB` 可连通；写读 + `SELECT ... VERSION time::now()` 冒烟通过。

## Scenario 2: 全量重灌 + 首条锚点确认

1. 对新 versioned 目录跑全量重灌（复用现有 `sync_pdms` 链路，**不写一次性迁移工具**）。
2. 重灌完成后查锚点表：

```surql
SELECT * FROM sesno_version_anchor WHERE source = 'full' ORDER BY dbnum;
```

**Expected**：每个成功解析的 dbnum 有一条 `source='full'` 锚点，`sesno` = 该 dbnum 的 latest_sesno，`anchored_at` 晚于该批全部 UPSERT/DELETE。

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
```

**Expected**：
- 增量落库成功，汇总 JSON 含 `version_anchor` 字段（本批 dbnum/sesno/anchored_at）。
- 锚点表出现对应 `source='incremental'` 记录。
- 模型生成、导出全链路无回归。
- `snapshot` 能返回改前属性；过期窗口外返回明确「历史已过期」类错误（不 panic、不空冒充）。

## 附录 A: 切换执行记录（T023 填写）

| 站点 | 切换日期 | dbnum 集合 | 首条 full 锚点 | 抽样比对结果 | 增量回归结果 | 备注 |
|---|---|---|---|---|---|---|
| _(待填 — 阻塞：需真实 managed 测试站点)_ | | | | | | |

## 附录 B: 已知回退与兜底

- retention 窗口外的 sesno 历史：CLI 返回明确「历史已过期」错误（不 panic、不返回空冒充）。此时改用 DuckLake 存档（specs/023）或 PDMS 源 db 文件重扫。
- 未启用 MODEL_KV 分离的站点：模型表也会被版本化，磁盘增长由 retention=90d 兜底；建议评估磁盘余量或启用 KV 分离。
- 管理端已初始化站点改开关被拒时：按 Scenario 1 新建目录 + 手工配置 + Scenario 2 重灌，**不要**试图原地改 comparator。
