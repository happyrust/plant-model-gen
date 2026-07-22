---
status: accepted
date: 2026-07-22
depends_on: ADR-0002, ADR-0005
---

# 模型生成水位对齐数据水位：欠账追赶闭环

增量链路此前只保证数据版本提交（Committed Watermark 推进），模型生成是可选的一次性动作：`watch-incremental` 默认不生成，生成失败后该区间也不会重试——数据水位推进后增量不再重采集，模型历史出现永久洞。我们决定把"模型生成水位"（每 dbnum 已发布 `model_gen` 锚点的最高 sesno）提升为与数据水位并列的一等公民，由常驻 watch 闭环负责把它追平到数据水位。

关键取舍：

- **锚点语义重定义**：`model_gen` 锚点 = "该 dbnum 模型状态已对齐 sesno N"的声明——纯删除、无模型变更的空操作照发；多段欠账一次追平只发一个锚点（锚在数据水位）；per-dbnum 独立推进；生成成功但后处理失败不发。否决"逐段补发历史锚点"：补发锚点的存储时间戳指向的是当下的 MVCC 切面，`VERSION AT` 会对历史撒谎。
- **欠账记录 = 持久化的增量分类日志**：数据提交成功时同流程幂等写入 `model_gen_debt`（五桶 refno + sesno 区间），追赶按 O(变更数) 消费；欠账区间出现洞时不自动整库重建，告警并等待人工 `catch-up --allow-full-regen`（唯一兜底语义是整库 regen）。否决纯推导式（每次从 MVCC 历史 diff 现算种子：大库大区间分钟级重活，还要新造历史分类器）与纯队列（缺行即卡死等人工）。
- **generate_model 默认翻转为开**："增量 = 数据 + 模型"是默认语义；纯数据同步站点用 `--no-generate-model` 显式声明意图。
- **属性级影响过滤进入采集分类**：仅 Modified 且全部变更属性判定为不影响生成输入的元素不进生成桶（消灭批量改名/改描述触发的无谓重建）；Added/Deleted/OWNER 变化/noun 变化无条件触发；生成链路上未知属性**默认触发**——与 unit-impact 的"未知默认不触发"取向相反，宁多生成不可漏生成。
- **职责边界**：SurrealDB MVCC + `model_gen` 锚点 = 库内行级模型历史（查询用）；ADR-0005 的最小交付单元提交 = 交付历史（发布用）；本闭环只推进前者，不自动触发 unit-export。
- **约束**：mesh `.glb` 按 geo_hash 内容寻址存放在 MVCC 之外——历史可查窗口内禁止清理旧 hash 文件，将来若做 mesh GC 必须以全部 `model_gen` 锚点可达的 geo_hash 集合为存活根；retention 配置为有限窗口的站点，模型历史与数据历史同受窗（维持 ADR-0001 的 retention=0 默认前提）。

实施见 `specs/026-incremental-model-gen-debt-catchup/`；术语（模型生成水位 / 模型生成欠账）见根 `CONTEXT.md`。
