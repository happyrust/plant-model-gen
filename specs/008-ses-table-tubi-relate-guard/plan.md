# Implementation Plan: ses 表缺失导致 tubi_relate 写入崩溃修复（spec 008）

## Approach

把 `ses` 空表保障从"只挂在三个旁路入口的 Rust 函数"前移到
"所有进程初始化必经的 schema 脚本"：

1. `resource/surreal/common.surql` 在 `fn::ses_data` / `fn::ses_date`
   定义之前加 `DEFINE TABLE IF NOT EXISTS ses SCHEMALESS;`。
2. 保留 `utils.rs::ensure_surreal_init()` 既有 DEFINE 作双保险（幂等、零成本）。
3. 已发布 release 包热修：仅替换包内 `resource/surreal/common.surql`，
   重跑生成任务即可（每次 init 都会重新执行该脚本）。

选择依据（grill-me 决策，2026-06-12）：

| 问题 | 决策 |
|------|------|
| Q1 保障落点 | A：common.surql 加 DEFINE，单点、全进程覆盖、release 包可热修 |
| Q2 验收范围 | A：release 包 250160 复跑 + 源码版 7997 回归 + 空库冒烟，共三项 |
| Q3 版本差异处置 | A：仅记录为已知事项，不改捆绑策略 |
| Q4 dt 字段语义 | A：保留 `dt=fn::ses_date(...)`，空表时 none 可接受 |

## Evidence Summary

- 失败现场：`dist/package/Plant3D-AIOS-win-x64/release/runtime/admin_sites/quicktest-250160-8080/logs/generate.log`
  - 预检/几何生成全部正常（pe_transform 刷新 2736 节点、16 BRAN、23 CATA、91 实例）
  - `[BRAN_TUBI] tubi_relate 语句执行失败（数据未写入）: The table 'ses' does not exist`
  - panic at `cata_model.rs:6253`，exit_code=101
- 站点 SurrealDB 版本：`logs/surreal.log` → `Running 3.2.0-nightly`
- 对照成功现场：`quicktest-7997-8080`（源码版 3110 服务），站点 surreal 为
  `D:\Rust\.cargo\bin\surreal.exe`（3.1 系），同代码生成成功
- 既有修复未生效原因：`ensure_surreal_init()` 调用点仅
  `cli_modes.rs:539` / `cata_closure_verify.rs:199` / `transform_rkyv_cache.rs:206`，
  生成主流程日志显示走 `使用 aios_core::init_surreal 初始化数据库...`
- schema 加载机制：rs-core `src/rs_surreal/function.rs::execute_surql_files_on_db()`
  每次 init 按文件名排序加载 `resource/surreal/*.surql` 并执行，
  release 包以磁盘文件形式分发这些脚本

## Phase 0 — Pin Reproduction

当前失败可直接复用现场：

```powershell
# 站点生成日志（已含失败证据）
Get-Content "dist\package\Plant3D-AIOS-win-x64\release\runtime\admin_sites\quicktest-250160-8080\logs\generate.log" -Tail 10
```

最小复现（3.2.0-nightly 行为）：

```powershell
# 用包内 surreal 起临时内存库，加载未修复的 common.surql，
# 建一条 pe 后调 fn::ses_date —— 期望报 The table 'ses' does not exist
dist\package\Plant3D-AIOS-win-x64\release\bin\surreal\surreal.exe start --user root --pass root --bind 127.0.0.1:18029 memory
# 另一终端：
# surreal sql ... < resource/surreal/common.surql
# CREATE pe:[250160,1] SET dbnum=250160, sesno=1;
# RETURN fn::ses_date(pe:[250160,1]);
```

## Phase 1 — common.surql 空表保障

修改 `resource/surreal/common.surql`，在"获取节点的会话数据"段（`fn::ses_data`
定义，约 261 行）之前插入：

```surql
-- ses（会话日期）表仅在 sync_history=true 的解析流程中被填充；
-- quick deploy 站点没有该表时，fn::ses_data / fn::ses_date 在
-- SurrealDB 3.2.0-nightly 下会抛 "The table 'ses' does not exist"，
-- 连带 tubi_relate 等 RELATE 批量 SQL 整条失败（spec 008）。
-- 这里无条件保证空表存在：空表上两个函数安全返回 none。
DEFINE TABLE IF NOT EXISTS ses SCHEMALESS;
```

约束：

- 只加 DEFINE，不改 `fn::ses_data` / `fn::ses_date` 函数体。
- 不动 `utils.rs::ensure_surreal_init()`。

## Phase 2 — 最小冒烟（3.2.0-nightly 空库）

用包内 surreal 起临时内存实例，加载修复后的 common.surql：

1. `CREATE pe:[250160,1] SET dbnum=250160, sesno=1;`
2. `RETURN fn::ses_date(pe:[250160,1]);` → 期望 `NONE`，无错误。
3. `RETURN fn::ses_date(pe:[999,1]);`（pe 不存在）→ 期望 `NONE`。
4. `INFO FOR DB` 确认 `ses` 表存在。

## Phase 3 — Release 包热修与 250160 复跑

1. 把修复后的 `resource/surreal/common.surql` 复制到
   `dist/package/Plant3D-AIOS-win-x64/release/resource/surreal/common.surql`。
2. 通过 3101 管理端重跑站点 `quicktest-250160-8080` 的生成任务
   （或完整 deploy）。
3. 检查 `logs/generate.log`：
   - 不再出现 `The table 'ses' does not exist`；
   - `[BRAN_TUBI] 写入 tubi_relate 成功` 或等效成功路径；
   - sidecar `job_done: status=succeeded, exit_code=0`。
4. 站点库抽查：`select count() from tubi_relate` 约 16 条（本库 TUBI 数）。

## Phase 4 — 源码版 7997 回归（3.1 路径）

源码版（3110 服务）站点 `quicktest-7997-8080` 重跑生成或确认既有产物：

- `tubi_relate` 写入行为不变；
- `tubings.parquet` 仍非空（历史值约 162KB）；
- 无新增 warning/error。

## Phase 5 — 一致性与归档

- 仓库 `resource/surreal/common.surql` 与 release 包内文件一致
  （后续 `build-windows-bundle` 打包自然带上，本次热修是提前同步）。
- `CHANGELOG.md` 记录修复。
- spec 008 标记验证结果。

## Risks

- R1：`DEFINE TABLE IF NOT EXISTS` 在更老的 SurrealDB（<2.0）不支持
  `IF NOT EXISTS` 语法。缓解：项目两条在用版本为 3.1 / 3.2.0-nightly，
  均支持；common.surql 中已大量使用 `OVERWRITE` 等 2.x+ 语法，无新增兼容负担。
- R2：空表上 `select ... from only type::record('ses',[...])` 在未来 nightly
  再次变更行为。缓解：Phase 2 冒烟脚本保留为回归手段；Known Issues 已记录
  版本差异检查习惯。
- R3：站点库为 RocksDB 持久库，DEFINE 写入需要站点 surreal 在线。
  缓解：生成流程本身会先启动站点 surreal 并跑 init（加载 common.surql），
  顺序天然满足。

## Rollback

`common.surql` 仅新增一条幂等 DEFINE 与注释，回滚即删除该段；
不影响任何函数定义与既有数据。

## Done Definition

- 空库冒烟（3.2.0-nightly）：`fn::ses_date` 返回 none 且无错误。
- release 包 `quicktest-250160-8080` 生成任务 Completed，
  `tubi_relate` 写入成功。
- 源码版 `quicktest-7997-8080` 回归通过。
- 仓库与 release 包的 `common.surql` 一致，CHANGELOG 已记录。
