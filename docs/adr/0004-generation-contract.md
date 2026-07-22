---
status: accepted
date: 2026-07-21
depends_on: ADR-0003
---

# 用生成契约标识模型语义

一次模型生成在开始时解析一份不可变的 `GenerationContract`。契约只包含会改变模型结果或结果完整性的语义配置：契约与几何算法版本、规范化后的 noun 启用和排除规则、debug 限制、验证策略、mesh 开关与精度、boolean 开关与模式、TUFL 结果过滤策略，以及 dry-run、跳过 AABB、跳过最终 sweep 等完整性策略。noun 名称统一大写、排序、去重后参与稳定哈希。

并发度、batch 大小、channel capacity、读写 backend、输出路径、导出格式及性能报告开关属于 `ExecutionTuning`，不得进入契约哈希。读取 backend 仍受 ADR-0003 的单会话约束，但 backend 选择不定义模型语义身份。

生成范围单独规范化为 `GenerationTargets`。Full、root-scoped 与 Incremental adapter 只负责产生已排序、去重的目标集合；同一个 executor 按 `BRAN/HANG → LOOP → CATE → PRIM` 执行。目标哈希与输入 manifest 哈希、契约哈希一起写入运行结果、日志和性能报告，用于定位“输入、语义、范围”三类差异。现有 `model_semantic_hash` 算法保持不变。

运行结果还记录仅覆盖 geometry producer 输出的 `geometry_artifact_hash`。它不包含 mesh、boolean 或 writer 后处理，可用于比较 Surreal 与 DrainOnly 是否消费了相同几何产物。

写入后端和 DrainOnly capability 共享同一 geometry producer 与内部 write pipeline。该 pipeline 隐藏 channel、worker、barrier、cleanup、final sweep 和关系 reconcile；导出与性能报告仍由调用 workflow 负责。

本决策不新增模型版本发布表，不设计发布或迁移协议，不改变数据库表、导出格式、boolean 算法或 mesh 算法。模型版本发布协议留给后续 ADR。
