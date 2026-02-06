# 验证 sync-to-db 数据写入指南

基于 plant-surrealdb skill 的最佳实践，本文档说明如何验证 `--sync-to-db` 后 SurrealDB 中的数据。

## 验证策略

### 1. Record ID 格式确认

根据 plant-surrealdb skill，pe key 格式为：
- **格式**: `pe:⟨ref0_ref1⟩`（使用反引号包裹）
- **示例**: `pe:`24381_145018``
- **注意**: ref0 不是 dbnum，需要通过 `DbMetaManager` 映射获取 dbnum

### 2. 查询语法最佳实践

#### 计数查询
```sql
-- 推荐：使用 SELECT VALUE count() 直接返回标量
SELECT VALUE count() FROM inst_relate WHERE in = pe:`24381_145018`;

-- 或使用 AS 别名（代码中常用）
SELECT count() AS cnt FROM inst_relate WHERE in IN [pe:`24381_145018`, pe:`24381_145019`];
```

#### 批量查询（使用数组）
```sql
-- 使用 IN 子句查询多个 refno
SELECT VALUE count() FROM inst_relate WHERE in IN [pe:`24381_145018`, pe:`24381_145019`, ...];

-- 或使用 FROM 数组（性能更好）
SELECT * FROM [pe:`24381_145018`, pe:`24381_145019`];
```

#### tubi_relate ID Range 查询（推荐）
```sql
-- 使用 ID Range 查询 tubi_relate（性能最优）
SELECT VALUE count() FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..];
```

### 3. 验证检查清单

#### 基础验证
1. **inst_relate**: 检查 PE 元素到实例的关系
   ```sql
   SELECT VALUE count() FROM inst_relate WHERE in = pe:`24381_145018`;
   ```

2. **geo_relate**: 检查实例到几何的关系
   ```sql
   SELECT VALUE count() FROM geo_relate;
   ```

3. **tubi_relate**: 检查管道直段关系（使用 ID Range）
   ```sql
   SELECT VALUE count() FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..];
   ```

4. **neg_relate / ngmr_relate**: 检查负实体关系
   ```sql
   SELECT VALUE count() FROM neg_relate;
   SELECT VALUE count() FROM ngmr_relate;
   ```

#### 详细验证
1. **检查实际写入的记录**
   ```sql
   SELECT in, out, owner FROM inst_relate WHERE in = pe:`24381_145018` LIMIT 10;
   ```

2. **检查关系完整性**
   ```sql
   -- 检查 inst_relate -> inst_info -> geo_relate 链路
   SELECT 
       ir.in as pe_refno,
       ir.out as inst_info_id,
       gr.out as inst_geo_id
   FROM inst_relate ir
   WHERE ir.in = pe:`24381_145018`
   LIMIT 5;
   ```

3. **检查 tubi_relate 详细信息**
   ```sql
   SELECT 
       id[0] as bran_refno,
       id[1] as index,
       in as leave_refno,
       out as arrive_refno,
       geo as geo_id
   FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..]
   LIMIT 10;
   ```

### 4. Rust 验证代码示例

```rust
use aios_core::{SUL_DB, SurrealQueryExt, init_surreal, RefnoEnum};
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_surreal().await?;
    
    let refno = RefnoEnum::from_str("24381/145018")?;
    let pe_key = refno.to_pe_key(); // 生成 pe:`24381_145018`
    
    // 查询 inst_relate 数量
    let sql = format!("SELECT VALUE count() FROM inst_relate WHERE in = {};", pe_key);
    let counts: Vec<u64> = SUL_DB.query_take(&sql, 0).await?;
    let cnt = counts.first().copied().unwrap_or(0);
    
    println!("inst_relate count for {}: {}", pe_key, cnt);
    
    // 查询 tubi_relate（使用 ID Range）
    let sql_tubi = format!(
        "SELECT VALUE count() FROM tubi_relate:[{}, 0]..[{}, ..];",
        pe_key, pe_key
    );
    let tubi_counts: Vec<u64> = SUL_DB.query_take(&sql_tubi, 0).await?;
    let tubi_cnt = tubi_counts.first().copied().unwrap_or(0);
    
    println!("tubi_relate count for {}: {}", pe_key, tubi_cnt);
    
    Ok(())
}
```

### 5. PowerShell 验证脚本要点

1. **正确处理反引号转义**
   ```powershell
   # 使用单引号避免 PowerShell 转义问题
   $peKey = 'pe:`24381_145018`'
   ```

2. **解析 SurrealDB HTTP API 响应**
   ```powershell
   # SELECT VALUE count() 返回格式: [N] 或 [[N]]
   function Get-CountFromResult($r) {
       if (-not $r.result) { return $null }
       $first = $r.result[0]
       if ($first -is [Array] -and $first.Count -gt 0) { return $first[0] }
       return $first
   }
   ```

### 6. 常见问题排查

#### 问题：查询返回 0 但日志显示已写入
**可能原因**：
1. 事务未提交（检查 TransactionBatcher 的 finish 调用）
2. pe key 格式错误（确认使用反引号格式）
3. 命名空间/数据库不匹配（确认 surreal-ns 和 surreal-db 配置）

**排查步骤**：
```sql
-- 1. 检查 pe 表是否存在该记录
SELECT * FROM pe:`24381_145018`;

-- 2. 检查所有 inst_relate 记录（不限定条件）
SELECT VALUE count() FROM inst_relate;

-- 3. 检查最近的写入记录
SELECT in, out, dt FROM inst_relate ORDER BY dt DESC LIMIT 10;
```

#### 问题：tubi_relate 查询返回 0
**可能原因**：
1. ID Range 语法错误
2. bran_refno 不正确

**排查步骤**：
```sql
-- 1. 检查 BRAN 是否存在
SELECT * FROM pe:`24381_145018` WHERE noun = 'BRAN';

-- 2. 尝试直接查询（不使用 Range）
SELECT * FROM tubi_relate WHERE id[0] = pe:`24381_145018` LIMIT 5;
```

### 7. 性能优化建议

1. **批量查询优于循环查询**
   ```sql
   -- ✅ 推荐：一次查询多个 refno
   SELECT VALUE count() FROM inst_relate WHERE in IN [pe:`24381_145018`, pe:`24381_145019`];
   
   -- ❌ 不推荐：循环查询
   -- for each refno: SELECT VALUE count() FROM inst_relate WHERE in = pe:`...`;
   ```

2. **使用 ID Range 查询 tubi_relate**
   ```sql
   -- ✅ 推荐：ID Range（性能最优）
   SELECT * FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..];
   
   -- ❌ 不推荐：WHERE 条件（全表扫描）
   SELECT * FROM tubi_relate WHERE id[0] = pe:`24381_145018`;
   ```

3. **使用 SELECT VALUE 只取值**
   ```sql
   -- ✅ 推荐：只返回标量值
   SELECT VALUE count() FROM inst_relate;
   
   -- ⚠️ 可用但性能略差：返回对象
   SELECT count() AS cnt FROM inst_relate;
   ```

## 参考

- [plant-surrealdb skill](../../.claude/skills/plant-surrealdb/SKILL.md)
- [数据库查询总结](../../.claude/skills/plant-surrealdb/references/数据库查询总结.md)
- [数据库架构](../../.claude/skills/plant-surrealdb/references/数据库架构.md)
- [常用查询方法](../../.claude/skills/plant-surrealdb/references/常用查询方法.md)
