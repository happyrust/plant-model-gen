# my-mcp-3 会话协作架构与原理

本文档描述当前工作区 `my-mcp-3` MCP 服务的会话协作模型。配套架构图：

- SVG：[`my-mcp-3-architecture.svg`](./my-mcp-3-architecture.svg)
- PNG：[`my-mcp-3-architecture.png`](./my-mcp-3-architecture.png)

## 1. 架构概览

`my-mcp-3` 是一个面向多会话协作的 MCP 服务。它把一次 Agent 回合拆成四类职责：

1. **会话心跳与消息消费**：`check_messages` 是回合结束时的强制心跳，也是下一条入站消息的消费入口。
2. **多会话通信**：`list_sessions`、`send_to_session`、`broadcast`、`send_message` 负责查看会话、点对点发送、广播和接收文本/图片。
3. **任务派发与汇报**：`dispatch_task`、`report_task`、`query_tasks` 支持主控 Agent 把结构化任务派给 worker，并跟踪任务状态。
4. **进度与上下文存档**：`save_progress`、`load_progress`、`send_progress` 用于保存任务断点、关键发现、关键文件和面板进度。

```text
用户 / Cursor
  -> Agent 执行循环
  -> MCP 工具 schema 校验
  -> my-mcp-3 服务
  -> 入站消息队列 / 任务协调器 / 进度存档
  -> 下一轮 Agent 处理
```

## 2. 核心工具职责

### `check_messages`

`check_messages` 是最关键的回合边界工具。它的参数契约要求：

```json
{
  "turn_complete": true
}
```

它有两个作用：

- **确认当前回合完成**：Agent 已经完成用户可见回复，且没有后续普通工具调用。
- **消费下一条消息**：如果工具返回内容，该内容就是当前 MCP 会话的下一条入站用户消息，必须直接处理，而不是当成日志摘要。

### 任务协作工具

- `list_sessions`：查看所有 MCP 会话状态，用于决定可用 worker。
- `dispatch_task`：向指定会话派发结构化任务，包含目标会话号、任务描述、优先级和上下文。
- `report_task`：worker 向主控汇报 `working`、`done` 或 `failed`，并提供执行总结。
- `query_tasks`：按任务 ID 或状态过滤查询任务进度。

### 消息通信工具

- `send_message`：接收文本和图片，作为当前会话输入。
- `send_to_session`：向指定会话发送点对点消息。
- `broadcast`：向所有活跃会话广播消息。
- `ask_question`：向用户发起简短问题。

### 进度存档工具

- `save_progress`：保存任务、已完成项、待办项、关键发现和关键文件。
- `load_progress`：恢复历史存档。
- `send_progress`：把当前进度同步到插件面板，不等同于持久化。

## 3. 回合原理

一次标准回合如下：

1. 用户在 Cursor 或 MCP 会话中发起输入。
2. Agent 读取必要上下文，按工具 schema 准备参数。
3. Agent 执行代码检索、文件编辑、任务派发或文档生成等工作。
4. Agent 先完成用户可见回复。
5. 最后调用 `check_messages({ "turn_complete": true })`。
6. 如果 `check_messages` 返回新消息，Agent 立即把它当作下一条用户输入继续处理。

这个设计把“回合结束”和“下一条消息获取”合并在一个工具里，避免多会话场景中出现队列阻塞或主控与 worker 状态不同步。

## 4. 任务派发原理

主控会话适合处理拆解、调度和收敛：

1. 主控调用 `list_sessions` 确认可用 worker。
2. 主控调用 `dispatch_task` 给空闲 worker 派发任务。
3. worker 执行任务，过程中可用 `report_task(status="working")` 汇报进展。
4. worker 完成后调用 `report_task(status="done" | "failed")`。
5. 主控通过自动通知或 `query_tasks` 汇总状态。
6. 所有子任务收敛后，主控回复用户并再次调用 `check_messages`。

任务派发适合相互独立的工作，例如并行代码审查、不同模块调研、多个方案比较。若任务共享大量可变状态，应由主控串行推进，避免 worker 之间互相覆盖结论。

## 5. 进度记忆原理

`save_progress` 存的是“任务级断点”，不是长期项目规范。建议保存：

- 当前任务定义；
- 已完成的可验证子项；
- 待完成的可验证子项；
- 结论性 findings；
- 下次恢复必须看的关键文件；
- 必要的上下文说明。

任务完成后，如果某些结论已稳定且对团队长期有用，应迁移到项目文档、`AGENTS.md` 或 `.cursor/rules`，而不是长期留在 progress 里。

## 6. 安全与一致性原则

- **schema-first**：调用 MCP 工具前先读取工具描述，避免参数遗漏或类型错误。
- **final heartbeat**：当前回合所有用户可见内容完成后，最后一步调用 `check_messages`。
- **message-as-input**：`check_messages` 返回内容时，它就是下一条用户输入。
- **single-owner task**：一个 worker 一次只处理一个派发任务，完成后汇报。
- **progress is temporary**：进度存档服务于恢复上下文，不替代项目级规则或架构文档。
- **正文与面板分离**：`reply` / `send_progress` 可同步到插件面板，但 Cursor 对话窗口仍是主要用户可见回复位置。
