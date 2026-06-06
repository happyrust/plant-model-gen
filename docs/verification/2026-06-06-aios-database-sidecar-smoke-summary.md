# aios-database sidecar smoke summary

日期：2026-06-06

结论：sidecar 迁移主路径已通过静态边界检查、HTTP smoke、WS/job smoke 和前端类型检查。`web_server` 继续作为 BFF/控制面，解析域事实由 `aios-database` sidecar 提供。

## 运行前提

构建最新 sidecar 二进制，避免 smoke 启动旧 sibling binary：

```powershell
cargo build --features web_server --bin aios-database
```

`web_server` 组件不要运行 `cargo test`，本计划使用编译、HTTP/WS smoke 和静态 guard 验收。

## 静态边界 guard

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/guard/web_server_parse_boundary_guard.ps1
```

结果：通过。

覆盖点：

- `src/web_server` 不重新引入 DB 文件头解析。
- `src/web_server` 不重新引入 dbnum 推导。
- `src/web_server` 不重新引入本地依赖闭包计算。

## scan / preview smoke

管理端原路径仍由 `web_server` 对 UI 暴露，底层事实来自 sidecar。

已验证：

- `POST /api/admin/projects/scan` HTTP smoke 通过。
- `POST /api/admin/sites/preview-parse-plan` HTTP smoke 通过。
- 空 `project_name` 的 direct sidecar preview error smoke 返回 `HTTP 400`、`INVALID_PROJECT_NAME`、`field=project_name`，不再落到裸 500。
- `scripts/smoke/sidecar_preview_facts_smoke.ps1` 默认运行通过：`included=6`、`entries=6`、`warnings=0`，验证 preview plan facts 输出结构。

## manifest hash smoke

Admin quick-deploy 创建配置后会立即写入 `parse-plan-manifest.json`。manifest 的 `inputs_hash` 只基于 sidecar 解析输入计算，不包含 `site_id`、`site_name`、`web_port` 等控制面字段。

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/admin_manifest_hash_smoke.ps1 `
  -BaseUrl http://127.0.0.1:<web-server-port>
```

结果：通过。

断言：

- manifest schema 含 `inputs_hash`、`sidecar_version`、`generated_at`、`entries`、`warnings`。
- manifest schema 含 `db_index.role=site_runtime`、正式 `db_index` 路径和匹配的 `db_index.inputs_hash`。
- 匹配 `inputs_hash` 的 preview index 会提升为正式 site runtime index，manifest 标记 `promoted_from_preview=true`。
- 相同解析输入两次创建得到相同 `inputs_hash`。
- 切换 `auto_parse_related_dbnums` 后 `inputs_hash` 改变。

## config write / quick deploy smoke

已验证：

- `POST /api/admin/sites/quick-deploy` 省略 `auto_parse_related_dbnums` 时，创建站点保存为 `false`。
- 临时站点可删除。
- 前端快速部署 checkbox 默认关闭。

相关行为：

- `web_server` 仍负责写 `DbOption.toml` / `DbOption-parse.toml` / `DbOption-generate.toml`。
- 解析事实来自 sidecar preview/job，不由 `web_server` 扫描 E3D project root。

## db_index smoke

真实样本长耗时 smoke：

- 输入：`manual_db_nums=[250160]`
- 结果：`db_files=220`、`ref0_total=422`、`scanned=220`、`errors=0`
- 耗时：约 8 分 28 秒

脚本化单文件 smoke：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1 `
  -DbIndexRootPath "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001" `
  -ManualDbNums 250160
```

结果：通过。

断言事件：

- `sidecar_hello`
- `db_index_rebuild_started`
- `db_index_rebuild_done`
- `job_submitted`
- `job_started`
- `stage_changed`
- `log_appended`
- `job_failed`

Preview 临时 index smoke：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_preview_index_smoke.ps1
```

结果：通过。

断言：

- sidecar preview plan 返回 entries。
- preview index 写入 `runtime/preview-index/<inputs_hash>/db_index.sqlite`。
- preview index path 不在 `runtime/admin_sites` 下，不污染正式 site runtime。
- `db_files > 0` 且 `errors=0`。

## parse/generate job smoke

默认 failed job 路径：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1
```

结果：通过。

断言事件：

- `sidecar_hello`
- `job_submitted`
- `job_started`
- `stage_changed`
- `log_appended`
- `job_failed`

cancel 路径：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1 -CancelJob
```

结果：通过。

断言事件：

- `job_cancel_requested`
- `job_cancelled`

success 路径：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/sidecar_job_events_smoke.ps1 `
  -ConfigNoExt "D:/work/plant-code/plant-model-gen/runtime/smoke/success-config/DbOption-success" `
  -ExpectJobSuccess `
  -JobTimeoutSec 120
```

结果：通过。

断言结果：

- job 终态：`succeeded`
- `exit_code=0`
- `job_done`
- `artifact_ready`

## UI/runtime smoke

已验证：

- `ManagedSiteRuntimeStatus` 暴露 `sidecar_job_kind`、`sidecar_job_id`、`sidecar_job_status`。
- 管理端 `SiteRuntimeCards` 展示 active sidecar job kind/status/id。
- `pnpm exec vue-tsc -b` 通过。

## 编译检查

已验证：

```powershell
cargo fmt --all
cargo check --features web_server --bin web_server
cargo check --features web_server --bin aios-database
pnpm exec vue-tsc -b
```

结果：通过。Rust 依赖库仍有既有 warning，未引入新的编译错误。
