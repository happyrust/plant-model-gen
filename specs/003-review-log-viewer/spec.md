# Feature Specification: 三维校审前端日志查看(Review Log Viewer)

> 来源:2026-06-10 手写需求笔记 —— "给三维校审流程添加上前端可以查看日志的功能;
> ① 查看日志可以查看很多种类型,校审只是其中一种,还有其它类型的日志,可以整理分类;
> ② 日志要区分 request/response 等,尤其是接口日志。"

## User Need

实施/运维人员在校审联调与现场排障时,需要在校审前端(plant3d-web,嵌入 PMS 的页面)直接查看
与当前单据/任务相关的日志,而不是登服务器翻文件。日志必须分类型组织(接口日志、校审流转
历史、站点运行日志等),其中接口日志必须能区分 request 与 response 内容。

## Scope

- 后端(plant-model-gen):
  - 新增接口 request/response 日志采集(axum 中间件,仅 review/platform 域路由)。
  - 新增统一日志查询契约:类型目录 + 聚合分页查询,适配三种既有/新增数据源。
  - SurrealDB 新表 `api_request_log` 及保留期清理。
- 前端(../plant3d-web):
  - 校审页调试抽屉(Log Drawer):按类型 tab 切换,默认过滤当前 form_id/task_id。

## Decisions(grill-me 访谈定案,2026-06-10)

| # | 决策点 | 结论 |
|---|---|---|
| 1 | 目标用户/场景 | 实施/运维技术排查为主(PMS 联调),按角色分级可见;入口为校审页调试抽屉 |
| 2 | 一期日志类型 | `api.request`(新建采集)+ `review.workflow`(已有 `review_workflow_history` 表)+ `site.file.{parse,generate,db,web,viewer}`(已有文件日志 API);MQTT/remote_sync 留二期 |
| 3 | 接口日志采集点 | 仅后端 axum 中间件(单一事实源,覆盖浏览器与 PMS S2S 调用);前端不重复采集 |
| 4 | 存储与保留 | SurrealDB 新表 `api_request_log`;body 截断 4KB;保留 7 天(后台定时清理);异步写入不阻塞请求 |
| 5 | 查询契约 | 统一聚合 API:`GET /api/logs/types` + `GET /api/logs?type=&form_id=&task_id=&site_id=&level=&q=&from=&to=&cursor=`;统一 `LogEntry` 响应;后端 adapter 适配三源 |
| 6 | 前端形态 | plant3d-web 校审页调试抽屉,手动刷新 + 可选 5s 轮询;`src/review/flags.ts` feature flag 门控;admin UI 本期不动;SSE 实时流留二期 |
| 7 | 鉴权 | 复用 review JWT 中间件;按 Role 分级:`api.request`/`site.file.*` 仅管理/实施类角色,`review.workflow` 全部已认证角色;类型目录接口按角色裁剪返回 |
| 8 | 采集范围/脱敏 | 仅 review/platform 域路由全量采集;丢弃 Authorization/Cookie 头;JSON body 中 token/password/secret 字段替换为 `***`;排除模型/网格等大流量 gen_model 接口 |

## Requirements

1. review/platform 域每个 HTTP 请求产生一条 `api_request_log` 记录,包含:
   `request_id`、`method`、`path`、`status`、`elapsed_ms`、`req_body`(截断+脱敏)、
   `resp_body`(截断+脱敏)、`form_id`/`task_id`(尽力从 path/body 提取)、`created_at`。
2. 日志写入异步执行;写入失败仅告警,绝不影响业务请求的响应。
3. `GET /api/logs/types` 返回当前角色可见的类型树(含每类的展示名与过滤维度声明)。
4. `GET /api/logs` 支持 `type` 必选,`form_id/task_id/site_id/level/q/from/to/cursor` 可选,
   返回统一 `LogEntry{ts, type, level, summary, detail, correlation{form_id, task_id, site_id, request_id}}`
   分页结构;三种数据源(Surreal 表 / review_workflow_history / 站点日志文件 tail)各自实现 adapter。
5. 保留期清理:`api_request_log` 超过 7 天的记录由后台任务周期删除。
6. 鉴权:两个新端点挂 review JWT 中间件;角色不满足时类型不可见、查询返回 403。
7. 前端抽屉:flag 开启时在校审页可打开;tab=类型;默认带当前 form_id/task_id 过滤;
   接口日志条目可展开查看 request/response 两段内容。
8. 默认行为零变化:flag 关闭时前端无入口;未挂中间件的路由(含 gen_model 大流量接口)不产生日志。

## Non-Goals

- 不做 MQTT / remote_sync 日志源接入(二期)。
- 不做 SSE/WebSocket 实时日志流(二期,轮询足够)。
- 不改 admin UI(ui/admin)既有站点日志面板。
- 不做前端网络层失败采集(浏览器 DevTools 已覆盖)。
- 不新增或运行 Rust test target;web_server 验证一律走 HTTP。
- 不记录 Authorization/Cookie 等敏感头,不全文存储超大 body。

## Acceptance Criteria

- 调用任一 review API(如创建批注)后,`GET /api/logs?type=api.request&form_id=<id>` 能查到
  对应记录,且 request/response 两段内容分离可读、敏感字段已脱敏。
- `GET /api/logs/types` 在管理类角色下返回 3 组类型;在业务角色下仅返回 `review.workflow`。
- `GET /api/logs?type=review.workflow&task_id=<id>` 返回与 `review_workflow_history` 表一致的流转记录。
- `GET /api/logs?type=site.file.parse&site_id=<id>` 返回与既有站点日志 tail 一致的内容。
- 写入 1 条超过 4KB body 的请求,存储记录 body 被截断且标记 `truncated=true`。
- 校审页 flag 开启后抽屉可用并默认过滤当前单据;flag 关闭无任何入口。

## Verification Result(2026-06-10 HTTP 冒烟,隔离环境)

环境:surreal 3.2.0-nightly(f01470af,memory 引擎,127.0.0.1:8042)+
web_server:3199(`db_options/DbOption-smoke003`,`[surrealdb] mode="ws"` 为 review 域硬性要求)。

| 验收点 | 结果 |
|---|---|
| api.request 记录(request/response 分离) | ✅ `GET /api/review/tasks → 200 (652ms)`,req/resp body 字段齐全 |
| form_id 关联过滤 | ✅ `form_id=FORM-SMOKE-G` 精确命中 1 条 |
| 脱敏 | ✅ `password/api_token → ***`(422 失败请求同样被记录) |
| 4KB 截断 | ✅ 6KB body → `req_truncated=true`,存储长度 4096 |
| 角色裁剪 | ✅ debug(sj)仅 review.workflow;admin 7 类 |
| 越权 403 | ✅ 非 admin 查 api.request → 403 |
| review.workflow adapter | ✅ success(空库零记录) |
| 迁移链 | ✅ 启动日志确认 `20260610_002 api_request_log` 已应用 |
| 中间件 fail-open | ✅ review db 不可用时业务响应不受影响,仅 warn |
| site.file.* | ✅ 真实站点(avevamarinesample-7997-e2e)回归:parse tail 5 条带 site_id 关联;generate + level=error 启发式过滤精确命中 1 条 |
| 前端 | logsApi vitest 3/3 绿;LogDrawer/flag 待开 flag 后人工联调 |

升级注记:surrealdb 客户端 3.2.0-nightly 与 3.1.0-alpha server **协议不兼容**
(`Failed to decode value from fb value`),部署时客户端/服务端必须同步升级;
Windows debug 版 server 需 `/STACK:16777216` 重链接。
