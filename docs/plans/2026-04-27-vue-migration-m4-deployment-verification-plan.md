# Vue 迁移 M4 部署验证计划（2026-04-27）

## 1. 背景

Vue 迁移第二批已完成 2 个交互增强：

- `GroupLogsPanel.vue`：同步日志关键词搜索结果高亮，使用 `escapeHtml + mark` 避免 `v-html` 注入风险。
- `GroupSitesPanel.vue`：协同站点卡片支持按名称、健康度、角色、文件数排序。

当前剩余待办为 **M4 部署验证**，目标是确认 `ui/admin` 在现有代码基线上仍能完成生产构建，并且构建产物可被静态预览服务加载。

## 2. 验证范围

本轮只覆盖前端部署面，不扩展新功能：

1. 类型与构建：执行 `npm run build`，覆盖 `vue-tsc -b` 与 `vite build`。
2. 静态产物冒烟：执行 `npm run preview`，访问预览首页确认 HTML 可加载。
3. 人工检查重点：
   - 同步日志关键词高亮在无关键词、有关键词、特殊正则字符输入下不报错。
   - 站点排序按钮能在四个字段间切换，重复点击同字段可切换升降序。
   - 构建过程无 TypeScript、Vue 模板或 Vite 打包错误。

不执行单元测试或 cargo test；如需后端联调，按项目约定启动真实 `web_server` 后用 HTTP/POST 验证。

## 3. 执行步骤

1. 在 `ui/admin` 执行：

   ```powershell
   npm run build
   ```

2. 构建成功后启动静态预览：

   ```powershell
   npm run preview -- --host 127.0.0.1 --port 4173
   ```

3. 用 HTTP 请求确认预览首页返回成功：

   ```powershell
   Invoke-WebRequest http://127.0.0.1:4173/ -UseBasicParsing
   ```

4. 记录结果：
   - 构建是否成功
   - 预览服务是否启动
   - 首页 HTTP 状态码
   - 如失败，记录首个错误与下一步修复入口

## 4. 实测结果

执行时间：2026-04-27

| 项目 | 结果 | 备注 |
|---|---|---|
| `npm run build` | 通过 | `vue-tsc -b && vite build` 0 error，Vite 构建耗时约 844ms |
| `npm run preview -- --host 127.0.0.1 --port 4173` | 通过 | 预览服务成功监听 `127.0.0.1:4173` |
| 预览首页 HTTP 冒烟 | 通过 | `StatusCode=200; Length=885` |

说明：首次 HTTP 冒烟命令因 PowerShell 引号转义失败，未触达页面；随后使用当前 PowerShell 会话直接重跑，请求返回 200。

## 5. 完成定义

- [x] `npm run build` 0 error。
- [x] `npm run preview` 可启动并监听 `127.0.0.1:4173`。
- [x] 预览首页 HTTP 200。
- [x] 将本轮结果更新到进度存档。

## 6. 风险与处理

- 若端口 `4173` 已占用，改用 Vite 自动分配端口，并以实际输出端口做 HTTP 冒烟。
- 若构建失败，优先修复 `GroupLogsPanel.vue` / `GroupSitesPanel.vue` 相关类型和模板错误；不做无关重构。
- 若预览页 404，检查 Vite base 配置与部署路径，必要时补充静态部署说明。
