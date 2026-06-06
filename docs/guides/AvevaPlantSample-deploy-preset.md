# AvevaPlantSample 站点部署预设模板（参考）

> 日期：2026-05-31
> 用途：以 AvevaPlantSample 为测试项目，给出一份**可直接参考、可正常部署**的站点预设配置模板。
> 事实来源：本机已验证可运行的部署 `runtime/.../admin_sites/avevaplantsample-18650/`
> （`metadata.json` + `DbOption.toml`），磁盘数据 `D:/AVEVA/Projects/E3D2.1/{AvevaPlantSample,AvevaCatalogue}` 实测在位。

---

## 0. 项目结构与前置条件

```
D:/AVEVA/Projects            ← 白名单根（admin_allowed_project_roots）
└── E3D2.1
    ├── AvevaPlantSample     ← 设计工程（DESI，主工程 primary）
    └── AvevaCatalogue       ← 元件库工程（CATA，role=library）
```

**前置 1 · 配置白名单**（否则创建站点会被 `canonical_project_path` 拒绝）：
在所用 `db_options/DbOption-*.toml` 顶层加：

```toml
admin_allowed_project_roots = ["D:/AVEVA/Projects"]
```

> 本地临时验证也可设环境变量 `AIOS_ADMIN_ALLOW_ANY_PROJECT_PATH=1`（生产勿用）。

**前置 2 · 强数据库凭据**：后端拒绝 `root/root` 等弱凭据。模板用 `aveva_site_admin` + 自定义强密码。

**前置 3 · admin 登录**：站点接口在 `admin_auth_middleware` 之下，需先 `POST /api/admin/auth/login` 拿 Bearer token。

---

## 1. 预设模板 A · 多工程（推荐，design + library）

> 利用多工程能力：一个站点合并 AvevaPlantSample（设计）+ AvevaCatalogue（元件库）。
> 直接 `POST /api/admin/sites`（body 为 `CreateManagedSiteRequest`）。

```json
{
  "site_name": "AvevaPlantSample",
  "projects": [
    {
      "path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
      "name": "AvevaPlantSample",
      "role": "design",
      "is_primary": true,
      "sort_order": 0
    },
    {
      "path": "D:/AVEVA/Projects/E3D2.1/AvevaCatalogue",
      "name": "AvevaCatalogue",
      "role": "library",
      "is_primary": false,
      "sort_order": 1
    }
  ],
  "project_name": "AvevaPlantSample",
  "project_path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
  "project_code": 7011,
  "manual_db_nums": [],
  "parse_db_types": ["SYST", "DESI", "CATA", "DICT", "GLB", "GLOB"],
  "force_rebuild_system_db": false,
  "gen_model": true,
  "gen_mesh": true,
  "gen_spatial_tree": true,
  "apply_boolean_operation": true,
  "mesh_tol_ratio": 3.0,
  "export_json": false,
  "export_parquet": true,
  "pipeline_db_mode": "file",
  "runtime_db_mode": "ws",
  "db_port": 18651,
  "web_port": 18650,
  "bind_host": "127.0.0.1",
  "db_user": "aveva_site_admin",
  "db_password": "<设置一个强密码，例如 AvevaPlantSample_18651!>",
  "auto_deploy": true
}
```

> 也可用前端「工程组成」区的 **「扫描」** 输入根目录 `D:/AVEVA/Projects/E3D2.1`，
> 自动发现并预填上述两个工程（角色/主工程/dbnum 冲突均自动标注）。

---

## 2. 预设模板 B · 单工程（兼容旧语义）

> 不需要把元件库单列时使用；后端会自动把它当作单 design 站点。

```json
{
  "project_name": "AvevaPlantSample",
  "project_path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
  "project_code": 7011,
  "parse_db_types": ["SYST", "DESI", "CATA", "DICT", "GLB", "GLOB"],
  "gen_model": true,
  "gen_mesh": true,
  "gen_spatial_tree": true,
  "apply_boolean_operation": true,
  "mesh_tol_ratio": 3.0,
  "export_parquet": true,
  "pipeline_db_mode": "file",
  "runtime_db_mode": "ws",
  "bind_host": "127.0.0.1",
  "db_user": "aveva_site_admin",
  "db_password": "<强密码>",
  "auto_deploy": true
}
```

> 省略 `db_port`/`web_port` 时后端从 8020/8080 起自动分配空闲端口。

---

## 3. 字段说明（取自真实可跑配置）

| 字段 | 取值 | 说明 |
|---|---|---|
| `project_code` | `7011` | AvevaPlantSample 的项目代号 |
| `parse_db_types` | 全 6 类 | 对应「全量系统数据」预设，首跑补齐属性/元件/字典 |
| `force_rebuild_system_db` | `false` | 已解析过的 SYST 优先复用 |
| `gen_model/mesh/spatial_tree` | 全 `true` | 生成模型 + 网格 + 空间树（Viewer 可用） |
| `apply_boolean_operation` | `true` | 精度更高、耗时更长 |
| `mesh_tol_ratio` | `3.0` | 默认 L1 精度 |
| `export_parquet` | `true` | 导出 Parquet（`export_json=false`） |
| `pipeline_db_mode` | `file` | 解析/生成走离线文件，不依赖已启动服务 |
| `runtime_db_mode` | `ws` | 正式运行连 SurrealDB 服务 |
| `db_port`/`web_port` | `18651`/`18650` | 可改或留空自动分配 |
| `db_user` | `aveva_site_admin` | 强凭据，勿用 root/root |

---

## 4. 部署步骤

### 4.1 一键（API，推荐）

