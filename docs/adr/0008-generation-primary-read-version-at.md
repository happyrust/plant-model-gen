---
status: accepted
date: 2026-07-22
depends_on: ADR-0007
supersedes: ADR-0003
---

# 模型生成主表直读：保留领域查询 trait，增量运行以单一 VERSION AT 时刻钉住

ADR-0003 的「固定版本读取会话 + 双后端 + 副本表」随 DuckLake 退役（ADR-0007）失去数据源。我们决定：**模型生成改为直读主 PE/ATT 表，保留领域查询 trait 边界（生成代码不含 SQL/record-id），删除 `generation_replica_element/hierarchy/reference/transform/db_catalog` 五张副本表、snapshot 绑定与 manifest 双向哈希校验**。

关键取舍：

- **全量生成（解析后首跑）活读当前态**——此时无版本，符合「版本只由增量产生」。
- **增量生成以单一 as-of 时刻钉住**：会话打开时取本次数据提交锚点的 `anchored_at`，所有读加 `VERSION AT T`。一个时间戳即一个一致的 MVCC 切面，防同进程并发提交（web 触发生成与 watch 提交同时跑）造成撕裂读；RocksDB 时间戳读成本近零。否决纯活读（撞上并发提交时静默读到半截、跨 dbnum 不一致无告警）。
- **「版本读取会话」降级为「生成读取时刻」**：一次生成绑定一个读取时刻；「输入版本清单」从绑定契约降级为观测记录（会话打开时各 dbnum 水位写进运行结果，供复现解释，不再 fail-closed 卡覆盖）。
- **模型水位起点握手**：全量生成成功收尾时发布一条 `model_gen` 锚点（sesno=解析基线，取 `dbnum_info_table` 当前值）——这是生成闭环自己的起点声明，由生成完成动作写入，解析路径仍零痕迹；spec-026 FR-002 的水位数学零特判成立，新站首轮 watch 不误报 needs_full_regen；全量生成漏跑时水位诚实为 0 → 告警 → 人工 `catch-up --allow-full-regen`。该行为即现有 `publish_model_gen_anchors_after_generation` 路径，本决策将其固定为原则。
- 保留 ADR-0003 唯一的长期资产：**批量事实读取 trait**。新适配器直读主表（长期生产验证过的 query 层路径）；层级遍历、CATA 闭包、排序、缺失语义仍由共享领域代码实现。

被否决的替代方案：① 纯活读不钉时刻——省一个时间戳换一个静默坑；② 保留副本表改由增量直接喂——白养五张表与复制链，只为躲开本来就在生产跑的主表读。
