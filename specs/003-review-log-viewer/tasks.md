# Tasks

> 按序执行;每个后端 Task 后跑 `cargo check`(不跑 test);web_server 验证走 HTTP。

## 后端(plant-model-gen)

- [x] **T101 api_request_log 表迁移(已完成)**
  - `rs_surreal/review/migrations/20260610_002_api_request_log.surql`
    (SCHEMALESS + form_id/task_id/created_at_ms/status 索引),已注册进 review_db.rs 迁移链;
    实现偏差:`created_at` 改为 `created_at_ms`(epoch ms 数字),规避 SurrealDB 大版本间
    datetime 序列化差异。
- [x] **T102 接口日志中间件(已完成)**
  - `src/web_api/api_request_log.rs`:采集 request_id/method/path/query/status/elapsed_ms/
    req_body/resp_body(4KB 截断 + truncated 标记);不落任何请求/响应头;
    JSON 字段含 token/password/secret(大小写不敏感、含子串)→ `***`;
    form_id/task_id 从 query/path/body 三处尽力提取;`tokio::spawn` 异步写,失败仅 warn。
- [x] **T103 中间件挂载(已完成)**
  - `review_api.rs` 与 `platform_api/mod.rs` 路由链最外层各追加一层(覆盖鉴权/上下文失败请求);
    gen_model 大流量路由不在挂载范围。
- [x] **T104 保留期清理任务(已完成)**
  - 首条日志写入时惰性启动每小时清理循环,删除 `created_at_ms < now-7d` 记录。
- [x] **T105 统一查询 API(已完成)**
  - `src/web_api/logs_api.rs`:`/api/logs/types`(Role 裁剪:admin 7 类,其余仅 review.workflow)
    + `/api/logs`(api.request=ts 游标 / review.workflow=offset 游标 / site.file.*=tail 无游标);
    已同步 `web_api/mod.rs` 装配与 `stateless_web_api_route_paths` 清单。
- [x] **T106 HTTP 冒烟验证(已完成,2026-06-10)**
  - 隔离环境:3.2-nightly surreal(127.0.0.1:8042, memory 引擎)+ web_server:3199
    (db_options/DbOption-smoke003,`[surrealdb] mode 必须为 ws`)。
  - 结果:debug 角色 types=1 类 / admin=7 类;GET /api/review/tasks 后
    `type=api.request` 查到记录(request/response 分离、status/elapsed_ms 齐全);
    `form_id=FORM-SMOKE-G` 过滤命中且 `password/api_token → ***`(422 失败请求也被记录);
    6KB body → `req_truncated=true, len=4096`;非 admin 查 api.request → 403;
    `review.workflow` 返回 success(空数据);中间件 fail-open 已验证
    (review db 不可用时业务响应不受影响)。
  - `site.file.*` 已用真实站点(avevamarinesample-7997-e2e)回归:parse tail 带 site_id
    关联与 level 分类;`level=error` 启发式过滤在 generate.log 上精确命中。

## 前端(../plant3d-web)

- [x] **T201 logsApi 客户端(已完成)**
  - `src/api/logsApi.ts` + `logsApi.test.ts`(pnpm vitest 3/3 绿);
    token 读取做了无 localStorage 环境兜底。
- [x] **T202 LogDrawer 组件(已完成)**
  - `src/components/review/LogDrawer.vue`:悬浮入口 + 类型 tab + 详情展开
    (接口日志含完整 req/resp 字段)+ 手动刷新 + 5s 轮询开关 + "仅当前单据"过滤 + 游标加载更多。
- [x] **T203 flag 门控接入(已完成)**
  - `flags.ts` 新增 `REVIEW_H_LOG_DRAWER`(默认 false);ReviewPanel.vue 挂载,
    传入 activeReviewFormId / currentTask.id;flag 关闭无任何入口。
    vue-tsc:改动文件零错误(仓库存量错误不在本次范围)。

## 收尾

- [x] **T301 文档与移交(已完成)**
  - 验收结果已回填(见 T106);二期候选:MQTT/remote_sync 源、SSE 实时流、admin UI 复用、
    site.file.* 真实站点回归、LogDrawer 组件级测试。
  - 升级注记:surrealdb 客户端升至 3.2.0-nightly 后,**server 必须同步升级**
    (3.1.0-alpha server 会报 `Failed to decode value from fb value`);
    匹配版 server 构建于 D:/work/plant-code/surrealdb-smoke-f01470af(Windows debug 版
    需 `/STACK:16777216` 重链接,否则主线程栈溢出)。
