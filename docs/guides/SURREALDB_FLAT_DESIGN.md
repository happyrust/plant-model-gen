# SurrealDB 极简方案：单表存储关联数据

## 核心设计

**不引入新存储，直接在 SurrealDB 中使用扁平化设计**

```surql
-- 单表设计
CREATE TABLE refno_relations;

-- 数据结构
{
    id: refno_relations:123,
    refno: 123,
    dbnum: 7997,
    inst_ids: [1, 2, 3],
    geo_hashes: [10, 20, 30],
    tubi_segments: [...],
    world_matrices: [...]
}
```

## 清理逻辑对比

### 旧版（500+ 行）
```rust
// 16 个并发任务 × 7 个表 × 复杂级联
stream::iter(chunks).map(|chunk| {
    tokio::spawn(async move {
        // 查询 inst_relate -> geo_relate -> inst_geo
        // 删除 inst_geo
        // 删除 geo_relate
        // 删除 inst_relate
        // 删除 inst_relate_bool
        // 删除 neg_relate
        // 删除 tubi_relate
    })
}).buffer_unordered(16)
```

### 新版（15 行）
```rust
pub async fn pre_cleanup_for_regen_surreal(seed_refnos: &[RefnoEnum]) -> Result<()> {
    let all_refnos = collect_descendant_filter_ids_with_self(
        seed_refnos, &[], None, true
    ).await?;

    let refno_ids = all_refnos.iter()
        .map(|r| format!("refno_relations:{}", r.0))
        .collect::<Vec<_>>()
        .join(",");

    // 单条 DELETE 完成！
    let sql = format!("DELETE FROM refno_relations WHERE id IN [{}];", refno_ids);
    model_primary_db().query(&sql).await?;

    Ok(())
}
```

## 优势

### ✅ 零迁移成本
- 继续使用 SurrealDB
- 不需要引入 SQLite
- 不需要数据迁移

### ✅ 极致简化
- 代码：500+ 行 → 15 行（**97% 简化**）
- 表数量：7 → 1
- SQL 语句：复杂级联 → 单条 DELETE

### ✅ 性能提升
- 无级联查询开销
- 单次网络往返
- 预计 **10-15x** 提升

### ✅ 维护简单
- 无外键约束
- 无级联逻辑
- 结构清晰

## 使用示例

### 集成到 run_regen_model

```rust
// src/cli_modes.rs::run_regen_model()

use crate::fast_model::gen_model::pdms_inst_surreal::pre_cleanup_for_regen_surreal;

pub async fn run_regen_model(
    config: &ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<GenModelResult> {
    let target_refnos = collect_regen_target_refnos(config).await?;

    // 新版清理（15 行）
    pre_cleanup_for_regen_surreal(&target_refnos).await?;

    let result = gen_all_geos_data(target_refnos, &db_option_override, None, None).await?;
    Ok(result)
}
```

### 保存数据

```rust
use crate::fast_model::gen_model::pdms_inst_surreal::{RefnoRelations, save_refno_relations_surreal};

// 聚合每个 refno 的关联数据
let mut relations_map: HashMap<RefnoEnum, RefnoRelations> = HashMap::new();

for inst in instances {
    let rel = relations_map.entry(inst.refno).or_default();
    rel.refno = inst.refno.0;
    rel.dbnum = dbnum;
    rel.inst_ids.push(inst.inst_id);
    rel.geo_hashes.push(inst.geo_hash);
}

// 批量保存
let relations: Vec<_> = relations_map.into_values().collect();
save_refno_relations_surreal(&relations).await?;
```

## 性能预估

| 操作 | 旧版 | 新版 | 提升 |
|------|------|------|------|
| 清理 1000 refnos | 5000ms | **400ms** | 12.5x |
| 清理 10000 refnos | 45000ms | **3000ms** | 15x |
| 插入 10000 条 | 3000ms | **300ms** | 10x |

## 实施步骤

1. ✅ 创建 `pdms_inst_surreal.rs` 模块
2. 在 `run_regen_model()` 中切换到新版清理
3. 修改保存逻辑，聚合数据后批量写入
4. 测试验证

**预计工期**: 1-2 天
