# 惰性计算 World Transform 方案

实现在模型生成时，当 `pe_transform.world_trans` 缓存不存在时，通过查找最近有缓存的祖先节点，逐层计算得到目标节点的世界变换。

## 问题分析

### 当前流程
1. `get_world_transform(refno)` → `get_world_mat4(refno, false)` → `get_transform_mat4(refno, false)`
2. `get_transform_mat4` 先查 `pe_transform` 缓存，无则调用 `compute_world_from_parent`
3. **问题**：`compute_world_from_parent` 只查直接父节点的 `world_trans`，若父节点也无缓存则返回 `None`

### 调用链
```
prim_model.rs:117  → aios_core::get_world_transform(refno)
  ↓
spatial.rs:348     → get_world_mat4(refno, false)
  ↓
transform/mod.rs:187 → get_transform_mat4(refno, false)
  ↓
transform/mod.rs:213 → compute_world_from_parent(refno, local_mat)
  ↓
transform/mod.rs:241 → query_pe_transform(parent_refno)  ← 只查父节点，无递归
```

## 现有可用资源

| 函数 | 位置 | 功能 |
|------|------|------|
| `query_ancestor_refnos(refno)` | `rs_surreal/query.rs:137` | 返回祖先链 `Vec<RefnoEnum>` |
| `find_nearest_cached_ancestor(refno)` | `rs_surreal/query.rs:154` | 查找最近有 `world_trans` 的祖先（**已实现但未被使用**） |
| `get_local_mat4(refno)` | `transform/mod.rs:158` | 计算单节点局部变换 |

## 实现方案

### 核心改造：`compute_world_from_parent`

```rust
async fn compute_world_from_parent(
    refno: RefnoEnum,
    local_mat: Option<DMat4>,
) -> anyhow::Result<Option<DMat4>> {
    // 1. 查找最近有 world_trans 缓存的祖先
    let cached_ancestor = find_nearest_cached_ancestor(refno).await?;
    
    // 2. 获取祖先链（从根到当前节点）
    let ancestors = query_ancestor_refnos(refno).await?;
    
    // 3. 确定计算起点
    let (start_idx, start_world) = match cached_ancestor {
        Some((idx, ancestor_refno)) => {
            let cache = query_pe_transform(ancestor_refno).await?;
            let world = cache
                .and_then(|c| c.world)
                .map(|t| t.to_matrix().as_dmat4());
            (idx + 1, world.unwrap_or(DMat4::IDENTITY))
        }
        None => (0, DMat4::IDENTITY),  // 从根节点开始
    };
    
    // 4. 从起点逐层累乘 local_mat
    let mut world_mat = start_world;
    for ancestor in ancestors.iter().skip(start_idx) {
        let local = get_local_mat4(*ancestor).await?.unwrap_or(DMat4::IDENTITY);
        world_mat = world_mat * local;
    }
    
    // 5. 最后乘上当前节点的 local_mat
    Ok(Some(match local_mat {
        Some(local) => world_mat * local,
        None => world_mat,
    }))
}
```

## 任务清单

1. **修改 `compute_world_from_parent`** (`transform/mod.rs:231-251`)
   - 集成 `find_nearest_cached_ancestor` 查找缓存起点
   - 使用 `query_ancestor_refnos` 获取祖先链
   - 从缓存起点逐层调用 `get_local_mat4` 累乘

2. **优化缓存写入**
   - 计算过程中沿途节点的 `world_mat` 也写入 `pe_transform`（可选，提升后续查询命中率）

3. **测试验证**
   - 运行 `--debug-model 17496_106028` 验证无缓存时仍能正确计算
   - 检查性能影响（祖先链查询+多次 `get_local_mat4`）

## 风险与注意事项

- **性能**：深层节点（祖先链长）计算开销较大，建议保持 `--refresh-transform` 预热机制
- **缓存一致性**：惰性计算结果需写入缓存，避免重复计算
- **循环引用**：`get_effective_parent_att` 已有 `MAX_DEPTH=10` 保护，同理应用于此
