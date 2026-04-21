# 三维校审流程 · Mermaid 配图源

配图配套文档见 `../三维校审流程与批注对话开发文档.md`。所有 `*.mmd` 文件均通过 `mermaid` 技能 `validate.sh` 验证（Mermaid CLI 渲染通过）。

## 导出单张 SVG

```bash
# 需要安装: npx (Node.js) + puppeteer(首次自动下载 Chromium)
/Users/dongpengcheng/.agents/skills/mermaid/tools/validate.sh \
  01-architecture.mmd \
  ./01-architecture.svg
```

## 批量导出 PNG (1920px 宽)

```bash
for f in *.mmd; do
  base="${f%.mmd}"
  /Users/dongpengcheng/.agents/skills/mermaid/tools/validate.sh "$f" "$base.svg"
  rsvg-convert -w 1920 "$base.svg" -o "$base.png"
done
```

## 文件清单

| 序号 | 文件 | 类型 | 说明 |
|------|------|------|------|
| 01 | `01-architecture.mmd` | flowchart | 系统架构（前/后端/数据/外部系统） |
| 02 | `02-workflow-state.mmd` | stateDiagram | 任务级工作流状态机（4 节点 × 6 状态） |
| 03 | `03-annotation-lifecycle.mmd` | stateDiagram | 批注生命周期（处理/决定） |
| 04 | `04-er-model.mmd` | erDiagram | 数据模型 ER 图（含内嵌 JSON） |
| 05 | `05-initiate-sequence.mmd` | sequenceDiagram | 发起校审 + 立即 submit 时序 |
| 06 | `06-submit-return-sequence.mmd` | sequenceDiagram | 提交/驳回 时序（alt 分支） |
| 07 | `07-comment-thread-sequence.mmd` | sequenceDiagram | 批注对话 + 处理动作 时序 |
| 08 | `08-record-snapshot-idempotent.mmd` | flowchart | 确认记录幂等快照流程 |
| 09 | `09-history-read.mmd` | flowchart | 四种历史读取口径 |
| 10 | `10-pms-integration.mmd` | sequenceDiagram | PMS 入站集成时序 |
| 11 | `11-component-tree.mmd` | flowchart | Vue 组件树 + Composable 分层 |
| 12 | `12-rbac-matrix.mmd` | flowchart | 角色 → 操作 RBAC 矩阵 |
