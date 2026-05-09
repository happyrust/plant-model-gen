# 异地协同 UI 浏览器验证与收尾计划（2026-04-27）

## 1. 背景

上一轮已完成异地协同后端与静态入口的主干验证：

- `web_server` 本机服务可通过 `127.0.0.1:3100` 访问，`admin/admin` 登录可获取 token。
- `/api/remote-sync/*` 主干接口已完成 HTTP 冒烟，env/site CRUD、运行时状态、日志、v2 任务/参数接口可用。
- `/admin/static/` 与 `/admin/static/#/collaboration` 静态入口返回 200。

剩余风险集中在浏览器真实交互层：登录跳转、协同工作台数据加载、站点排序、日志关键词高亮、抽屉表单与确认弹窗是否符合预期。

## 2. 下一步方案

采用“浏览器真实路径 + 非破坏性操作优先”的验证方案：

1. 启动真实 `web_server` 或复用已运行的 `127.0.0.1:3100`。
2. 用浏览器访问 `/admin/static/#/collaboration`，验证未登录会进入登录页，登录后回到协同页。
3. 检查协同页四个主区域：
   - 拓扑/概览：协同组列表、运行时状态、统计卡片可渲染。
   - 站点：排序按钮在名称、健康度、角色、文件数之间可切换，重复点击同字段可切升降序。
   - 洞察/任务：active tasks、failed tasks 区块能加载空态或真实数据。
   - 日志：关键词搜索支持普通字符串与特殊正则字符，不报错且高亮安全。
4. 只执行不会改变运行态的动作：打开/关闭新增或编辑抽屉、HTTP/MQTT/站点诊断、取消删除确认。
5. 暂不执行 `applyEnv`、`activateEnv`、`stopRuntime`，避免写入 `DbOption.toml` 或影响 watcher/MQTT 运行态。

## 3. 执行步骤

1. 环境确认：

   ```powershell
   Invoke-WebRequest http://127.0.0.1:3100/admin/static/ -UseBasicParsing
   ```

2. 登录与路由：
   - 打开 `http://127.0.0.1:3100/admin/static/#/collaboration`。
   - 未登录时确认跳转到登录页。
   - 使用本地管理员账号登录后确认回到协同工作台。

3. UI 主链路：
   - 确认协同组列表、详情头、拓扑/概览、站点、洞察、日志区块无白屏。
   - 切换 `#topo`、`#sites`、`#insight`、`#logs` 四个标签。
   - 在站点区依次点击名称、健康度、角色、文件数排序按钮。
   - 在日志区输入 `sync`、`.`、`[abc]`、`a+b` 等关键词，确认过滤和高亮不触发异常。

4. 非破坏性交互：
   - 打开新增协同组抽屉后关闭，不提交。
   - 打开新增站点抽屉后关闭，不提交。
   - 如有可删除目标，只验证删除确认弹窗的取消路径。

5. 记录结果：
   - 浏览器 URL 与登录跳转结果。
   - 关键区域是否渲染。
   - 排序/高亮/抽屉/取消删除是否通过。
   - 控制台或网络错误的首个失败点。

## 4. 验收标准

- [ ] `/admin/static/#/collaboration` 可通过浏览器打开并完成登录跳转闭环。
- [ ] 协同工作台主区域无白屏，核心接口失败时有可见错误提示或空态。
- [ ] 站点排序四个字段可点击，升降序状态可见。
- [ ] 日志关键词高亮对特殊正则字符输入不报错。
- [ ] 新增/编辑抽屉能打开与关闭；删除确认可取消且不发出删除请求。
- [ ] 未执行 `apply`、`activate`、`stop` 等运行态写操作。

## 5. 风险与边界

- 当前环境可能没有可成功诊断的 metadata 站点或 MQTT broker，诊断失败只记录为环境限制，不作为 UI 阻断。
- 如果没有站点或日志数据，先记录空态表现；需要交互数据时再创建临时 env/site，并在验证后清理。
- 如浏览器验证发现缺陷，优先做最小修复并重新执行对应冒烟，不扩展无关 UI 重构。

## 6. 执行记录

执行时间：2026-04-27

| 项目 | 结果 | 备注 |
|---|---|---|
| 计划文件创建 | 通过 | 本文件已创建 |
| 服务可达性检查 | 通过 | `GET /admin/static/` 返回 `StatusCode=200; Length=885`；首次命令因 PowerShell 变量转义失败，未触达页面 |
| 浏览器登录与协同页加载 | 通过 | 未登录访问 `/admin/static/#/collaboration` 跳到登录页，登录后回到协同工作台 |
| 临时验证数据 | 已清理 | 创建 `ui-smoke-20260427-224956` 与 3 个站点用于 UI 验证，结束后已删除 env/site |
| 站点排序与日志高亮 | 通过 | 名称排序可在 `Alpha/Beta/Gamma` 与反序之间切换；补充 1 条临时日志后，关键词 `[abc]+.` 可过滤并高亮 `/tmp/[abc]+.json` 中的匹配片段 |
| 非破坏性交互 | 通过 | 新增站点抽屉可打开，取消后关闭；未执行 `apply/activate/stop` |
| Tab 路由缺陷修复 | 通过 | 点击内部 Tab 曾把 Vue Hash 路由改成 `#/sites`；已改为 `?tab=sites/logs/...` 并通过构建与浏览器复验 |
| 日志高亮临时数据 | 已清理 | 创建 `ui-log-smoke-20260427-225610`、1 个站点与 1 条日志；验证后已删除，`REMAINING_LOG_ROWS=0` |
| metadata 成功诊断 | 通过 | 临时提供 `/admin/static/metadata.json`，`test-http` 返回 `metadata.json 可达`、`code=200`，`metadata` 接口返回 `source=remote_http; entry_count=1`；临时文件与 env/site 已清理 |
| MQTT 成功诊断 | 通过 | 临时使用 `npx aedes-cli --port 1883` 启动本地 broker，`test-mqtt` 返回 `MQTT 连接可达`、`addr=127.0.0.1:1883`；临时 env 已删除，broker 已停止 |

## 7. 修复记录

- 修复文件：`ui/admin/src/views/CollaborationWorkbenchView.vue`
- 问题：协同工作台使用 `location.hash` 保存内部 Tab 状态；在 `createWebHashHistory('/admin/')` 下，点击“站点”会污染 Vue Router 的 hash 路由并跳到 `#/sites`。
- 处理：改为使用路由 query 的 `tab` 字段保存内部 Tab，默认 `topo` 不写 `tab`；监听 `route.query.tab` 同步 `activeTab`。
- 验证：
  - `npm run build` 通过，覆盖 `vue-tsc -b && vite build`。
  - 浏览器点击“站点”后 URL 保持在 `#/collaboration?env=...&tab=sites`。
  - 站点排序、日志特殊字符关键词与真实高亮、新增站点抽屉打开/取消均通过。
