# Implementation Plan

## Approach

接口日志是唯一需要新建采集的源,其余两类(校审流转历史、站点文件日志)均为既有数据源,
只需在统一查询契约后面各写一个 adapter。先做后端纵切(中间件 → 存储 → 查询 API),
HTTP 冒烟通过后再做前端抽屉。全程 feature-flag/路由挂载点控制,默认行为零变化。

## Files

### 后端(plant-model-gen)

- `rs_surreal/review/migrations/2026MMDD_002_api_request_log.surql`(新)
  - `DEFINE TABLE api_request_log SCHEMALESS` + `form_id`/`task_id`/`created_at`/`status` 索引。
- `src/web_api/api_request_log.rs`(新)
  - axum 中间件 `api_request_log_layer`:克隆截断 req body → 执行 handler → 截断 resp body
    → 脱敏(丢 Authorization/Cookie;JSON 字段 token/password/secret → `***`)
    → 提取 form_id/task_id → `tokio::spawn` 异步写 Surreal(复用 `review_db_session()`)。
  - 后台保留期清理任务(7 天,启动时 spawn,周期 1h)。
- `src/web_api/logs_api.rs`(新)
  - `GET /api/logs/types`:按 `jwt_auth::Role` 裁剪类型树。
  - `GET /api/logs`:`LogQuery` → 按 `type` 分派 adapter:
    - `api.request` → 查 `api_request_log` 表;
    - `review.workflow` → 查 `review_workflow_history` 表;
    - `site.file.*` → 复用 `web_server::managed_project_sites::tail_log`(内存过滤 level/q)。
  - 统一 `LogEntry` DTO 与 cursor 分页。
- `src/web_api/review_api.rs`(改,~1552 中间件链追加 `api_request_log_layer`)
- `src/web_api/platform_api/mod.rs`(改,~70 中间件链追加同一层)
- `src/web_api/mod.rs` / `src/web_server/mod.rs`(改,挂载 logs 路由 + JWT 中间件 + 启动清理任务)

### 前端(../plant3d-web)

- `src/api/logsApi.ts`(新):`fetchLogTypes()` / `fetchLogs(query)`,复用 reviewApi 的 base-url 与鉴权头逻辑。
- `src/review/flags.ts`(改):新增 `logDrawer` flag。
- 校审页外壳新增 `LogDrawer` 组件(tab=类型;列表+详情展开;接口日志 request/response 分段展示;
  手动刷新 + 可选 5s 轮询;默认 form_id/task_id 过滤)。具体挂点(ribbon 按钮或调试菜单)实施时按现有外壳结构定。

## Validation

- 后端:`cargo check`;启动 web_server 后 HTTP 冒烟(不跑 cargo test):
  1. 带 JWT 调一个 review API → `GET /api/logs?type=api.request` 查到记录且已脱敏/截断;
  2. `GET /api/logs/types` 按角色差异返回;
  3. `type=review.workflow` 与 `type=site.file.parse` 各查一次与既有数据比对。
- 前端:`pnpm vitest run src/api/logsApi.test.ts`;flag 开关手测抽屉出现/消失。

## Risks

- body 缓冲:axum 读取 body 后需重新注入,注意 `Request`/`Response` 重组的内存峰值(4KB 截断上限控制)。
- 站点文件日志 adapter 的 level/q 过滤是文本启发式,准确度有限(可接受,排障场景)。
- `review_workflow_history` 字段口径以 RUS-244 迁移后 schema 为准,adapter 做 option 容错。
