# Tasks

- [x] **T501 Tee logger + 环形缓冲(已完成)**
  - `src/web_api/server_runtime_log.rs`:RuntimeLogEntry + 有界 ring(cap 5000,message 2KB)
    + TeeLogger + `install_tee_env_logger()`;`bin/web_server.rs` 接入(默认过滤语义不变)。
- [x] **T502 server.runtime adapter(已完成)**
  - `logs_api.rs`:admin 类型目录增"进程运行日志";level/q/from_ms/to_ms/offset cursor,
    newest-first。
- [x] **T503 验证(已完成,2026-06-11)**
  - 启动期 info 可查(jwt_auth 等 target);error 过滤不变式成立;非 admin 403;
    review API 触发后 `q=review` 命中 3 条;Playwright 实测抽屉自动出现新 tab(前端零改动)。
- [x] **T504 回填 spec(已完成)**
