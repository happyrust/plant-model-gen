# Feature Specification: EQUI/TMPL 模板原语生成补齐(spec 011)

## User Need

在 `aps250160_0001` 的按需 CATA 解析与模型生成流程中,以 E3D 官方 RVM 导出
为基准对拍 `ZONE 2013286704/821`(`/03SKID3`)时,生成结果缺少两个料仓设备
及其模板原语。用户需要生成端覆盖 `EQUI -> TMPL` 子树中的参数化几何
(`CYLI` / `DISH` / `SNOU` / `PYRA` 等),使生成模型与 RVM 基准在成员清单和
几何可见性上保持一致。

## Current State(已探明,2026-06-13)

### 对拍结论

- spec 009 的 RVM 导入与身份解析已支持 821 ZONE 样本,解析率 `127/127(100%)`。
- 重新生成后,对拍仍稳定出现 `missing=35`。
- 其中可确认的生成端缺陷是 `EQUI` 模板几何全缺:
  - `EQUIP1` / `EQUIP2` 的 `TMPL` 下 `CYLI(906/998)`,
    `DISH(907/999)`, `SNOU(908/1000)`;
  - `SUBE -> TMPL` 下 `PYRA(941/942/1033/1034)`;
  - 以及关联的 `NOZZ(852/909/944/1001)`。
- `JLDATU` / `PLDATU` 属 datum 定位辅助件,当前目标 noun 不包含,先进入豁免桶;
  `BRAN` 是既有管段口径差异,不在本 spec 修。

### 关键诊断证据

生成日志显示 PRIM 阶段并非整体未运行:

- `[Prim:CYLI] 168`
- `[Prim:DISH] 34`
- `[Prim:SNOU] 4`
- `[Prim:PYRA] 106`

但 821 样本中的目标 refno 没进入最终生成结果。用现有
`examples/dump_tree_node.rs` 直接检查 release 站点产物
`250160.tree` 后,已排除“树文件缺节点/子树不可达”的早期假设:

```text
tree: ...\quicktest-250160-8080\output\AvevaPlantSample\scene_tree\250160.tree
roots: [2013278512_0, 2013286704_1327]

2013286704/821 exists=true
  ancestors: 2013278512_0 -> 2013286704_11
  children: 822, 835, 851, 943, 1035

2013286704/906 exists=true
  ancestors: 2013278512_0 -> 2013286704_11 -> 2013286704_821 -> 2013286704_851 -> 2013286704_3535

2013286704/907 exists=true
  ancestors: 2013278512_0 -> 2013286704_11 -> 2013286704_821 -> 2013286704_851 -> 2013286704_3535

2013286704/908 exists=true
  ancestors: 2013278512_0 -> 2013286704_11 -> 2013286704_821 -> 2013286704_851 -> 2013286704_3535

2013286704/941 exists=true
  ancestors: 2013278512_0 -> 2013286704_11 -> 2013286704_821 -> 2013286704_851 -> 2013286704_910 -> 2013286704_3536

2013286704/998/999/1000/1033/1034 exists=true
```

因此当前排名最高的根因假设是:

1. 第二阶段 BFS 能看到目标节点,但 `collect_target_refnos_grouped` 或分类集合没有把这些
   821 子树目标 refno 放入本轮处理集合。
2. PRIM 处理器处理了同 noun 的其他 refno,但目标 refno 在 `process_*_refno_page`
   或其输入预取/过滤阶段被跳过。
3. 目标 refno 生成了 `ShapeInstancesData`,但写入/导出端没有落到
   `instances.parquet`。

## Final Result(2026-06-14 复验)

本 spec 的生成端缺陷已收敛并修复:

- `CYLI/DISH/SNOU/PYRA` 根因是两层叠加:
  - precise CATA closure 旧口径只纳入 `CATA`,未解析外部模板 `DESI`
    (`acp7015_0001/dbnum=7015`);
  - 即使 `DESI` 入库,模板 PRIM 源记录的标准尺寸仍为 0,实际参数来自
    父级 `EQUI/SUBE.DESP` 并由 `TMPL -> DDSE -> DDAT(DKEY/DDPR)` 映射。
- `NOZZ` 根因是 catalogue 引用链未归一化:
  `NOZZ.CATR -> SPCO.CATR -> SCOM` 在 `resolve_desi_comp` fallback 场景下
  未被解到真实 `SCOM`,导致设计件自身被当作 `tubi_scom` 求解,
  trace 表现为 `is_tubi=true` 且 `gm_out=0/ngm_out=0`。
- 修复后 821 ZONE Parquet 抽查:
  - `NOZZ 852/909/944/1001` 均进入 `instances.parquet`,每个 2 行
    `geo_instances`;
  - `CYLI/DISH/SNOU/PYRA` 目标均进入 `instances.parquet`,每个 1 行
    `geo_instances`。
- 新 RVM compare 报告
  `runtime/rvm-compare/rvm-compare-8647000551151633205-20260614-083451.json`
  中,本 spec 的 `EQUI/TMPL` 模板目标 `14/14 matched`;总 missing 从修复前
  35 降到 21,剩余项为 4 个 BRAN 与 17 个 JLDATU/PLDATU 表示差异。
- 476 BRAN 回归:
  `matched=8`, `extra_in_gen=0`, `noun_mismatch=0`;`missing_in_gen=1`
  为 spec 009 已记录的 BRAN 自身 TUBE 挂组口径差异。

## Scope

- 建立一个可复跑的 821 ZONE 诊断闭环,精确断言 906/907/908/941/942/998/999/1000/1033/1034
  在 TreeIndex、阶段输入、处理器输出、writer/export 四个边界的状态。
- 修复生成管线中导致 `EQUI -> TMPL` / `SUBE -> TMPL` 参数化几何漏生成的代码路径。
- 将 RVM 对拍报告中豁免桶外的 `EQUI` 模板 PRIM missing 降为 0。
- 保留 spec 009 导入侧修复与 `--compare-rvm` 口径,本 spec 只处理生成端。

## Non-Goals

- 不在本 spec 修复 `rvm-rs` 层级变换组合问题。
- 不把 `JLDATU` / `PLDATU` datum 件加入生成目标。
- 不改变 BRAN/TUBI 的 RVM 表示差异口径。
- 不做全量性能优化或 UI 改动。

## Acceptance Criteria

1. `dump_tree_node` 或后续正式诊断命令能证明 821 目标节点在 `.tree` 中存在且从 ZONE 可达。
2. 新增诊断在修复前能稳定指出目标 refno 在哪一个生成边界丢失。
3. 修复后,821 ZONE 对拍中 `CYLI/DISH/SNOU/PYRA/NOZZ` 的豁免桶外 missing 为 0。
4. `2013286704/476` BRAN 样本对拍不回归。
5. 相关测试或最小 CLI 验证命令可由新人按 `tasks.md` 复跑。
