# MBD Pipe Dims Followups Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在 `dims[]` 方案 1A 已落地的前提下，继续按顺序完成：文本规范对齐（RUS-170）与端到端截图验收（RUS-171），并保持“前端负责屏幕布局/避让、后端只提供语义锚点与字段”的分层。

**Architecture:** 后端继续输出同一 `dims[]`，前端按 `dims.kind` 过滤/分色渲染；文本规范以一个集中格式化函数产出 `text`，避免散落 `format!()`；截图验收用固定 refno + 固定 query params + 记录 dbno/batch_id，形成可复现证据链。

**Tech Stack:** Rust（axum/web_server）、SurrealDB（source=db）、plant3d-web（Three 标注渲染）。

---

## Context / 已完成（2026-02-06）

- RUS-167/RUS-168（plant3d-web）已完成：请求透传 + 按 `dims.kind` 分组/过滤 + 颜色区分。
- RUS-169（后端）已完成：`source=db` 下补齐 `tubi_relate.arrive_axis/leave_axis` 读取，用于 `kind=port`（不再退化为 start/end）。

---

### Task 1: RUS-170 文本规范对齐（单位/小数位/overall 文本语义）

**Files:**
- Modify: `src/web_api/mbd_pipe_api.rs`

**Step 1: 写一个最小“格式化函数”并加单测（先做可测的纯函数）**

目标：把 dims 的 `text` 生成集中到一个函数里，确保：
- 不输出 `NaN/inf`
- 不输出 `-0`
- 小数位策略统一（暂定 0 位，等确认后可调整）

建议新增函数（示例签名）：

```rust
fn format_dim_length_text_mm(length: f32) -> String
```

并在 `#[cfg(test)]` 中补 3-4 个 case：
- `10.4 -> "10"`
- `0.49 -> "0"`（或按最终策略）
- `-0.0 -> "0"`
- `NaN -> "0"`

**Step 2: 将所有 dims.text 改为调用该函数**

覆盖范围：
- `kind=segment`
- `kind=chain`
- `kind=overall`
- `kind=port`

**Step 3: 明确 overall 的 text 语义（最小实现）**

默认策略（建议）：仍只输出数值（例如 `"1234"`），由前端在面板/标签上通过 `[总长]` 显示语义；避免在 3D label 内塞入“总长/单位”导致屏幕拥挤。

若后续需要 MBD/PML 风格（如 `L=1234` / `TOT 1234`），在该函数中加一个可选参数或另写 `format_overall_text_mm()`，不要在主流程散落判断。

**Step 4: 验证**

Run（只做编译校验，避免受当前仓库既有测试失败影响）：
- `cargo check -q --bin web_server --features web_server`

Expected：
- 编译通过，无新增 clippy 级错误。

---

### Task 2: RUS-171 端到端截图验收（Model Testing）

**Files:**
- (No code required) 主要输出截图与记录信息；如需脚本辅助，再新增 `scripts/`。

**Step 1: 选取 1-2 个典型分支 refno**

建议覆盖两类：
- 简单直线段较多（便于验证 chain/overall）
- 含明显元件（阀/弯头等）（便于观察 port dims 的端口点差异）

**Step 2: 固定 query params 并截图**

建议参数（示例）：
- `source=db`
- `include_dims=true`
- `include_chain_dims=true`
- `include_overall_dim=true`
- `include_port_dims=true`
- `include_welds=true`
- `include_slopes=true`

记录：
- refno
- dbno（若前端有传）
- batch_id（若前端有传）
- 返回 stats（segs/dims/welds/slopes）

**Step 3: 提交验收材料到 Linear**

在 RUS-171 留言/附件中附：
- 2-4 张截图（至少包含 port dims 与 chain/overall 同屏）
- 上述记录信息

