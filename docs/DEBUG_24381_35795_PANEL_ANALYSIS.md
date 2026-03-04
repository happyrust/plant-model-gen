# 24381/35795 (1RX-RM05) Panel 模型生成检查报告

## 测试命令

```bash
cargo run --bin aios-database -- --debug-model 24381/35795 --regen-model --export-obj
```

## 一、默认配置下的问题（index_tree_enabled_target_types = ["BRAN"]）

| 项目 | 结果 | 说明 |
|------|------|------|
| Refno 分类 | Cate=0, Loop=0, Prim=0 | 24381/35795 是 Room (PANE)，子树中无 BRAN |
| BRAN 收集 | 0 个 | `collect_target_refnos_pruned` 从 Room 出发 BFS 遇不到 BRAN |
| 第二阶段 | 跳过 | target_nouns 仅 ["BRAN"]，排除后为空 |
| OBJ 导出 | ⚠️ 跳过 | "无几何可导出" |

**结论**：默认只启用 BRAN 时，Room 下的 Panel (PANE) 不会被模型生成管线处理。

---

## 二、扩大 noun 范围后的结果（--gen-nouns BRAN,GWALL,WALL,FLOOR,PANE）

| 项目 | 结果 |
|------|------|
| BFS 收集 | 20 个 PANE refno (loop=20) |
| 几何生成 | 20 个 PANE 进入 LOOP 流程 |
| 成功生成 Mesh | 5 个 |
| CSG 失败 | 10 个（三角化索引映射失败） |
| OBJ 导出 | ✅ 成功，含 5 个 panel 几何 |

### 成功生成的 PANE (5 个)

- 24381_35798 (geo_hash=3395906195227906396)
- 24381_35931 (geo_hash=17362429290346988789)
- 24381_35947 (geo_hash=17516989368330364487)
- 24381_35983 (geo_hash=6766364274865118194)
- 24381_36061 (geo_hash=9742907310070725768)

### CSG 失败的 PANE (10 个，典型 geo_hash)

| geo_hash | 错误 |
|----------|------|
| 11868052209961831489 | 三角化索引映射失败 idx=3, best_d=0.00048828125 |
| 3080706177900460082 | idx=10, best_d=0.00024414063 |
| 162033681244479065 | idx=2, best_d=0.00024414063 |
| 14160015242993409933 | idx=4, best_d=0.00012207031 |
| 13500363943519248336 | idx=5, best_d=0.00012207031 |
| 11762737619811663171 | idx=5, best_d=0.00024414063 |
| 10313162117607086028 | idx=2, best_d=0.00039672852 |
| 4629334266638812066 | idx=1, best_d=0.00011328049 |
| 16497329568412439380 | idx=46, best_d=0.00021362305 |
| 8065548986903304905 | idx=1, best_d=0.00012207031 |
| 10705972727916437426 | idx=3, best_d=0.00015640259 |

---

## 三、结论：Panel 模型生成状态

| 检查项 | 状态 | 说明 |
|--------|------|------|
| 所有 panel 都生成 | ❌ 否 | 20 个 PANE 中仅 5 个成功，10 个因 CSG 失败无 mesh |
| 默认配置可生成 | ❌ 否 | 需 `--gen-nouns BRAN,GWALL,WALL,FLOOR,PANE` 或修改 DbOption |
| OBJ 导出 | ✅ 部分 | 成功导出 1RX-RM05.obj，含 5 个 panel |

---

## 四、建议

1. **生成 Room/Panel 模型**：在 `DbOption.toml` 中增加 PANE 或使用：
   ```bash
   cargo run --bin aios-database -- --debug-model 24381/35795 --regen-model --export-obj --gen-nouns BRAN,GWALL,WALL,FLOOR,PANE
   ```

2. **修复 10 个 CSG 失败**：`Extrusion ProfileProcessor 三角化索引映射失败` 多为轮廓顶点与三角化结果索引不一致，需在 `plant-model-gen` 的 extrusion 管线中排查：
   - 检查 polyline 顶点顺序与 ear-clipping 三角化输出
   - 放宽/调整 `best_d` 容差或索引映射逻辑

3. **缺失 5 个**：20 个中 5 成功 + 10 失败 = 15，另有 5 个可能为负实体或未进入 mesh 流程，可进一步查日志确认。
