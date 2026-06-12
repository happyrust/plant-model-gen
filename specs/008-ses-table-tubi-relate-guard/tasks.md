# Tasks: ses 表缺失导致 tubi_relate 写入崩溃修复（spec 008）

## T001 — 固化失败证据

- 保存 `quicktest-250160-8080` 的 `generate.log` 失败段（panic at
  `cata_model.rs:6253`、exit_code=101）与 `surreal.log` 版本行
  （`Running 3.2.0-nightly`）到本 spec。
- 记录对照组：源码版 `quicktest-7997-8080`（3.1 surreal）同代码生成成功。

验收：

- spec 008 的 Root Cause Chain 与现场日志可互相印证。
- 后续可用同一站点复跑对比。

## T002 — common.surql 增加 ses 空表保障

- 修改 `resource/surreal/common.surql`：在 `fn::ses_data` 定义（约 261 行）
  之前插入带注释的 `DEFINE TABLE IF NOT EXISTS ses SCHEMALESS;`。
- 不改 `fn::ses_data` / `fn::ses_date` 函数体。
- 不动 `src/fast_model/utils.rs::ensure_surreal_init()`（保留双保险）。
- 不动 `cata_model.rs` 的 fail-loud 行为。

验收：

- `git diff` 仅含 common.surql 一处新增段。
- 注释说明 spec 008 背景与"空表上安全返回 none"约束。

## T003 — 3.2.0-nightly 空库冒烟

- 用 release 包内 `bin/surreal/surreal.exe` 起临时内存实例（非 8020/8022 端口）。
- 加载修复后的 `common.surql`。
- 执行：
  1. `CREATE pe:[250160,1] SET dbnum=250160, sesno=1;`
  2. `RETURN fn::ses_date(pe:[250160,1]);` → `NONE`，无错误
  3. `RETURN fn::ses_date(pe:[999,1]);` → `NONE`
  4. `INFO FOR DB;` → `ses` 表存在
- 结束后停掉临时实例。

验收：

- 四步输出与期望一致；修复前同步骤第 2 步报
  `The table 'ses' does not exist`（可选对照）。

## T004 — release 包热修

- 复制仓库修复后的 `resource/surreal/common.surql` 到
  `dist/package/Plant3D-AIOS-win-x64/release/resource/surreal/common.surql`。
- 比对两文件哈希一致。

验收：

- 包内外 `common.surql` SHA256 相同。
- 不替换任何二进制。

## T005 — 250160 站点复跑（3.2.0-nightly 路径）

- 通过 3101 管理端对 `quicktest-250160-8080` 重新提交生成任务。
- 轮询任务与站点 runtime 至终态。
- 检查 `logs/generate.log`：
  - 无 `The table 'ses' does not exist`；
  - TUBI 阶段走到 `tubi_relate` 写入成功；
  - sidecar `job_done: status=succeeded, exit_code=0`。
- 站点库抽查 `select count() from tubi_relate`（期望约 16 条）。

验收：

- 任务 Completed、站点不再 Failed。
- tubi_relate 行数与本库 TUBI 数一致。

## T006 — 7997 站点回归（3.1 路径）

- 源码版（3110 服务）站点 `quicktest-7997-8080` 重跑生成（或闭包确认）。
- 检查 `tubi_relate` 写入与 `tubings.parquet` 非空（历史约 162KB）。

验收：

- 3.1 路径行为不变，无新增错误。

## T007 — 归档与变更记录

- `CHANGELOG.md` 记录：quick deploy 站点 TUBI 写入因 ses 表缺失崩溃的修复
  （common.surql 空表保障；SurrealDB 3.1/3.2-nightly 缺表行为差异记入已知事项）。
- spec 008 三件套更新实际验证结果与数据。

验收：

- spec / plan / tasks 与实现一致，维护者可按命令复现与验证。
