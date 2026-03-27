# 模型生成方案总览（运行态事实版）

> 更新时间：2026-03-26  
> 适用仓库：`/Volumes/DPC/work/plant-code/plant-model-gen`  
> 目标：整理当前仓库内**仍然生效**的模型生成方案、导出方案、调试方案、截图方案与常见问题修复点，作为后续同步到 Linear 文档的事实源。

---

## 1. 文档定位

本文不是历史设计稿汇总，也不是纯性能优化方案合集，而是面向当前运行态的“事实版地图”。

重点回答以下问题：

1. 当前 `aios-database` 支持哪些模型生成/导出方案？
2. 这些方案分别走哪条代码链路？
3. 当前的事实源是什么？
4. 哪些旧文档仍可参考，哪些已经不能当作运行态事实？
5. 最近一次真实问题闭环（`21909_10209`）修到了哪里、如何验证？

---

## 2. 当前事实源与边界

### 2.1 当前事实源

以下内容以当前代码和运行态入口为准：

- CLI 主入口：`src/main.rs`
- CLI 模式分发：`src/cli_modes.rs`
- 模型生成编排器：`src/fast_model/gen_model/orchestrator.rs`
- TreeIndex 查询入口：`src/fast_model/gen_model/query_provider.rs`
- OBJ 导出与截图：`src/fast_model/export_model/export_obj.rs`
- 输出目录与 `db_meta_info.json` 事实源：`src/options.rs`
- 项目初始化（scene_tree / db_meta）：`src/init_project.rs`

### 2.2 当前不再作为运行态事实源的内容

以下文档可作为历史背景，但**不能直接当作当前实现说明**：

- `docs/guides/MIGRATION_GUIDE.md`
  - 含“方案 A / 方案 B”式迁移说明，部分内容偏历史阶段。
- `docs/plans/2026-02-14-model-generation-optimization-perf.md`
- `docs/plans/2026-02-14-model-generation-optimization-hardening.md`
  - 主要描述阶段性性能/硬化任务，不等于当前总览。
- `docs/guides/MODEL_RELATION_STORE_MIGRATION.md`
  - 偏迁移/替换说明，不是完整生成方案地图。

---

## 3. 当前模型生成主链路

## 3.1 CLI 总入口

`src/main.rs` 负责：

1. 解析 `--regen-model / --debug-model / --export-* / --capture` 等参数
2. 判断是否先执行生成，再执行导出
3. 构造 `ExportConfig`
4. 调用 `cli_modes::run_regen_model()` / `cli_modes::run_generate_model()` / `export_obj_mode()` 等模式函数

当前关键语义：

- `--regen-model`
  - 清理后重新生成
- `--debug-model`
  - 增量/定向生成，不做整套清理
- `--export-*`
  - 默认是**纯导出**，除非显式叠加 `--regen-model`
- `--capture`
  - 与导出链路联动，在 OBJ 导出后生成预览截图

## 3.2 生成编排器

当前统一编排入口：

- `src/fast_model/gen_model/orchestrator.rs::gen_all_geos_data()`

运行态事实：

- 当前生成链会初始化 `cache_miss_report`，模式标记为 `Direct`
- 代码里已明确写明：
  - `cache-first 模式已移除（foyer-cache-cleanup），使用 Direct 模式`

也就是说，当前模型生成主线已经不是旧文档里那种“foyer cache-first”叙述，而是：

> **TreeIndex 做层级查询 + Direct 做几何生成与写出。**

## 3.3 TreeIndex 角色

TreeIndex 的主要职责不是直接生成 mesh，而是：

- 负责层级展开
- 负责按 root / noun / descendants 收集 refno
- 负责 `db_meta_info.json` 支持的 refno → dbnum 映射

关键入口：

- `src/fast_model/gen_model/query_provider.rs`
  - 启动时优先使用 TreeIndex 查询提供者
  - 若 `.tree` 不存在，会尝试自动生成 scene_tree
- `src/init_project.rs`
  - `run_init_project_mode()` 第一步即生成 `scene_tree`

---

## 4. 当前支持的模型生成方案（运行态）

下面按“实际使用场景”整理。

## 4.1 方案 A：项目初始化方案（scene_tree / db_meta 预热）

### 目标

在正式生成模型前，先初始化项目级索引与元数据。

### 入口

- `src/init_project.rs::run_init_project_mode()`

### 核心步骤

1. 生成 `scene_tree`
2. 写出 `output/<project>/scene_tree/*.tree`
3. 写出 `output/<project>/scene_tree/db_meta_info.json`
4. 初始化 Surreal 连接并加载 `db_meta`

### 适用场景

- 新项目首次接入
- 切换 `DbOption` 配置后重新建立 scene_tree
- 排查“refno 无法映射 dbnum”“Tree 索引缺失”等问题

### 关键事实

- 当前 `db_meta_info.json` 是 refno → dbnum 的事实源
- 不允许把 ref0 直接当 dbnum 猜测

---

