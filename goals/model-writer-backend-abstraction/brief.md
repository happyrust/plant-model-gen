# ModelWriter 完整后端抽象

## Outcome

将当前 ModelWriter 基础写入 trait 收口为完整模型写入后端生命周期边界，并通过 CLI JSON 与 web_server POST 验证。

## Context

- 当前主线代码已有 `src/fast_model/gen_model/model_writer.rs`，其中 `ModelWriter` 只覆盖 base `ShapeInstancesData` batch 写入：`prepare`、`write_batch`、`finish`。
- 已有实现：`SurrealModelWriter` 与 `DrainOnlyWriter`；`orchestrator.rs` 的 base writer 阶段已通过 trait 调用 `write_batch`。
- 仍有大量模型持久化职责留在 `src/fast_model/gen_model/orchestrator.rs` 直接调用 Surreal 相关 helper，包括 mesh 结果持久化、AABB/PTS 回写、`inst_relate_aabb`、missing negative relation reconcile、boolean worker 调度。
- `runs_downstream_pipeline()` 当前存在但未真正驱动流程，是需要收口或删除的死接口风险。
- 用户明确要求下一步计划使用 Plannotator 定制，并选择：完整后端抽象、计划通过后立即实施、验证同时使用 CLI JSON 与 web_server POST。
- 仓库 `AGENTS.md` 明确约束：不要使用 test 或编译 test；针对 `web_server` 要运行服务后用 POST 测试；针对 aios-database 使用 CLI + JSON 验证。

## Constraints

- 默认 `ModelWriterMode::Surreal` 行为必须保持兼容，不能改变当前正常生成链路的可观察结果。
- `ModelWriterMode::DrainOnly` 不得写入、删除或清理 SurrealDB 中的模型数据；所有持久化/破坏性阶段必须 NoOp 并记录 skipped reason。
- 不运行 Rust unit/integration tests，也不编译 test target；验证以 CLI JSON、`cargo check`/等价静态检查、web_server POST 为主。
- 不引入真实 DuckLake/Parquet writer 实现；本目标只预留接口，避免扩大范围。
- 不做远端 push、PR、数据库破坏性迁移、公开 CLI 不兼容变更，除非用户明确批准。
- 修改应匹配现有 Rust 风格、错误传播方式、日志风格和 feature guard 模式。

## Non-Goals

- 不实现生产级 DuckLake/Parquet/compare backend。
- 不重写模型生成算法、mesh 计算算法、boolean 算法本身。
- 不把整个 orchestrator 拆成新框架；只收口持久化后端边界。
- 不新增或依赖 Rust test 作为验收手段。
- 不改部署脚本、远端服务器配置或 CI 配置，除非执行中发现这是完成验证的必要条件并经用户确认。

## Ask Before

- git commit、push、创建 PR。
- 删除、移动、批量重命名现有文件。
- 修改公开 CLI 参数且造成不兼容。
- 引入新外部依赖或启用重型 feature。
- 任何可能删除或重写现有 SurrealDB 模型数据的 cleanup 语义变更。
- 将接口预留升级为真实 DuckLake/Parquet/compare backend 实现。
- 如果验证需要连接远端服务器或修改运行环境。

## Done Means

完成一个可实施、已通过 Plannotator 审核的目标包，并在后续实施中使所有模型写入阶段通过统一 `ModelWriterBackend` 生命周期执行；Surreal 默认行为兼容，DrainOnly 安全无副作用，CLI JSON 与 web_server POST 均提供可审计证据并记录到 `progress.jsonl`。
