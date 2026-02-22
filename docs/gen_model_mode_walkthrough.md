# Index Tree 模式：模型生成流程分析与总结

在当前架构中，模型生成流程完全重构并进入了“Index Tree 优先”的高度模块化与并行阶段。整个流程可以通过入口 `orchestrator.rs` 追踪至核心管线 `index_tree_mode.rs`，包含了细粒度的预处理、预取缓存和基于分阶段的几何模型并行生成机制。

以下是当前模型生成流程的核心执行步骤：

## 一、 准备与预检查阶段 (Precheck Phase)
发生在生成动作正式进入编排器之前，由 `precheck_coordinator::run_precheck` 处理：
1. **范围确定**：根据配置 (`DbOption.toml` 内的 `manual_db_nums`, `exclude_db_nums`)，合并本地或 SurrealDB 数据提取出涉及到的数据库编号 (`dbnum`) 集合。
2. **数据完整性校验**：
   - 检查并提取 `db_meta_info.json`。
   - 检查各个数据库是否已存在由 PDMS 原始数据转化的 `Tree 索引文件` (`.tree`)。若缺失则当场触发全量同步拉取，生成所需的树形信息结构。
   - 检测本地变换缓存 `transform_cache`（foyer 本地缓存位置），确保基础路径环境就绪。

## 二、 核心生成流水线 (Gen Index Tree Geos Optimized)
预检查完成后进入了 `gen_index_tree_geos_optimized` 方法。
这个核心管线的设计原则是：**分类处理、严格依赖排序（BRAN优先 -> LOOP -> PRIM -> CATE）、利用流式并发或两阶段高速缓存。**

### 1. 阶段一：BRAN/HANG 核心管线 (Pipeline Phase 1)
管道与支吊架（`BRAN`/`HANG`）由于包含复杂连通拓扑和依赖，会作为独立的“第一阶段优先”进行深度遍历生成：
- **树索引查找根节点**：快速调用 Tree Index 从指定 DB 筛选出指定的根。
- **采集所有的后代组件**：获得根下包含的 CATE 数据。
- **构建哈希重用映射 (`cata_hash_map`)**：这是渲染重用 CATE 组件的依据。
- **预热缓存与预取 (Prefetch)**：如果处于离线/缓存两阶段生成配置（`PrefetchThenGenerate`），此处会强制把世界坐标计算结果、实例详细参数存入本地缓存，并在缓存准备完成后切断与数据库联系。
- **产生并下翻集合体配置信息**：生成 `CATE` 以及专精构建基于轨迹的 `Tubing`（管道管壁结构与弯头等），生成的模型发送进输出通道。

### 2. 阶段二：通用深度查询路径 (Pipeline Phase 2)
在第一阶段完成后，流程将收集其余的 `Noun` 根节点（如果未经过配置隔离屏蔽）。
通过查询预建的 `TreeIndex` 提取出所有 `LOOP`、`PRIM` 和非管道组件的 `CATE` 数据并放入执行集合：
- **全量递归收集**：基于查得的入口 roots 递归检索所含有 `LOOP` / `PRIM` / `CATE`。
- **预取流程 (Prefetch)**：同样按照缓存优先政策，尝试合并网络 I/O，将所有依赖数据灌入 `geom_input_cache`（图元缓存）与 `cata_resolve_cache`（字典缓存）。
- **依序并行生成**：
  严格遵循如下依赖顺序分批次分段执行逻辑：
  1. **`process_loop_stage`**：处理 `LOOP` 图形。
  2. **`process_prim_stage`**：处理原生图元 `PRIM` 。
  3. **`process_cate_stage`**：处理剩余的定义件 `CATE`。

### 3. 基于流式通道的高速生成 (Streaming Generation Scheme)
流程中值得关注的优化是其引用的 **通道流式分发机制**。当符合条件（开启指定环境变量、并选择走内存优先时）：
生成逻辑不再是传统的“请求一页 -> 生成一页”，而是解耦成了“生产者-消费者模型”：
* **生产者任务 (Producer)**：按照分页向后异步预取解析所需的 `Input` 块，送入缓冲通道 (`flume::bounded`)。
* **消费者任务 (Consumer)**：不断收取拿好的依赖数据：
  1. 将数据顺手拷贝镜像入本地缓存提供持久化支持。
  2. 交由下游实际运算执行生成三角网格的任务。该步骤在极高的多线程配比下可吃满所有核心运算单元并直接下发结果。

## 三、 收尾工作与聚合落盘
在全管线的不同生产者、类别均将计算好的三角网格下推至同一个接受 Channel（即上方的 `sender: flume::Sender<ShapeInstancesData>`）后：
后台负责网格收集的 `mesh_worker`、`boolean_worker` 以及 `AABB_worker` 将会陆续完工。随后完成将：
1. 各网格块写入输出存储服务（写入 SurrealDB / SQLite Rtree 结构树中）。
2. 构建并关联组件边界球（AABB Box）记录以供后期轻型房间框选遮挡剔除查阅。
3. 提供相关耗时统计供排查与性能剖析（Console Summary）。