## 4.2 方案 B：全量重建方案（CLI `--regen-model`）

### 目标

对目标范围执行清理后重新生成模型关系、几何实例、mesh、布尔结果等。

### 入口

- `src/main.rs` 解析 `--regen-model`
- `src/cli_modes.rs::run_regen_model()`
- 最终进入 `gen_all_geos_data()`

### 当前语义

- `--regen-model` 单独使用：只重建，不导出
- `--regen-model + --export-*`：先重建，再导出

### 常见命令

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --regen-model \
  --dbnum 5525
```

### 适用场景

- 模型缓存/关系已过期
- 修改了几何解析逻辑后需要重建
- 闭环验证某个 refno 的真实生成结果

---

## 4.3 方案 C：定向增量生成方案（CLI `--debug-model`）

### 目标

只对指定 refno（或提升后的 BRAN/HANG 根）做定向生成，避免全库重建。

### 入口

- `src/main.rs`
- `src/cli_modes.rs::run_generate_model()`

### 当前语义

- `--debug-model` 倾向于“补齐缺失、增量生成、调试验证”
- 不做 `regen` 的全量清理
- 与 `--export-obj` 组合时，当前会自动截图（若未显式指定 `--capture`）

### 常见命令

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --debug-model 21909_10209 \
  --export-obj
```

### 适用场景

- 单模型定位问题
- 复现某个具体 refno 的生成异常
- 修复几何语义后快速回归

---

## 4.4 方案 D：按 refno 重建并导出方案（`--regen-model + --debug-model/--export-obj-refnos`）

### 目标

对单个或少量 refno 做“重建 + 导出 + 截图”闭环。

### 当前推荐命令

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --regen-model \
  --debug-model 21909_10209 \
  --capture /tmp/21909_10209_capture_fix3 \
  --capture-width 1200 \
  --capture-height 900
```

### 适用场景

- 修复某个几何 bug 后立即闭环验证
- 输出 OBJ 与 PNG 供人工比对
- 排查“是不是旧缓存没刷新”

---

## 4.5 方案 E：纯导出方案（不触发生成）

### 目标

只查询当前 DB/缓存中的几何关系并导出，不重新生成模型。

### 入口

- `src/main.rs`
- `src/cli_modes.rs::export_obj_mode()`
- `src/fast_model/export_model/export_obj.rs`

### 当前语义

见 `docs/guides/EXPORT_COMMANDS.md`：

- 默认 `--export-*` 是**纯导出**
- 不会自动触发模型生成

### 常见命令

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --export-obj-refnos 21909_10209 \
  --export-obj-output /tmp/21909_10209.obj
```

### 适用场景

- 仅需要导出已有结果
- 对照 DB 中当前几何实例与导出效果

---

## 4.6 方案 F：截图预览方案（`--capture`）

### 目标

在 OBJ 导出后自动生成 PNG 预览图，方便人工核对。

### 入口

- `src/main.rs` 设置 `CaptureConfig`
- `src/fast_model/export_model/export_obj.rs::maybe_capture_obj_preview()`

### 当前事实

- 之前 refno 导出路径存在“OBJ 导出了，但 `--capture` 不落图”的问题
- 当前已在 `export_obj_for_refnos()` 路径补上截图调用
- 现在 `--capture` 已可用于真实回归验证

### 常见命令

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --regen-model \
  --debug-model 21909_10209 \
  --capture /tmp/21909_10209_capture \
  --capture-width 1200 \
  --capture-height 900
```

---

## 4.7 方案 G：Web / 流式生成方案

### 目标

服务化场景下按 refno 或任务流式推进生成。

### 相关代码

- `src/web_server/stream_generate.rs`
- `docs/模型接口/refno_model_generation_api.md`

### 当前边界

- Web 侧存在 `stream_generate` 与 `generate-by-refno` 相关设计/实现
- 但本文以当前 `aios-database` CLI 运行态为主
- 若要联调 Web，必须走真实服务与 POST 请求，而不是 test

---

## 5. 当前 refno / dbnum 规则

这是近期排障里最重要的一条运行态事实。

## 5.1 规则

> **ref0 != dbnum**

例如：

- `21909_10209` 中的 `21909` 只是 `ref0`
- 不等于真实 dbnum

## 5.2 真实事实源

当前应统一依赖：

- `output/<project>/scene_tree/db_meta_info.json`
- 以及 `db_meta_manager`

对应路径获取逻辑见：

- `src/options.rs::get_db_meta_info_path()`

## 5.3 当前生效行为

在 refno 定向生成/导出场景下：

- 应先根据 refno 推导真实 dbnum
- 不应直接把 CLI 里的 `--dbnum` 或 ref0 当事实源

这条规则已经在近期修复中被明确收口。

---

## 6. 当前几何语义与可见性规则

## 6.1 `tube_flag=false`

当前结论：

> **`tube_flag=false => invisible` 是正确规则。**

它代表占位/包络体，默认不作为可见实体导出。

## 6.2 负实体 / 布尔结果

当前模型语义仍依赖以下数据：

- `neg_relate`
- `inst_relate_bool`
- `ngmr_relate`

如果某条实例没有负实体与布尔结果，那么导出就是按可见正实体 primitive 直接拼装的结果。

## 6.3 SCYL 负高度的关键修复

近期闭环问题表明：

- `SCYL` 在 `height_raw < 0 && centre_line_flag = true` 时
- 之前使用了错误的半高偏移方向

修复点在：

- `../rs-core/src/prim_geo/category.rs`

修复后的正确逻辑是：

- `rotation` 与 `translation` 应统一使用 `effective_dir = axis_dir * sign(height_raw)`
- 不能 `rotation` 用 signed direction，而 `translation` 仍用原始 `axis_dir`

关联信息：

- Issue：`happyrust/rs-core#2`
- Commit：`f20a7fb` (`fix: correct negative scyl center offset direction`)

