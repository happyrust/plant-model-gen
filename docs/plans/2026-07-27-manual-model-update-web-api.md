# 手动模型更新 Web API 开发计划

## 边界

- `plant-model-gen` 是唯一服务实现，复用现有 `IncrementRun`、Version Commit 和 `INCREMENT_RUNS`。
- 增量执行键是 `dbnum + sesno`；ZONE 只做预览分桶。
- BRAN、HANG、SUPPO、EQUI 是最小模型生成单元；FTUB 只是管件。
- 只处理 DESI，CATA 变化暂不进入本期流程。
- 不使用 SurrealDB live query；任务进度通过普通 HTTP 查询，后续再接现有 WebSocket 广播。

## 接口与进度

| 阶段 | 接口 | 状态 | 验收 |
| --- | --- | --- | --- |
| M1 | `GET /api/v1/health` | 已完成 | HTTP 200 |
| M1 | `GET /api/v1/dbnums` | 已完成 | 返回 Version Commit/dbnum 状态 |
| M1 | `POST /api/v1/update/preview` | 已完成 | 只读；按 SESNO 统计并按 ZONE 分桶 |
| M2 | `POST /api/v1/update/execute` | 已完成 | 202；复用 IncrementRun；排除非 DESI |
| M2 | `GET /api/v1/tasks` | 已完成 | 返回进程内运行记录，可按 state/kind 过滤 |
| M2 | `GET /api/v1/tasks/{id}` | 已完成 | 返回单次运行状态 |
| M3 | `GET /api/v1/update/pending-units` | 已完成 | 复用 model_gen_debt，列出待补生成单元 |
| M3 | `POST /api/v1/model/ensure` | 待确认生成 seam | 只在找到现有单 refno 入口后接入 |
| M4 | `GET /api/v1/ws` | 已完成 | 复用 ProgressHub，发布手动更新运行状态 |

## 验证顺序

1. 用小型 DESI dbnum 验证 preview JSON：`execution_scope=dbnum+sesno`、`zone_role=reporting_bucket`。
2. 用无有效 dbnum 请求验证 execute 边界，不写数据；真实写入只在确认目标 SESNO 后执行。
3. 通过 tasks 列表和详情验证异步状态。
4. M3/M4 完成后运行独立端口服务，以 HTTP/POST 和 WebSocket 验证；不新增或运行 `cargo test`。
