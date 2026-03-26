# 站点配置中心（web_server 单站点单项目）开发文档

> 更新日期：2026-03-25

## 1. 功能目标

本功能实现“**一个 web_server 进程对应一个站点 + 一个项目**”的运行时语义，并提供站点注册与站点配置管理能力。

- 每个 web_server 实例在启动时把自己注册到 SQLite 中心注册表；
- `/api/sites` 与 `/api/deployment-sites` 提供同一份清单事实源；
- 前端通过 `/deployment-sites` 页面完成站点的列表/详情/新建/编辑/导入/删除/健康检查；
- 每个站点记录支持“区域、项目、项目代号、前端地址、后端地址、监听地址”等核心字段。

## 2. 核心设计原则

- **单站点单项目语义**：一个进程只维护一个站点记录；多项目语义通过多个进程并行承载。
- **中心注册表单一事实源**：站点事实以 `deployment_sites.sqlite` 为准。
- **状态可观测**：状态含 `Offline`，由 `last_seen_at + registry_ttl_secs` 推导。

## 3. 实现范围（完成项）

### 3.1 后端注册/运行态

- 新增运行态配置结构：
  - `src/web_server/site_registry.rs` 中 `WebServerRuntimeConfig`
  - `src/web_server/web_listen.rs` 中 `site_identity_json()` 增补站点身份输出
  - `src/web_server/mod.rs` 启动流程里完成 `init_site_identity` 与 `upsert_runtime_site`
- 启动阶段执行站点自注册，并周期心跳更新 `last_seen_at`
- 异常退出时尝试标记为 `Stopped`（离线由 TTL 判定）

### 3.2 中心注册表（SQLite）

- SQLite 表：`deployment_sites`
  - 新增/补齐字段：
    - `site_id`、`name`、`region`、`project_name`、`project_path`、`project_code`、`frontend_url`、`backend_url`、`bind_host`、`bind_port`、`status`、`health_url`、`owner`、`notes`、`config_json`、`last_seen_at`、`created_at`、`updated_at`
  - 兼容字段保留：`env`、`selected_projects`、`e3d_projects_json` 等
- 唯一约束：
  - `site_id` 唯一
  - `backend_url` 唯一（过滤空值）
  - `bind_host + bind_port` 唯一（过滤空值）
- 迁移策略：旧表补列，不重建主表。

### 3.3 模型与请求体

文件：`src/web_server/models.rs`

- `DeploymentSite` 收敛核心字段：
  - `site_id`、`region`、`project_name`、`project_path`、`project_code`、`frontend_url`、`backend_url`、`bind_host`、`bind_port`
- 请求体支持：
  - `DeploymentSiteCreateRequest`
  - `DeploymentSiteUpdateRequest`
  - `DeploymentSiteImportRequest`
  - `DeploymentSiteQuery`
- `project_code` 已接入；`project_path` 与 `e3d_projects` 兼容但按单项目语义使用。

### 3.4 站点 API

文件：`src/web_server/handlers.rs`、`src/web_server/mod.rs`

- 统一路由：
  - `GET /api/sites`
  - `GET /api/deployment-sites`
  - `POST /api/deployment-sites`
  - `GET /api/deployment-sites/{id}`
  - `PUT /api/deployment-sites/{id}`
  - `DELETE /api/deployment-sites/{id}`
  - `POST /api/deployment-sites/{id}/healthcheck`
  - `POST /api/deployment-sites/import-dboption`
  - `GET /api/site/identity`
- `GET /api/sites` 与 `GET /api/deployment-sites` 清单来源同一事实源。

### 3.5 页面与前端逻辑

文件：
- `src/web_server/handlers.rs` 的 `deployment_sites_page()`（页面模板）
- `src/web_server/static/deployment-sites.js`（Alpine 状态与交互）

页面现状包含：
- 列表、筛选、分页
- 详情抽屉（基础/地址/附加/运行配置）
- 新建/编辑弹窗
- 从 DbOption 导入
- 复制地址、手动健康检查、删除

## 4. API 契约（交付给 UI 的字段口径）

### 4.1 列表返回

`GET /api/deployment-sites`（`GET /api/sites` 同步）返回：

