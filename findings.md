# 站点部署功能发现

## 2026-05-06 审查发现

- `admin_registry_handlers::create_site_task` 当前先调用 `handlers::api_create_deployment_site_task`，再写入 `admin_task_handlers::insert_task`，产生两个 task id/状态源；`insert_task` 只保存不 dispatch。
- `/api/deployment-sites` 在 `mod.rs` 已收敛为公开只读，但 `static/deployment-sites.js` 仍 POST `/api/deployment-sites/{id}/healthcheck`。
- `managed_project_sites::canonical_project_path` 现在默认要求 `admin_allowed_project_roots`，否则需显式设置 `AIOS_ADMIN_ALLOW_ANY_PROJECT_PATH=1` 或 `admin_allow_any_project_path=true`。
- 注册表 admin API 已对 `db_password/password/surreal_password` 做响应脱敏；`site_registry::update_site` 已保留占位密码对应的旧真实密码。

## 2026-05-06 APS 运行验证发现

- 旧 APS 进程 `18330` 使用旧构建，admin 健康检查响应仍暴露 `config.db_password`；不能代表当前源码验证结果。
- 当前源码版启动前需要同步本地 `rs-core dev-3.1`，否则 `plant-model-gen` 最新 main 引用的 MBD V2 direct API 编译失败。
- `/api/deployment-sites` 公开列表曾返回 `total=4` 但 `items=[]`，原因是公开列表对已经序列化的 `DeploymentSite` 再反序列化，`SystemTime` 自定义序列化后无法回读；已改为直接在 JSON value 上移除敏感字段。
- admin task 响应曾暴露 `config.db_password`；已对 task 列表、详情、创建、重试响应做递归密钥脱敏。
- admin task id 曾允许非 ASCII 字符进入 URL path，导致包含中文配置名的 task id 难以稳定详情查询；已限制为 ASCII URL-safe 片段。
- APS 当前源码版 `18330` smoke 通过：`/api/health`、`/api/site/identity`、公开站点列表、admin 登录、admin registry 清单/健康检查、任务创建与详情查询均可用；DataGeneration 对已运行站点返回失败状态属于运行态保护。