```bash
BASE=http://127.0.0.1:3100

# 1) 登录拿 token
TOKEN=$(curl -sS -X POST "$BASE/api/admin/auth/login" \
  -H 'Content-Type: application/json' \
  -d '{"username":"<admin>","password":"<admin密码>"}' | jq -r '.data.token')

# 2) 创建站点 + 自动完整部署（auto_deploy=true）
curl -sS -X POST "$BASE/api/admin/sites" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d @avevaplantsample-preset.json | jq .
```

`auto_deploy:true` 会提交「完整部署」任务（解析 → 生成 → 启动）。

### 4.2 分步（可控）

```bash
SID=avevaplantsample-18650   # 创建返回的 site_id
curl -sS -X POST "$BASE/api/admin/sites/$SID/parse"    -H "Authorization: Bearer $TOKEN"   # 解析
curl -sS -X POST "$BASE/api/admin/sites/$SID/generate" -H "Authorization: Bearer $TOKEN"   # 生成
curl -sS -X POST "$BASE/api/admin/sites/$SID/start"    -H "Authorization: Bearer $TOKEN"   # 启动
```

### 4.3 前端 UI

`/admin/#/sites` → 新建站点 → 「工程组成」扫描 `D:/AVEVA/Projects/E3D2.1` 导入两工程 → 「解析范围」选「全量系统数据」预设 → 填强凭据 → 「保存并一键部署」。

---

## 5. 验证

```bash
curl -sS "$BASE/api/admin/sites/$SID/runtime" -H "Authorization: Bearer $TOKEN" | jq .   # 阶段/进程/解析状态
curl -sS "$BASE/api/admin/sites/$SID/logs"    -H "Authorization: Bearer $TOKEN" | jq .   # 三类日志摘要
curl -sS "$BASE/api/admin/sites/$SID/deploy-validation" -H "Authorization: Bearer $TOKEN" | jq .
```

**Pass 标准**：`runtime` 走完 `Running`，`db_running/web_running=true`，`entry_url` 可打开；`deploy-validation` 无 blocking。

---

## 6. Viewer 客户入口

部署完成后，管理端的「打开 Viewer」按钮生成的是独立 `plant3d-web` 入口，不再使用旧的 `backend=...` 包装 URL。

客户 URL 形态：

```text
http://123.57.182.243/?output_project=AvevaPlantSample&show_dbnum=7997
```

### 6.1 本机/Windows 默认行为

未配置 `AIOS_VIEWER_BASE_URL` 或站点 `public_base_url` 时，后端会探测机器本机 IPv4。Windows 本机没有 Nginx 接管 80 端口时，管理端会结合站点的受管 `viewer_port` 生成：

```text
http://<local-ip>:<viewer_port>/?output_project=<project>&show_dbnum=<dbnum>
```

受管 Viewer 默认绑定 `0.0.0.0`，可通过环境变量覆盖：

```powershell
$env:AIOS_VIEWER_BIND_HOST = "0.0.0.0"
```

如果只允许本机访问，可改为 `127.0.0.1`，但此时 `http://<local-ip>:<viewer_port>` 会不可达。

### 6.2 生产/Nginx 入口

生产环境推荐给每个客户入口配置明确的 Viewer Base URL：

```bash
export AIOS_VIEWER_BASE_URL=http://123.57.182.243
```

或在站点配置中使用 `public_base_url`。显式配置后，管理端会按该入口生成 URL，不会自动附加 `viewer_port`。

Nginx 需要同时提供两类能力：

- 静态服务：`plant3d-web` 构建产物挂在 `/`。
- 同源反代：`/api/`、`/files/`、`/ws/` 转发到对应 `web_server`。

示例配置见 `shells/deploy/nginx-plant3d-web.conf.example`。

Linux 受管启动时会尝试自动写入 Nginx 配置并 reload：

```bash
export AIOS_NGINX_BIN=nginx                         # 可选，默认 nginx
export AIOS_NGINX_CONF_DIR=/etc/nginx/conf.d        # 可选，默认 /etc/nginx/conf.d
```

流程为：写入 `plant3d-web-<site_id>.conf` → `nginx -t` → `systemctl reload nginx`；如果未运行则尝试 `systemctl enable --now nginx`，再 fallback 到 `nginx -s reload`。无权限或无 Nginx 时会在 viewer 日志给出可复制的手动命令，并继续使用受管 `vite preview` fallback。

### 6.3 多站点隔离

一个 Viewer Base URL 绑定一个 `web_server` 后端。多个客户/站点并行时，不要共用同一个根入口，应使用不同 vhost、域名或端口，例如：

```text
http://plant-a.example.com/?output_project=PlantA&show_dbnum=7997
http://plant-b.example.com/?output_project=PlantB&show_dbnum=7998
```

---

## 7. 常见问题

| 现象 | 原因 | 处理 |
|---|---|---|
| 创建报「未在允许的根目录白名单内」 | 未配 `admin_allowed_project_roots` | 配白名单含 `D:/AVEVA/Projects` |
| 创建报弱凭据被拒 | `db_user/password` 太弱 | 换强密码（或临时 `AIOS_ALLOW_WEAK_DB_CREDS=1`） |
| 扫描/保存报 dbnum 冲突 | 两工程含相同 dbnum | 多工程合并不允许 dbnum 重复，消解后再保存 |
| 接口 401 | 未带有效 token | 先 `login` 拿 Bearer token |
| 接口 503 + 「admin auth unavailable」 | 未初始化 admin 账户 | 先建管理员账户再登录 |

---

## 8. 备注

- 本模板的所有取值来自本机已验证可运行的 `avevaplantsample-18650` 部署，可直接复用。
- 多工程能力见 `docs/plans/2026-05-31-multi-project-site-plan.md`（含 `/api/admin/projects/scan` 扫描 API）。
- 真实可 POST 的 JSON 见本文件 §1 / §2 代码块，按需另存为 `avevaplantsample-preset.json`。
