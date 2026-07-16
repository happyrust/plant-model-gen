# Implementation Plan: PE/ATT 版本化存储（SurrealDB RocksDB versioned）

**Branch**: `022-versioned-pe-att-storage` | **Date**: 2026-07-13 | **Spec**: `specs/022-versioned-pe-att-storage/spec.md`

**Input**: Feature specification from `/specs/022-versioned-pe-att-storage/spec.md`

## Summary

在 SUL_DB（项目主库，RocksDB）实例级开启 fork 提供的 `versioned=true` MVCC 存储（retention 默认 90d 可配置），新建 `sesno_version_anchor` 锚点表把业务 sesno 映射到存储时间戳，PE/ATT 历史由 `SELECT ... VERSION $t` 直接回答；删除保持硬 DELETE；与 DuckLake 发布链共存；存量站点靠 sync_pdms 重灌切换；消费入口先只做 CLI。

## Technical Context

**Language/Version**: Rust（workspace 现行 toolchain）

**Primary Dependencies**:
- `surrealdb`（git happyrust/surrealdb, branch dev-3.1，本地 checkout D:\work\plant-code\surrealdb）——RocksDB UDT versioned + VERSION 查询
- `aios_core`（git happyrust/rs-core, branch dev-3.1，本地 checkout D:\work\plant-code\rs-core）——`project_primary_db()` / SUL_DB 连接层，新增 version_query 模块
- plant-model-gen 本仓——启动参数透传、锚点写入、CLI

**Storage**: SurrealDB over RocksDB（`rocksdb://<dir>?versioned=true&retention=90d`）；锚点表在项目 NS/DB 内

**Testing**: 仓库规则——不写 cargo test；用 CLI + JSON 输出验证，web_server 相关用 HTTP/POST 验证。参考 `db-data/test_version_data.surql` 扩展验证脚本

**Target Platform**: Windows 开发机 + Linux 部署（systemd/nohup 脚本模板同步改）

**Project Type**: CLI + 常驻服务（web_server 站点管理）

**Performance Goals**: 单元素历史快照查询 < 2s；增量落库因锚点写入增加的开销 < 1%（一条 UPSERT）

**Constraints**:
- versioned 是建库属性：已存在的非 versioned 数据目录不能原地打开为 versioned，必须新目录重灌
- GC 60s 粒度推进水位线；低于水位线的 VERSION 读报 InvalidArgument，必须在封装层翻译
- HLC 进程内单调；锚点时间取数据库侧 `time::now()`
- 同一 dbnum 增量落库必须串行（现状满足，需在文档固化为约束）

**Scale/Scope**: 单站点 PE/ATT 千万级记录；90d retention 下版本增量与增量修改率成正比（PDMS 设计库日修改量通常远小于全量）

## Constitution Check

- 不使用 cargo test：验证全部走 CLI `--json` 与 SurrealQL 脚本 ✅
- aios-database 用 cli + json 方式测试验证 ✅
- 不写入任何密钥到仓库 ✅

## Project Structure

### Documentation (this feature)

```text
specs/022-versioned-pe-att-storage/
├── spec.md              # 已完成
├── plan.md              # 本文件
└── quickstart.md        # M4 产出：存量站点切换 + 验证步骤
```

### Source Code (repository root)

```text
D:\work\plant-code\rs-core\src\rs_surreal\
└── version_query.rs         # 新增：sesno→时间戳换算、VERSION 查询封装、GC 越界错误翻译

d:\work\plant-code\plant-model-gen\src\
├── options.rs               # DbOptionExt 新增 versioned_storage / version_retention 配置
├── cli_modes.rs             # 自启动 surreal start 连接串拼 ?versioned=true&retention=
├── web_server\
│   ├── managed_project_sites.rs   # 站点启动/脚本模板透传 versioned 参数
│   └── db_startup_manager.rs      # 同上
├── data_interface\
│   └── sesno_increment.rs   # 增量落库成功收尾写 sesno_version_anchor
├── versioned_db\
│   └── database.rs          # sync_pdms 全量重灌完成后写首条锚点
└── version_management\
    └── cli.rs               # model-version history snapshot/timeline/diff 子命令

db-data\
└── verify_versioned_pe_att.surql   # 验证脚本（基于 test_version_data.surql 扩展）
```

