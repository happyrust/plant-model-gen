# 站点部署功能修复计划

## 目标

修复站点部署审查中发现的后台功能错位问题，让管理端注册表、受管站点、任务创建与健康检查入口保持一致。

## 阶段

1. [complete] 收敛注册表任务创建链路，避免双任务状态源和不执行的问题。
2. [complete] 清理或兼容旧 `/api/deployment-sites/*` 写接口迁移后的入口断裂。
3. [complete] 同步 `admin_allowed_project_roots` 安全默认值的配置/说明，避免后台创建站点被意外拒绝。
4. [complete] 做最小验证：编译检查或按项目规则用运行后的 POST/GET 验证关键接口。
5. [complete] 复审合并主分支后的站点部署实现，确认 admin/public/managed site API 边界仍一致。
6. [complete] 启动 APS 相关 `web_server` 配置，使用 HTTP 登录、站点清单、站点详情、健康检查和任务创建接口做运行验证。
7. [complete] 汇总 APS 运行验证结论，记录剩余风险和后续处理项。
8. [complete] 将 APS 站点部署 smoke 固化为回归口径：公开接口字段、admin 脱敏、任务创建与详情查询。
9. [complete] 做最终无敏感信息泄漏检查，覆盖公开站点、admin registry、admin tasks 三类响应。
10. [complete] 收敛本轮改动状态，标记仍需人工决定的内容（是否提交、是否清理临时 target）。

## 风险

- 当前工作树已有大量既有改动，修复时只触碰站点部署相关文件。
- 项目规则要求 web_server 不使用 test；验证优先使用运行服务和 HTTP POST/GET。
- `plant3d-web` 当前另有未完成 merge conflict，本轮只处理 `plant-model-gen`，不跨仓改动冲突。
