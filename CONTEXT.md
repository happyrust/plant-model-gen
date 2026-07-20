# plant-model-gen 领域术语表

PDMS/E3D 工厂设计数据的解析、模型生成与版本化服务。本表是全仓唯一术语真相源；实现细节不在此记录（见 `specs/` 与 `docs/adr/`）。

## 版本与历史

**sesno（设计会话号）**:
PDMS/E3D 源库的业务版本号，随每次设计会话递增。全系统唯一的业务版本坐标，配合 dbnum 使用：`(dbnum, sesno)`。
_Avoid_: 版本号、session id

**数据版本（数据历史）**:
PE/ATT 源数据行在 versioned 存储中的历史状态，按 `(dbnum, sesno)` 经数据锚点定位。
_Avoid_: 数据快照、源版本

**模型版本（模型历史）**:
模型表（几何实例与关系）在 versioned 存储中的历史状态，按 `(dbnum, sesno)` 经模型锚点定位。模型库本身是可由数据重新生成的缓存。
_Avoid_: 交付单元版本、unit version、release

**版本锚点**:
业务版本号与存储时间戳之间的唯一桥梁，键 `(dbnum, sesno, source)`。source 表达锚定语义：`full`/`incremental` = 该 sesno 的数据就绪时刻；`model_gen` = 该 sesno 的模型生成完成时刻。"有锚点 = 该语义下的一致快照"。
_Avoid_: 版本标记、checkpoint

**导出物**:
按锚点从库内导出的一次性交付文件（parquet/glb 等）。导出物没有独立的版本身份，其版本就是导出时使用的锚点坐标。
_Avoid_: release 包、交付单元、release_id

## 增量链路

**增量更新**:
以 sesno 区间从源 db 文件收集元素变更、落库 PE/ATT、并按变更分类驱动模型重生成的流程。唯一常驻入口是 watch-incremental 轮询。
_Avoid_: 热更新、实时同步

**增量分类日志（IncrGeoUpdateLog）**:
一次增量收集产出的变更 refno 分桶（prim / loop_owner / bran_hanger / basic_cata / delete），是增量模型生成的种子。

**重生成清理（pre_cleanup）**:
覆盖式重建前删除目标 refno 及后代的旧模型产物的步骤，是"插入即忽略"写入模式的对偶操作；被删数据的历史由 versioned 存储层保留。

## 已废除术语（历史文档中出现时一律按废除理解）

- **交付单元版本 / unit version / release_id**：由 DuckLake 交付链承载的版本身份，已整体退役（ADR-0001）
- **历史重放 / 物理基线**：用源文件重放历史的机制，被存储层时间旅行取代
- **模型 record id 的 sesno 槽位**：`[ref0, ref1, sesno]` 形状的应用层版本化残留，已删除
