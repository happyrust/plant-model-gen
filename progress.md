# 站点部署功能修复进度

## 2026-05-06

- 已完成 review-only 审查，确认 P1/P2 风险点。
- 已建立 `task_plan.md`、`findings.md`、`progress.md` 作为本轮修复记录。
- 已新增 `admin_task_handlers::create_and_dispatch_site_task`，注册表创建任务改为单一 admin task id，并立即 dispatch。
- 已将旧 `static/deployment-sites.js` 的站点管理写接口改到 `/api/admin/registry/*`，并补充 `localStorage.admin_token` Authorization header。
- 已在 `db_options/DbOption.toml` 增加 `admin_allowed_project_roots = ["D:/AVEVA/Projects"]`，匹配新的项目路径白名单默认值。
- `ReadLints` 检查本轮 Rust/JS 修改文件：无 linter errors。
- `cargo check --bin web_server --features web_server` 通过；输出仅包含 `pdms-io-fork`/`parse_pdms_db` 既有 warnings。
- 已将 `feat/collab-api-consolidation` fast-forward 合入 `origin/main` 最新提交 `bc5715e` 并推送到远端。
- 开始第二轮：复审合并后的站点部署实现，并准备运行 APS 相关 `web_server` 做 HTTP 验证。
- 旧 APS 18330 已按用户要求中止；当前源码版 APS 使用 `target-aps-current` 重新编译并在 18330 启动成功。
- APS 首轮 smoke 暴露公开站点列表 `items=[]`、admin task 响应泄漏 `db_password`、非 ASCII task id 详情查询不稳三个问题。
- 已修复 `handlers::api_get_public_deployment_sites`，避免 JSON 往返反序列化导致公开列表清空。
- 已修复 `TaskInfo::generate_task_id`，任务 ID 限制为 ASCII URL-safe 字符。
- 已修复 `admin_task_handlers` 响应脱敏，task 列表/详情/创建/重试不再返回真实 `db_password/password/surreal_password`。
- 修复后 `ReadLints` 检查本轮 Rust 修改文件：无 linter errors。
- 修复后 APS HTTP smoke 通过：`/api/health` 200；公开 `/api/deployment-sites` 返回 4 条且无 `config/project_path`；admin 登录成功；admin registry 列表/探活密码脱敏；创建 `DataGeneration` 任务返回 ASCII task id，任务详情可查询且密码脱敏。该任务因站点已运行而进入 `Failed`，符合运行态保护。
- 已执行下一步计划：把 APS smoke 口径收敛为公开接口字段、admin 脱敏、任务创建/详情三类回归点。
- 敏感信息回归脚本第一次执行因 PowerShell 字符串转义错误失败，未触发有效断言；已改用 `[char]34` 构造字段名后重跑成功。
- 最终敏感信息回归结果：公开站点响应不包含真实密码、不包含 `config`、不包含 `project_path`；admin registry 响应不包含真实密码且包含 `********`；admin tasks 响应不包含真实密码且包含 `********`。
- 当前 APS 新版仍在 `127.0.0.1:18330` 运行；`target-aps-current/` 为本轮生成的独立编译目录，仍未清理。
- 最终工程收敛：`CARGO_TARGET_DIR=target-aps-current cargo check --bin web_server --features web_server` 通过，用时约 8m01s；仅有 `pdms-io-fork` / `parse_pdms_db` 既有 warning。
- 已复查本轮核心 diff：`admin_task_handlers.rs`、`models.rs`、`handlers.rs` 与计划记录文件包含 APS 修复；其中 `handlers.rs` 同时包含合并前既有未提交改动，后续提交时需按变更来源拆分/确认。
