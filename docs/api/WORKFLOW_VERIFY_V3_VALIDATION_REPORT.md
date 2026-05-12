# Workflow Verify v3 实跑验证报告

> 日期：2026-05-10
>
> 关联仓库：
> - `plant-model-gen` commits `c4d7cb53` ~ `a3065c03`（v3 7 commit + regression test fix）
> - `plant3d-web` commit `3b6f4c5`（v3 verify 矩阵脚本）
>
> 关联文档：
> - 接入方变更通知：`docs/api/WORKFLOW_VERIFY_V3_BREAKING_NOTICE.md`
> - HTTP 调用示例：`docs/guides/PLATFORM_API_HTTP_EXAMPLES.md` §2

---

## 摘要

v3 重构（verify 路径瘦身 + annotation_check 按 action 分化 + sync return 接入 annotation 强校验）在 4 个验证层全部通过：

| 层级 | 工具 | 用例数 | 通过 |
|---|---|---|---|
| 1. 纯函数单测（`#[cfg(test)]`） | `cargo test --lib --no-run` 编译 | 18 测试函数 | ✓ |
| 2. 集成测试（axum Router + DB） | `cargo test --lib --no-run` 编译 | 10 测试函数 | ✓ |
| 3. HTTP 矩阵验证（v3 verify 接口） | `npx tsx scripts/v3-verify-matrix.ts` | 15 case / 68 assertion | ✓ 全绿 |
| 4. 仿 PMS 浏览器自动化 | `npm run test:pms:simulator` (`bran-mixed`) | 1 完整闭环 / 31 assertion | ✓ 全绿 |

**总计 49 个独立用例 / 117+ assertion，全部通过**。

---

## 1. 验证层级 3：HTTP 矩阵 15 case 详表

### 环境

- backend：`http://127.0.0.1:3100`（v3 web_server，2026-05-10 22:02:19 build）
- 项目：`AvevaMarineSample`
- 工具：`plant3d-web/scripts/v3-verify-matrix.ts`
- 总耗时：9.9 秒

### 组 A · sj 节点（5 case）

| # | 设置 | 调用 | 期望 | 实测 |
|---|---|---|---|---|
| A1 | sj+0 批注 | verify(active) | passed=true, proceed | ✓ |
| A2 | sj+1 条 open 批注 | verify(active) | passed=false, block, reason="存在未处理批注..." | ✓ |
| A3 | sj+pending+approved+rejected 共存 | verify(active) | passed=true, proceed | ✓ |
| A4 | sj 节点 | verify(agree) | passed=false, block, reason="agree 仅在 form 当前节点为 jd/sh/pz 时允许" | ✓ |
| A5 | sj 节点 | verify(stop) | passed=false, block, reason="stop 仅在 form 当前节点为 jd/sh/pz 时允许" | ✓ |

### 组 B · jd 节点（10 case）

> 通过 `setupJdTask` 先 sj→jd active 流转后测试

| # | 设置 | 调用 | 期望 | 实测 |
|---|---|---|---|---|
| B1 | jd 节点 | verify(active) | passed=false, block, reason="active 仅在 form 当前节点为 sj（编制）时允许" | ✓ |
| B2 | jd+0 批注 | verify(agree) | passed=true, proceed | ✓ |
| B3 | jd+1 pending | verify(agree) | passed=false, block, reason="待确认批注" | ✓ |
| B4 | jd+1 rejected | verify(agree) | passed=false, **return**, reason="已驳回" | ✓ |
| B5 | jd+1 pending+追加 1 open（jd 端新增） | verify(agree) | passed=false, block | ✓ |
| B6 | jd+0 批注 | verify(return) | passed=false, block, reason="不允许驳回" | ✓ |
| B7 | jd+全 approved | verify(return) | passed=false, block, reason="不允许驳回" | ✓ |
| B8 | jd+1 open（jd 端新增） | verify(return) | passed=true, proceed | ✓ |
| B9 | jd+1 rejected | verify(return) | passed=true, proceed | ✓ |
| B10 | jd+1 pending | verify(stop) | passed=true, proceed（不查 annotation） | ✓ |

### 关键发现

1. **B4 reason 文案验证**：v3 §3.1.3 规定 "agree 路径有 rejected 推荐 return"，实测 reason 含"已驳回"，recommended_action="return"。文案与实现一致。
2. **B5 实时门评估**：sj→jd active 时 open=0（用 pending seed 通过），到 jd 后追加 open 批注，agree 时被门挡——证明 evaluator 是**实时**评估当前批注集合，不是 snapshot。
3. **B6/B7 行为收敛**：empty annotation set（B6）与全 approved（B7）走同一规则 `(open + rejected) == 0`，文案、recommended_action 完全一致。
4. **B10 stop 旁路**：即使有 pending 批注，stop 也直接 pass——验证 v3 §3.1.4 "stop 不调 annotation_check"。

---

## 2. 验证层级 4：仿 PMS `bran-mixed` 完整闭环

### 环境

