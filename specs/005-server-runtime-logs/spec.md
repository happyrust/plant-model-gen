# Feature Specification: 进程内运行日志(server.runtime)

> spec 003/004 的二期第一项(004 访谈 Q2 既定方向:tracing/log → 内存 ring → adapter)。
> 诉求:校审页触发的实时模型生成等操作,其日志只存在于 web_server 进程内(log/tracing 宏),
> 不落站点文件日志;抽屉里应能直接看到。

## Approach(已在 004 访谈定向)

- web_server 进程把全局 logger 替换为 Tee 包装:原样转发给 env_logger,同时把
  `info` 及以上级别的记录写入进程内有界环形缓冲(默认 5000 条,旧记录淘汰)。
- `tracing::*` 宏在未安装 tracing subscriber 时经 log 桥接同样流经全局 logger,天然被覆盖。
- `/api/logs` 新增类型 `server.runtime`(admin-only),从 ring 查询;无持久化(重启即清,
  定位是"看刚才发生了什么")。

## Requirements

1. Tee logger 不改变既有控制台输出格式与过滤行为;ring 写入开销为 O(1) 且加锁粒度最小。
2. `server.runtime` 条目含 ts_ms/level/target/message;message 截断 2KB。
3. `/api/logs?type=server.runtime` 支持 level(error/warn)/q(target 或 message 子串)/
   from_ms/to_ms/cursor(offset)/limit;newest-first。
4. `/api/logs/types` 对 admin 增列 `server.runtime`;非 admin 不可见(403 规则同 api.request)。
5. 仅 web_server 二进制安装 Tee;aios-database CLI 行为不变。
6. 前端零改动(抽屉 tab 由类型目录动态渲染)。

## Non-Goals

- 不做持久化/跨重启;不做 SSE 实时推送;不采集 debug/trace 级别。

## Acceptance Criteria

- 启动 web_server 后,`GET /api/logs?type=server.runtime`(admin)能查到启动期 info 日志
  (如 review schema 预热);`level=error` 过滤只返回 error 条目。
- 触发一次 review API 后,ring 中能看到对应域的新日志条目。
- 非 admin 角色查询返回 403;types 目录按角色裁剪。
- 浏览器抽屉自动出现"进程运行日志"tab 且可加载数据(无前端代码改动)。

## Verification Result(2026-06-11)

| 验收点 | 结果 |
|---|---|
| 启动期日志可查 | ✅ `type=server.runtime` 返回 info 条目(含 target,如 jwt_auth token 生成) |
| error 过滤 | ✅ `level=error` 仅返回 error(当前 0 条,不变式成立) |
| 越权 | ✅ 非 admin → 403;types 目录按角色裁剪 |
| 触发后新条目 | ✅ 调 review API 后 `q=review` 命中 3 条新日志 |
| 前端零改动 | ✅ Playwright 实测抽屉自动出现"进程运行日志"tab(类型目录驱动) |
| 控制台行为 | ✅ Tee 转发 env_logger,输出格式/过滤与升级前一致 |
