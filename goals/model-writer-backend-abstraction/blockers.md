# Blockers: ModelWriter 完整后端抽象

## Open Questions

- 最终 CLI JSON 验证入口使用新 binary、现有 CLI 子命令，还是 admin endpoint 的 dry/verify 模式？执行前可先按最小改动选择；若需要新增公开 CLI 参数且不兼容，必须询问用户。
- web_server POST 验证应使用哪个现有 endpoint：admin task、generation、或其他项目内已有入口？执行时需先检索并记录选择依据。
- `runs_downstream_pipeline()` 是保留为实际控制 DrainOnly 快速路径，还是删除？若两种方案代价接近，优先删除死接口；若已有明确调用价值则保留并接入。
- `RecordingBackend` 是否在本目标实现？默认只在轻量、无重依赖情况下实现；若会扩大范围，则记录为后续目标。

## Stop And Ask

- 需要 git commit、push、创建 PR。
- 需要删除、移动、批量重命名现有源码文件。
- 需要修改公开 CLI 参数且造成不兼容。
- 需要真实实现 DuckLake、Parquet 或 compare backend。
- 需要执行可能删除/覆盖 SurrealDB 现有模型数据的 cleanup 验证。
- 需要连接远端服务器、修改系统环境、安装依赖或改 CI。
- 发现默认 Surreal 行为无法保持兼容，需要产品/架构取舍。

## Dangerous Or High-Risk Actions

- 数据库 cleanup、删除模型关系、重建 SurrealDB 表。
- 远端服务器操作或任何需要凭据的操作。
- git push / force push / PR 创建。
- 新增重型依赖或启用明显增加编译负担的 feature。
- 批量格式化或大范围重构与本目标无关文件。

## Known Blockers

- 历史记录显示 `cargo check --lib` 可能受 NASM/toolchain 环境阻塞；执行时先尝试项目推荐 feature 组合，若失败需记录环境证据并使用 CLI JSON + web_server POST 补充验证。
- web_server POST 的具体 endpoint/payload 尚未最终确定；实施阶段需检索 `src/web_server` / `src/web_api` / admin handlers 后确定。
- 当前仓库存在未必相关的工作区文件；执行 commit 前必须按用户确认另行处理，不能自动提交。