---

## 7. 当前常用验证方案（仅 CLI，不跑 test）

## 7.1 仅重建

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --regen-model \
  --dbnum 5525
```

## 7.2 单 refno 重建 + 截图

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --regen-model \
  --debug-model 21909_10209 \
  --capture /tmp/21909_10209_capture_fix3 \
  --capture-width 1200 \
  --capture-height 900
```

## 7.3 纯 OBJ 导出

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --export-obj-refnos 21909_10209 \
  --export-obj-output /tmp/21909_10209.obj
```

## 7.4 refno 定向导出（非调试模式）

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen

./target/debug/aios-database \
  --config db_options/DbOption-zsy \
  --export-obj-refnos 21909_10209 \
  --capture /tmp/21909_10209_capture_only
```

---

## 8. 近期闭环案例：`21909_10209`

## 8.1 问题概述

`21909_10209` 的 OBJ/截图结果与期望阀门形态不一致，排查过程中先后暴露出多层问题。

## 8.2 已闭环的问题

### 问题 1：占位大圆柱遮挡

- 根因：`tube_flag=false` 的占位体被错误导出为可见几何
- 处理：当前规则收敛为 `tube_flag=false => invisible`

### 问题 2：ref0 被当成 dbnum

- 根因：refno 定向场景中，`21909` 被错当 dbnum 使用
- 处理：统一以 `db_meta_info.json` 映射 dbnum，不能猜

### 问题 3：`--capture` 在 refno 导出路径不生效

- 根因：`export_obj_for_refnos()` 未调用截图逻辑
- 处理：补齐 `maybe_capture_obj_preview()`

### 问题 4：SCYL 负高度圆盘方向反了

- 根因：`rotation` 用 `effective_dir`，但 `translation` 半高偏移仍用原始 `axis_dir`
- 处理：统一使用 `effective_dir`
- 结果：左右圆盘方向已恢复正确

## 8.3 仍需单独追踪的问题

### 横向主体圆柱 `16596_10591`

当前独立待查问题是：

- 横向主体圆柱长度为什么是 `238`
- 它与两侧圆盘内侧面净距 `292` 的关系是否符合 catalogue 原意

这已不属于本轮已解决问题，而是下一条独立几何语义问题。

---

## 9. 与现有仓库文档的关系

## 9.1 继续保留、可引用

- `docs/guides/EXPORT_COMMANDS.md`
  - 适合作为命令速查表
- `docs/模型接口/refno_model_generation_api.md`
  - 适合作为 Web 按 refno 生成接口设计说明
- `docs/plan-export-dbnum-instances-web.md`
  - 适合作为 delivery-code 兼容导出方案说明

## 9.2 需要带着“历史视角”阅读

- `docs/guides/MIGRATION_GUIDE.md`
- `docs/plans/2026-02-14-model-generation-optimization-perf.md`
- `docs/plans/2026-02-14-model-generation-optimization-hardening.md`
- `docs/guides/MODEL_RELATION_STORE_MIGRATION.md`

这些文档仍有参考价值，但不应直接替代本文的运行态事实。

---

## 10. 对 Linear 文档同步的建议

当前仓库内未发现明确的 Linear 文档目录或自动同步入口，因此建议采用以下方式：

1. 以本文作为 **Linear 源稿**
2. 后续将本文内容同步到 Linear 的“模型生成方案总览”页面
3. 若未来仓库增加 Linear 文档同步机制，再将本文纳入自动同步源

建议在 Linear 中拆成以下子页面：

- 模型生成总览
- CLI 方案与命令
- 导出与截图方案
- 几何语义与布尔规则
- 典型问题闭环案例（`21909_10209`）

---

## 11. 一句话总结

当前 `plant-model-gen` 的模型生成主线可以概括为：

> **TreeIndex 负责层级与 dbnum 映射，Direct 模式负责几何生成，CLI 通过 `regen/debug/export/capture` 组合出不同调试与交付方案；近期已修复 ref0/dbnum 误判、refno 导出不截图、以及 SCYL 负高度圆盘方向反转等关键问题。**
