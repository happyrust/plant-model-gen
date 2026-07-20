---
status: accepted
date: 2026-07-20
---

# 版本化统一收敛到 RocksDB versioned，退役 DuckLake 交付链与历史重放

specs/022 为 PE/ATT 引入了 SurrealDB(RocksDB) 实例级 MVCC（`versioned=true`）后，仓内实际存在四套并行的版本机制：存储层 MVCC、DuckLake 交付单元版本（specs/023，键 `(dbnum, refno, sesno)` + release 发布对账链）、"源文件历史重放/物理基线"工具链、以及模型 record id 里的 sesno 槽位（应用层版本化残留，实际恒为 0）。我们决定：**库内一切版本诉求（数据 + 模型）统一由 RocksDB versioned + `sesno_version_anchor` 锚点回答，其余三套全部退役**。

关键取舍：

- **retention=0（无限保留）是本决策的前提**，写入默认配置。站点要享受全历史回溯就承担磁盘只增不减；磁盘受限站点可手工调有限窗口，代价是主动放弃窗口外历史。窗口外唯一兜底是 PDMS 源文件重新解析——不再维护任何重放工具链。
- **交付改为"按锚点导出"**：导出物是一次性产物，版本身份就是导出时的锚点坐标 `(dbnum, sesno)`，废除 release_id 与常驻发布目录/对账状态机。
- **锚点键扩为 `(dbnum, sesno, source)`**，source ∈ {full, incremental, model_gen}：数据就绪与模型生成完成是两个不同的一致性时刻，必须分别锚定；"查 sesno=N 的模型"取 model_gen 锚点（最近不大于回退），"查 sesno=N 的数据"取 full/incremental 锚点。
- **模型 record id 收敛为纯 refno 键**（`[ref0, ref1]` 系），历史职责完全交给存储层；模型库定位为可再生缓存，id 形状切换采用硬切换（versioned 建库属性本就要求新目录重灌，两个迁移边界天然重合），不做双读兼容层。
- **ModelWriter 的 DuckLake/Parquet 写入后端随交付链一并退役**（`ModelWriterMode` 收敛为 Surreal/DrainOnly）；面向 web 查看器的 Parquet 导出器保留——它是导出物本体，与版本机制无关。

被否决的替代方案：① 保留 DuckLake 作为交付版本层与 022 共存（原 023 决策）——两套版本真相源的对账与心智成本在 retention=0 后不再有收益支撑；② 应用层 sesno 版本化（record id 携带 sesno、多代共存）——会击穿所有按 refno 前缀 range 扫描的读路径，改造面远大于存储层方案。

实施计划见 `specs/024-unified-rocksdb-versioning/`；术语定义见根 `CONTEXT.md`。