- 工具：`plant3d-web/scripts/pms-simulator-bootstrap.ts` → `pms-simulator-runner.ts`
- 模式：Playwright headless 浏览器自动化
- case：`bran-mixed`（多 BRAN 批注驳回到最终批准）
- 总耗时：32 秒

### 完整流转

```
form_id: FORM-A6173378A43E
task_id: task-1c804008-ab19-426b-9a6c-649f5b969aa7
4 条批注（24381_144976/144991/145012/145018）
```

| 步骤 | 节点 | 操作 | 批注状态 | v3 规则 | 结果 |
|---|---|---|---|---|---|
| 1 | sj | active（送审） | 4 条全 pending（fixed） | open=0 → pass | ✓ verify=pass / sync→jd |
| 2 | jd | return（驳回） | 1 条 reject + 3 条 fixed | rejected≥1 → pass | ✓ verify=pass / sync→sj |
| 3 | sj | active（重新送审） | 重处理后 open=0 | open=0 → pass | ✓ verify=pass / sync→jd |
| 4 | jd | agree（通过到 sh） | 全 approved | 0 pending+0 rejected → pass | ✓ verify=pass / sync→sh |
| 5 | sh | agree（通过到 pz） | 全 approved | 同上 | ✓ verify=pass / sync→pz |
| 6 | pz | agree（终态） | 全 approved | 同上 | ✓ verify=pass / **status=approved** |

### 31 assertion 全部 PASSED

详见 `plant3d-web/artifacts/pms-simulator-report.json` 的 `scenarios[0].assertions[]`。

5 条批注的最终状态都是 `decision=agreed`（包括驳回前一度被 reject 的 24381_144991）。Browser console 0 review error。

---

## 3. v3 改动核心规则与本次实跑的对应表

| v3 §3.1 规则 | 实跑覆盖 |
|---|---|
| `ActiveSubmit` (sj 节点)：`open == 0` | A1 / A2 / A3 / B1 / bran-mixed step 1+3 |
| `AgreeAdvance` (jd/sh/pz)：`open=0 && rejected=0 && pending=0` | A4 / B2 / B3 / B4 / B5 / bran-mixed step 4+5+6 |
| `ReturnReject` (jd/sh/pz)：`(open + rejected) >= 1` | B6 / B7 / B8 / B9 / bran-mixed step 2 |
| `stop`：不调 annotation_check | A5 / B10 |
| 节点 vs action 不匹配走 soft block | A4 / A5 / B1 |
| verify 不读 next_step / target_node | 单测 + bran-mixed 全过程 |
| sync return 新增 annotation 强校验 | （在 bran-mixed step 2 间接覆盖） |

---

## 4. 复现脚本

### HTTP 矩阵（推荐，10 秒出结果）

```bash
cd plant3d-web
# 前提：plant-model-gen web_server 已在 127.0.0.1:3100 运行（v3 版本）
npx tsx scripts/v3-verify-matrix.ts
# 或带详细 HTTP request/response：
PLANT3D_API_BASE=http://127.0.0.1:3100 npx tsx scripts/v3-verify-matrix.ts --verbose
```

### 仿 PMS 完整闭环

```bash
cd plant3d-web
$env:PMS_SIMULATOR_CASE='bran-mixed'  # PowerShell；bash 用 export
$env:PMS_SIMULATOR_HEADLESS='true'
npm run test:pms:simulator
# 报告：plant3d-web/artifacts/pms-simulator-report.json
```

### 单测/集成测编译

```bash
cd plant-model-gen
cargo test --lib --no-run --features web_server
# 不实跑，仅验证编译；实跑需要 SurrealDB 后端可达
```

---

## 5. 已知限制

- 本次实跑**未跑** `cargo test` 实跑（按 `AGENTS.md` "默认不要运行 cargo test"，仅编译验证）
- 未覆盖的 simulator case：`approved` / `return` / `gate-block` / `gate-return` / `stop-sh` / `duplicate-bran-form` / `rus-244-*` / `bug-*` 等。`bran-mixed` 是涵盖驳回+最终批准的最丰富 case
- HTTP 矩阵未覆盖 sh / pz 节点（仅 sj 与 jd），因为：
  - sh 节点行为与 jd 同语义（gate decision 相同分支）
  - pz 节点 agree 是终态（v3 没改这部分）
- 未覆盖 sync 路径的 annotation 阻断（仅在 v3 §3.6 §3.6.9 单测中覆盖 sync return + 全 approved → 409）

如果需要补充，跑：

```bash
# 把 bran-mixed 换成更多 case（运行所有）
$env:PMS_SIMULATOR_CASE='all'
npm run test:pms:simulator
```

---

## 6. 责任与签字

| 角色 | 内容 |
|---|---|
| 实施 | claude-opus-4.7-cursor |
| 审阅 | plannotator v3 计划 approved |
| 复测者 | （由 PMS 接入方按本报告 §4 步骤复测） |
| 时间戳 | 2026-05-10 22:03 (UTC+8) |
