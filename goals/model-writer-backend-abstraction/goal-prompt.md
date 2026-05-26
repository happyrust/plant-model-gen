# Codex Goal Prompt: ModelWriter 完整后端抽象

After every critical document in this folder is approved with Plannotator, paste or set this goal:

```text
/goal 将当前 ModelWriter 基础写入 trait 收口为完整模型写入后端生命周期边界，并通过 CLI JSON 与 web_server POST 验证。

使用 `goals/model-writer-backend-abstraction/` 作为唯一长期计划来源：
- 先读 `brief.md`，确认任务目标、上下文、约束、非目标和必须询问的事项。
- 按 `plan.md` 实施：把当前窄版 `ModelWriter::write_batch()` 抽象升级为完整 `ModelWriterBackend` 生命周期边界；让 Surreal 保持默认兼容行为，让 DrainOnly 完整实现安全 NoOp + 统计；不要在本目标中真实实现 DuckLake/Parquet/compare backend。
- 按 `verification.md` 验证：不运行或编译 Rust tests；使用静态检查、CLI JSON 验证，以及启动 web_server 后用 POST 验证。
- 每完成一个实质步骤或验证，都向 `goals/model-writer-backend-abstraction/progress.jsonl` 追加证据，包含时间、命令/文件、结果、关键日志或 artifact 路径。
- 遇到 `blockers.md` 中列出的情况必须暂停并询问用户，尤其是 git commit/push/PR、删除/移动文件、不兼容 CLI 变更、破坏性 cleanup、远端服务器或真实 DuckLake/Parquet 实现。

完成标准：
- `ModelWriterBackend` 覆盖 init、cleanup、base batch、mesh persist、inst_relate_aabb、missing neg reconcile、boolean bridge、finalize。
- `orchestrator.rs` 不再直接承担模型持久化 helper 调用。
- 默认 Surreal 路径行为兼容，有 web_server POST 证据。
- DrainOnly 不写入/删除 SurrealDB，有 CLI JSON skipped 证据。
- `runs_downstream_pipeline()` 被实际使用或删除。
- 所有验收项都有 `progress.jsonl` 证据。

不要在所有验收项都有真实证据前宣称完成。
```
