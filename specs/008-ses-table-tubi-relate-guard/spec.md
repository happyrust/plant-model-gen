# Feature Specification: ses 表缺失导致 tubi_relate 写入崩溃修复（spec 008）

## User Need

quick deploy 站点在 release 包环境下执行模型生成时，TUBI 关系写入阶段必须稳定成功，
不允许因为站点库缺少 `ses`（会话日期）表而让整次生成 panic 退出。

当前 release 包现场（站点 `quicktest-250160-8080`，AvevaPlantSample dbnum=250160，
任务 `avevaplantsample-_avevaplantsample-_a1a85633`，sidecar job `6fd7e350`）：

```text
[BRAN_TUBI] tubi_relate 语句执行失败（数据未写入）: The table 'ses' does not exist
thread 'main' panicked at src\fast_model\gen_model\cata_model.rs:6253:25:
写入 tubi_relate 失败（语句级错误）: The table 'ses' does not exist
🛰️ sidecar 模型生成 job 6fd7e350 status=failed, exit_code=101
```

崩溃发生前管线一切正常：预检通过、pe_transform 现场刷新 2736 节点成功、
16 个 BRAN / 23 个唯一 CATA / 91 个实例几何生成完成；失败只发生在
`tubi_relate` RELATE 批量 SQL 的语句级校验。

## Root Cause Chain

1. **SQL 依赖**：`tubi_relate` 的 RELATE 语句末尾携带 `dt=fn::ses_date(<pe>)`
   （`src/fast_model/gen_model/cata_model.rs` 三处构造点：约 5250 / 5732 / 6101 行）。
2. **函数实现**：`fn::ses_date` 定义在 `resource/surreal/common.surql`（约 268 行），
   内部 `type::record('ses', [$pe.dbnum, $pe.sesno])` 后 `select ... from only $id`，
   需要 `ses` 表存在。同文件 `fn::ses_data`（约 262 行）与历史节点查询
   （约 252 行）同样依赖该表。
3. **表的来源**：`ses` 表仅在 `sync_history=true` 的解析流程中被填充
   （`pdms_inst.rs` ses_keys 写入路径）。quick deploy 站点的解析不开启
   sync_history，站点库从未创建过 `ses` 表。
4. **修复落空**：2026-06-11 提交 `e85a9031` 已在
   `src/fast_model/utils.rs::ensure_surreal_init()` 中加入
   `DEFINE TABLE IF NOT EXISTS ses SCHEMALESS;` 的空表保障，但该函数只挂在
   三个入口（CLI 嵌入式模式 `cli_modes.rs:539`、闭包校验
   `cata_closure_verify.rs:199`、rkyv transform 缓存 `transform_rkyv_cache.rs:206`）。
   模型生成主流程走 `run_app → aios_core::init_surreal`，不经过
   `ensure_surreal_init()`，空表保障从未执行。
5. **版本行为放大**：release 包捆绑 SurrealDB **3.2.0-nightly**
   （站点 `logs/surreal.log` 可证），对缺表查询直接抛
   `The table 'ses' does not exist`；开发机源码版站点用
   `D:\Rust\.cargo\bin\surreal.exe`（3.1 系），对缺表查询容忍返回 none。
   因此同一份代码：源码版 7997 站点生成成功，release 包 250160 站点崩溃。
6. **错误显形**：`cata_model.rs:6242-6256` 此前已修复"语句级错误被静默吞掉"
   （`response.check()` 后 panic），让本缺陷以 exit_code=101 暴露——
   该 fail-loud 行为是正确的，本 spec 不回退它。

## Scope

- `resource/surreal/common.surql`：在 `fn::ses_data` / `fn::ses_date` 定义之前
  增加 `DEFINE TABLE IF NOT EXISTS ses SCHEMALESS;`，作为唯一权威空表保障。
- 保留 `src/fast_model/utils.rs::ensure_surreal_init()` 中既有 DEFINE 作为双保险，
  不删除、不扩散到更多调用点。
- 已发布 release 包的热修路径：替换包内 `resource/surreal/common.surql`
  后重跑生成任务（common.surql 在每次数据库初始化时由
  rs-core `execute_surql_files_on_db()` 重新执行，DEFINE 幂等生效，
  无需重编二进制、无需手工连库建表）。
- 验证范围：release 包站点 `quicktest-250160-8080`（3.2.0-nightly 路径）、
  源码版站点 `quicktest-7997-8080`（3.1 路径）、空库 `fn::ses_date` 最小冒烟。

## Non-Goals

- 不修改 `tubi_relate` 的 `dt=fn::ses_date(...)` 语义：quick deploy 站点
  `ses` 为空表时 `dt` 恒为 none，可接受；sync_history 站点仍能取到真实会话日期。
- 不回退 `cata_model.rs` 的语句级错误 fail-loud 行为。
- 不在本 spec 内调整 release 包捆绑的 SurrealDB 版本（3.2.0-nightly），
  版本行为差异仅记录为已知事项（见 Known Issues）。
- 不把 `ensure_surreal_init()` 接入生成主流程（避免与 run_app 入口的
  WS router 竞争问题再次纠缠，见该函数 spec 006 注释）。
- 不做存量站点的批量 DB 迁移；存量站点在下一次进程初始化加载
  common.surql 时自然获得空表。

## Known Issues（记录，不在本 spec 内处理）

- SurrealDB 3.1 与 3.2.0-nightly 对"查询不存在的表"的行为不一致：
  3.1 容忍返回 none，3.2.0-nightly 抛语句级错误。release 包（捆绑
  3.2.0-nightly）与开发机（PATH 3.1）跑同一份代码可能出现
  "只在 release 包复现"的故障。涉及 `fn::*` 中所有按名查表的函数，
  后续如再遇同类故障应优先检查表存在性前提。

## Acceptance Criteria

1. 修复后的 `common.surql` 在空库上执行后，`ses` 表存在；
   对存在的 pe 记录调用 `fn::ses_date`（ses 无对应记录）返回 none，不报错
   ——在 3.2.0-nightly 下验证。
2. release 包热修后重跑 `quicktest-250160-8080` 生成任务：
   `tubi_relate` 写入成功（16 条 TUBI 关系），sidecar job `job_done:
   status=succeeded, exit_code=0`，任务 Completed。
3. 源码版站点 `quicktest-7997-8080`（3.1 路径）生成回归通过，
   `tubi_relate` 行为不变。
4. `resource/surreal/common.surql` 与 `dist` release 包内同名文件内容一致。