**Structure Decision**: 封装逻辑下沉 rs-core（version_query），本仓只做配置透传、锚点写入时机、CLI 参数层——与现有 project_primary_db 分层一致。

## Milestones

### M1 — 实例级 versioned 开关（P1 前置）

1. `DbOptionExt` 新增 `versioned_storage: bool`（默认新站点 true）与 `version_retention: String`（默认 "90d"）
2. 所有 `surreal start` 拼接点追加 `?versioned=true&retention=<r>`：
   - `cli_modes.rs`（自动启动 + 手动启动提示文案）
   - `web_server/managed_project_sites.rs`（站点进程 + systemd/nohup 脚本模板）
   - `web_server/db_startup_manager.rs`、`web_server/handlers.rs`、`src/bin/web_server.rs`
3. 嵌入式 open（如有 rocksdb:// 直连路径）同步拼参数
4. 验证：起一个新实例，`INFO FOR DB` / 写读 + `VERSION time::now()` 冒烟

### M2 — 锚点表与固化时机（P1）

1. schema：`DEFINE TABLE sesno_version_anchor SCHEMAFULL`，字段 dbnum(int)、sesno(int)、anchored_at(datetime, DEFAULT time::now())、source(string)，唯一索引 (dbnum, sesno)
2. `sesno_increment.rs::persist_pdms_increment_*` 成功路径末尾（全部 upsert/delete flush 完成后）UPSERT 锚点；失败路径不写（沿用现在 exec_sql_checked 的错误传播，天然满足 FR-004）
3. `versioned_db/database.rs::sync_pdms` 完成后写 `source="full"` 首条锚点（每个 dbnum 当前 latest_sesno）
4. 验证：CLI 跑一次增量，SurrealQL 查锚点表核对时间序

### M3 — rs-core version_query + CLI（P1/P2）

1. rs-core 新增 `version_query.rs`：
   - `resolve_anchor(dbnum, sesno) -> Option<(sesno, datetime)>`（最近不大于回退）
   - `snapshot_at(refno, sesno)`：锚点换算 → `SELECT * FROM <pe_key> VERSION $t` + ATT 表同刻查询
   - `diff_range(refnos, from_sesno, to_sesno)`：两端快照字段级对比
   - GC 越界（InvalidArgument）翻译为 `HistoryExpired` 错误
2. plant-model-gen `version_management/cli.rs` 挂 `model-version history` 子命令组：snapshot / timeline / diff，全部支持 `--json`
3. 验证：对 db-data 测试实例执行三个子命令，核对与 test_version_data.surql 手工查询一致

### M4 — 存量切换流程 + quickstart（P2）

1. quickstart.md：新建 versioned 数据目录 → 改站点配置 → sync_pdms 重灌 → 首条锚点确认 → 抽样比对当前态 → 切流 → 跑一次增量回归
2. managed_project_sites 站点编辑允许改 versioned 开关（仅未初始化/待重灌站点可改，已初始化站点提示需重灌）
3. 验证：测试站点全流程走一遍（SC-004）

## 风险与对策

| 风险 | 对策 |
|------|------|
| 未启用 MODEL_KV 的站点模型表被版本化，磁盘增长 | retention=90d 兜底；quickstart 注明建议开 KV 分离；后续可观测磁盘水位 |
| 增量落库半途失败已写入部分版本（无锚点但历史里有中间态） | 锚点是唯一业务可见入口，无锚点的中间时间戳不对外暴露；重跑增量覆盖后写锚点 |
| fork 升级导致 VERSION 行为变化 | revision.lock 固定 fork 版本；升级时跑 verify_versioned_pe_att.surql 回归 |
| GC 水位线推进与长查询竞争 | 封装层统一翻译 HistoryExpired，CLI 提示改用 DuckLake 存档或源文件重扫 |

## Out of Scope

- HTTP 历史查询 API（Q7 决策推迟）
- 模型数据（inst_relate/mesh 等）的版本化
- 用 VERSION diff 替换 patch_only 发布链的 affected 证据（列为后续候选，本期不改 register/reconcile）
- SurrealKV 引擎的 versioned 路径（生产统一 RocksDB）
