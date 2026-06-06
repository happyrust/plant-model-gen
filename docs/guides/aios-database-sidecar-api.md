# aios-database sidecar API

`web_server` 是管理后台 BFF 和控制面；`aios-database` sidecar 拥有 E3D/DB 解析域。

关键边界：

- `web_server` 负责鉴权、站点 CRUD、配置文件写入、sidecar 生命周期、UI 兼容响应。
- `aios-database` 负责项目扫描、DB 文件头解析、dbnum/db type 推导、依赖索引、解析预览、parse/generate job。
- `web_server` 不应读取 E3D project root，不应打开 DB 文件头，不应计算依赖闭包。

## 启动 sidecar

```powershell
target/debug/aios-database.exe serve `
  --site-key <site-or-smoke-key> `
  --bind-host 127.0.0.1 `
  --http-port <port> `
  --runtime-dir runtime/smoke/<key> `
  --token <bearer-token>
```

所有 API 使用本机 loopback。若设置了 token，请带：

```http
Authorization: Bearer <bearer-token>
```

## HTTP API

### 健康检查

```http
GET /health
```

用于启动探测。返回 2xx 表示 sidecar 可用。

### 扫描工程

```http
POST /projects/scan
```

输入由 `web_server` 代理自管理端工程扫描请求。sidecar 负责扫描项目目录和读取 DB 事实。

### 预览解析计划

```http
POST /parse/preview-plan
```

返回 parse plan，包括 included DB files、dbnum、db type、来源和 warnings。参数错误返回结构化错误，例如：

```json
{
  "success": false,
  "message": "项目名不能为空",
  "data": null,
  "error": {
    "code": "INVALID_PROJECT_NAME",
    "field": "project_name",
    "retryable": false
  }
}
```

### 重建 db_index

```http
POST /db-index/rebuild
```

示例 body：

```json
{
  "roots": [{ "name": "smoke-root", "path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001" }],
  "index_path": "runtime/smoke/db-index.sqlite",
  "force": true,
  "manual_db_nums": [250160]
}
```

该接口会通过 `/events` 广播 `db_index_rebuild_started`、`db_index_rebuild_progress`、`db_index_rebuild_done` 或 `db_index_rebuild_failed`。

### 提交 CLI job

```http
POST /jobs/submit-cli
```

示例 body：

```json
{
  "config_no_ext": "runtime/admin_sites/<site>/DbOption-parse",
  "cwd": "D:/work/plant-code/plant-model-gen",
  "stdout_path": "runtime/admin_sites/<site>/parse.log",
  "stderr_path": "runtime/admin_sites/<site>/parse.err.log"
}
```

返回：

```json
{
  "success": true,
  "data": { "job_id": "<uuid>" }
}
```

### 查询 CLI job

```http
GET /jobs/{job_id}
```

典型状态：`queued`、`running`、`cancelling`、`succeeded`、`failed`、`cancelled`。

### 取消 CLI job

```http
POST /jobs/{job_id}/cancel
```

取消请求成功后会广播 `job_cancel_requested`，终止后广播 `job_cancelled`。

## WebSocket events

```http
GET /events
```

连接建立后先收到：

```json
{ "type": "sidecar_hello", "site_key": "<site-key>" }
```

CLI job 事件：

- `job_submitted`
- `job_running`
- `job_started`
- `stage_changed`
- `log_appended`
- `artifact_ready`
- `job_done`
- `job_failed`
- `job_cancel_requested`
- `job_cancelled`

`web_server` 会在提交 parse/generate sidecar job 后订阅 `/events`，按 `job_id` 过滤事件，写入站点日志并触发 runtime snapshot。

## Smoke commands

默认 failed job + events：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1
```

取消路径：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1 -CancelJob
```

db_index 单文件路径：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1 `
  -DbIndexRootPath "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001" `
  -ManualDbNums 250160
```

成功 job 路径需要提供稳定 config：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1 `
  -ConfigNoExt "D:/work/plant-code/plant-model-gen/runtime/smoke/success-config/DbOption-success" `
  -ExpectJobSuccess `
  -JobTimeoutSec 120
```

## Boundary guard

运行本地 guard，防止 `web_server` 重新进入解析域：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/guard/web_server_parse_boundary_guard.ps1
```

guard 会扫描 `src/web_server/*.rs`，禁止重新出现本地 DB 文件头解析、dbnum 推导和依赖闭包相关符号。