```json
{
  "items": [DeploymentSite],
  "total": 1,
  "page": 1,
  "per_page": 10,
  "pages": 1
}
```

### 4.2 部署站点对象（关键字段）

- `site_id`（站点 ID，主识别）
- `name`（站点名称）
- `region`（区域）
- `project_name`（项目名）
- `project_code`（项目代号）
- `project_path`（项目路径，可为空）
- `frontend_url`
- `backend_url`
- `bind_host`
- `bind_port`
- `status`
- `last_seen_at`
- `env`, `owner`, `health_url`, `notes`
- `url`（兼容字段，通常等于 backend_url）
- `config`（DatabaseConfig）

### 4.3 查询参数

- `page`, `per_page`
- `q`（模糊匹配：站点名/项目名/地址/站点ID/项目代号）
- `status`, `region`, `project_name`, `env`, `owner`
- `sort`
- `registry_ttl_secs`（状态离线阈值）

### 4.4 运行状态

`status` 取值：`Configuring`、`Deploying`、`Running`、`Failed`、`Stopped`、`Offline`

> `Offline` 由 `last_seen_at` 与 TTL 计算，不一定是持久状态。

### 4.5 删除约束

- 删除当前进程对应站点返回 `409`，前端需明确提示

## 5. 配置文件与实例示例

支持文件：`db_options/DbOption-mac.toml`、`db_options/DbOption-zsy.toml`

示例关注段：

```toml
[web_server]
bind_host = "127.0.0.1"
port = 3100
site_id = "avevamarinesample-3100"
site_name = "AvevaMarineSample"
region = "sjz"
frontend_url = "http://123.57.182.243"
public_base_url = "http://127.0.0.1:3100"
deployment_sites_sqlite_path = "deployment_sites.sqlite"
registry_ttl_secs = 120
heartbeat_interval_secs = 30
```

## 6. 请求示例

### 6.1 创建站点（示例）

```json
POST /api/deployment-sites
{
  "site_id": "site-abc-3100",
  "name": "A 站点",
  "region": "sjz",
  "project_name": "AvevaMarineSample",
  "project_code": 1516,
  "frontend_url": "http://127.0.0.1:5173",
  "backend_url": "http://127.0.0.1:3100",
  "bind_host": "0.0.0.0",
  "bind_port": 3100,
  "config": {
    "project_name": "AvevaMarineSample",
    "project_code": 1516
  }
}
```

### 6.2 更新站点（示例）

```json
PUT /api/deployment-sites/site-abc-3100
{
  "site_id": "site-abc-3100",
  "region": "sjz",
  "project_name": "AvevaMarineSample",
  "project_code": 1516,
  "frontend_url": "http://127.0.0.1:5173",
  "backend_url": "http://127.0.0.1:3100",
  "bind_host": "127.0.0.1",
  "bind_port": 3100,
  "config": {
    "project_name": "AvevaMarineSample",
    "project_code": 1516
  }
}
```

### 6.3 导入 DbOption（示例）

```json
POST /api/deployment-sites/import-dboption
{
  "path": "db_options/DbOption-zsy.toml",
  "frontend_url": "http://127.0.0.1:5174",
  "backend_url": "http://127.0.0.1:3101"
}
```

## 7. UI 对接要点（给前端会话）

1. 清单优先读 `/api/deployment-sites`（或 `/api/sites`）
2. 列表项主键与详情查询统一使用 `site_id`
3. 表单提交尽量带齐：`site_id/name/region/project_name/project_code/frontend_url/backend_url/bind_host/bind_port/config`
4. 读取/展示时优先 `frontend_url/backend_url/bind_host/bind_port`
5. 错误处理：
   - 400：字段校验与唯一性冲突
   - 409：当前运行实例不能删
6. `project_code`、`project_path` 为单项目语义重要字段，需展示 + 可编辑

## 8. 注意事项

- 当前实现不要求“运行时注册表自动创建多进程级项目聚合”；其设计前提就是“多实例 = 多进程”
- 兼容性：`env`、`e3d_projects` 仍保留用于回填展示
- 建议在 UI 中使用 `last_seen_at` + `status` 双保险展示，避免误判

