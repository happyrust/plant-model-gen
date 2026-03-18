# inst_mesh_meta 方案（SurrealDB 为主存）  
2026-03-15  

## 需求概述
- 在 SurrealDB 建立 `inst_mesh_meta`（或同义 record）作为主存：`refno -> [mesh_ids] + combined_tf + geo_transform + has_neg(+world_aabb)`。
- build spatial 时，从 SurrealDB 批量导出瘦索引到 sqlite（与 aabb 同库）用于粗筛；房间计算先粗筛，再批量回 SurrealDB 取详情，内存缓存。
- 移除旧文件缓存路径依赖，统一走 SurrealDB + 内存批量缓存。

## 验收标准
1. SurrealDB 中 `inst_mesh_meta` 记录数 ≥ 关联 `inst_geo` 的 refno 数；抽样 3 条 mesh_ids/combined_tf 与原数据一致（误差 < 1e-6）。
2. build spatial 导出的 sqlite 表行数与 SurrealDB 同步；导出耗时 ≤ 30s（以 7997 为基准）。
3. 房间计算（dbnum=7997，RUST_LOG=info）总耗时较基线降低 ≥30%，日志可见批量查询（batch ≤400），无全表扫描警告。
4. 房间计算结果与旧逻辑一致：抽样 5 房间，面板 mesh 关联差异为 0。
5. `cargo check --release --bin aios-database` 通过。

## 设计要点
- Schema：`refno`(PK)、`mesh_ids`(array)、`combined_tf`、`geo_transform`(opt)、`has_neg`、`updated_at`，为 refno 建索引；可选 mesh_ids/has_neg 复合索引。
- 写入：生成/导出阶段计算 combined_tf/geo_transform 后批量 upsert（500~1000 一批）。
- 导出 sqlite：build spatial 阶段从 SurrealDB 拉取，事务写入 `inst_mesh_meta_idx`（refno, mesh_count, has_neg, tf_hash/flag）。
- 查询：房间计算粗筛后按 refno 批量查 SurrealDB，结果入 DashMap 内存缓存；保留旧查询作为回退开关。
- 配置：`prefer_inst_mesh_meta` 默认 true，`batch_size` 可配。

## 实施步骤
1) 数据模型与 SurrealQL 封装：新增 `inst_mesh_meta` model 与 batch upsert/query API（参考现有 inst_geo 封装）。  
2) 生成/导出链路：在 fast_model/gen_model 阶段落表并打日志。  
3) build spatial：新增导出到 sqlite 的瘦索引表，事务写入，缺表自动创建。  
4) 房间计算：改用 `inst_mesh_meta` 批量查询 + 内存缓存，保留回退开关。  
5) 配置与默认：开启 prefer_inst_mesh_meta，暴露 batch_size。  
6) 验证：7997 全链路跑通，耗时与结果对比，记录日志与耗时指标。  

## 风险与缓解
- 同步不一致：以 SurrealDB 为权威；sqlite 每次导出重建表。  
- 数据膨胀：不存 mesh 二进制，仅存 ID/变换；定期统计。  
- 兼容回退：遇缺表或错误自动回退旧查询并报警。  
- 数值精度：统一使用现有变换库，测试设误差阈值。  
