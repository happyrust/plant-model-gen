# Claude Code MCP 配置清单

> 整理时间：2026-02-11
> 配置来源：`~/.claude/mcp_config.json`、`~/.claude/settings.json`、`~/.claude/plugins/installed_plugins.json`

---

## 一、MCP Servers（`mcp_config.json`）

### 1. ace-tool（Augment 代码上下文引擎）

| 项目 | 值 |
|------|-----|
| command | `ace-tool-rs` |
| args | `--base-url https://aug.gptclubapi.xyz/ --token <REDACTED>` |
| disabled | false |
| 用途 | 代码库语义搜索，自然语言检索代码片段 |

### 2. github（GitHub MCP Server）

| 项目 | 值 |
|------|-----|
| command | `npx -y @modelcontextprotocol/server-github` |
| env | `GITHUB_PERSONAL_ACCESS_TOKEN=<REDACTED>` |
| disabled | false |
| timeout | 60s |
| 用途 | GitHub issue/PR/repo 操作 |

### 3. linear（Linear 项目管理）

| 项目 | 值 |
|------|-----|
| command | `C:\Users\Administrator\AppData\Local\pnpm\linear-mcp-server.CMD` |
| env | `LINEAR_API_KEY=<REDACTED>` |
| 用途 | Linear issue/project/cycle 管理 |

### 4. ida-pro-mcp（IDA Pro 逆向分析）

| 项目 | 值 |
|------|-----|
| command | `C:\Python314\python.exe` |
| args | IDA Pro MCP server.py 脚本路径 |
| disabled | false |
| timeout | 1800s (30min) |
| 用途 | IDA Pro 反编译、反汇编、函数分析、内存读取等逆向工程操作 |
| 自动授权 | 全部工具（35+ 个操作） |

### 5. x64dbg-mcp（x64dbg 动态调试）

| 项目 | 值 |
|------|-----|
| command | `C:\Python314\python.exe` |
| args | `D:\reverse\x64dbg-mcp\src\x64dbg.py` |
| disabled | false |
| timeout | 600s (10min) |
| 用途 | x64dbg 动态调试：寄存器/内存读写、断点、单步、汇编、模块列表等 |
| 自动授权 | 全部工具（25 个操作） |

### 6. dnspy-mcp（dnSpy .NET 逆向）

| 项目 | 值 |
|------|-----|
| command | `D:\reverse\dnSpy-net-win64\MCPProxy\MCPProxy-STDIO-to-SSE.exe` |
| args | `http://localhost:3003` |
| disabled | false |
| timeout | 600s (10min) |
| 用途 | .NET 程序反编译与分析（通过 SSE 代理连接 dnSpy） |

---

## 二、Plugins（已安装插件）

| 插件 | 版本 | 安装时间 | 说明 |
|------|------|----------|------|
| claude-mermaid | 1.2.0 | 2026-01-25 | Mermaid 图表实时预览与保存 |
| oh-my-claudecode (omc) | 3.8.14 | 2026-01-30 | Claude Code 增强插件 |
| superpowers | 4.2.0 | 2026-02-01 | 高级工作流技能集（代码审查、TDD、调试等） |
| claude-mem | 9.1.1 | 2026-02-08 | 跨会话持久化记忆/语义搜索 |

启用状态（`settings.json` → `enabledPlugins`）：

- `claude-mem@thedotmack`: ✅
- `claude-mermaid@claude-mermaid`: ✅
- `superpowers@claude-plugins-official`: ✅
