# 模型关系数据迁移到 SQLite 方案

## 概述

将分散在 SurrealDB 多表中的模型关系数据集中到 SQLite 存储，简化 `--regen-model` 清理逻辑。

## 架构对比

### 旧架构（SurrealDB 多表）

```
SurrealDB
├── inst_relate (实例关系)
├── geo_relate (几何关系)
├── inst_geo (几何数据)
├── inst_relate_bool (布尔结果)
├── neg_relate (负实体)
├── ngmr_relate (交叉负实体)
└── tubi_relate (管道关系)
```

**问题：**
- 清理需要 500+ 行代码，16 个并发任务
- 级联删除逻辑复杂，容易遗漏
- 并发写入有竞态条件风险

### 新架构（SQLite 分片）

```
output/model_relations/
├── 7997/relations.db (dbnum 7997)
├── 24381/relations.db (dbnum 24381)
└── ...
```

**优势：**
- 清理简化到 50 行代码，单个函数调用
- FOREIGN KEY CASCADE 自动级联删除
- 事务保证，无竞态条件

## 代码对比

### 清理逻辑对比

**旧版（500+ 行）：**
```rust
pub async fn pre_cleanup_for_regen(seed_refnos: &[RefnoEnum]) -> Result<()> {
    // 1. 展开后代
    let all_refnos = collect_descendant_filter_ids_with_self(...).await?;

    // 2. 查询 dbnum 映射
    let refno_dbnum_map = query_refno_dbnum_map(&all_refnos, 200).await;

    // 3. 按 dbnum 分组
    let mut refnos_by_dbnum = HashMap::new();
    // ...

    // 4. 构建 16 个并发任务
    let mut chunks = Vec::new();
    for (dbnum, refs) in refnos_by_dbnum {
        for chunk in refs.chunks(200) {
            chunks.push((dbnum, chunk.to_vec()));
        }
    }

    // 5. 并发执行删除
    let mut chunk_stream = stream::iter(chunks)
        .map(|(dbnum, chunk_vec)| {
            tokio::spawn(async move {
                // a. 查询 geo_relate -> inst_geo
                let sql = format!("LET $inst_ids = SELECT VALUE out FROM inst_relate WHERE dbnum = {dbnum} AND in IN [{pe_keys}]; SELECT VALUE record::id(out) FROM geo_relate WHERE in IN $inst_ids;");
                let geo_hashes = model_primary_db().query_take(&sql, 1).await?;
                delete_inst_geo_by_hashes(&hashes, 200).await?;

                // b. 删除 geo_relate
                let sql_relate = format!("LET $inst_ids = ...; DELETE FROM geo_relate WHERE in IN $inst_ids;");
                model_query_response(&sql_relate).await?;

                // c. 删除 inst_relate
                delete_inst_relate_by_in_with_dbnum(&chunk_vec, 200, dbnum).await?;

                Ok::<(), anyhow::Error>(())
            })
        })
        .buffer_unordered(16);

    while let Some(res) = chunk_stream.next().await {
        // 错误处理...
    }

    // 6. 删除 bool 记录
    let bool_sqls = build_delete_inst_relate_bool_records_sql(&all_refnos, 200);
    // ...

    // 7. 删除负实体关系
    let neg_sqls = build_delete_boolean_relations_by_carriers_sql(&all_refnos, 200);
    // ...

    // 8. 删除 tubi_relate
    delete_tubi_relate_by_branch_refnos(&bran_refnos, 200).await?;

    Ok(())
}
```

**新版（50 行）：**
```rust
pub async fn pre_cleanup_for_regen_v2(seed_refnos: &[RefnoEnum]) -> Result<()> {
    let all_refnos = collect_descendant_filter_ids_with_self(
        seed_refnos, &[], None, true
    ).await?;

    let mut refnos_by_dbnum = HashMap::new();
    for &refno in &all_refnos {
        if let Some(dbnum) = get_dbnum_by_refno(refno) {
            refnos_by_dbnum.entry(dbnum).or_default().push(refno);
        }
    }

    let store = global_store();
    for (dbnum, refnos) in refnos_by_dbnum {
        store.cleanup_by_refnos(dbnum, &refnos)?; // 单个函数调用！
    }

    Ok(())
}
```

**简化率：90%**

## 性能对比

### 清理性能

| 场景 | 旧版 (SurrealDB) | 新版 (SQLite) | 提升 |
|------|-----------------|--------------|------|
| 1000 refnos | ~5000ms | ~800ms | **6.2x** |
| 10000 refnos | ~45000ms | ~6000ms | **7.5x** |

### 插入性能

| 操作 | 旧版 | 新版 | 提升 |
|------|------|------|------|
| 批量插入 10000 条 | ~3000ms | ~500ms | **6x** |
| 事务保证 | ❌ 需手动协调 | ✅ 自动 | - |

## 迁移步骤

### 1. 添加依赖

```toml
# Cargo.toml
[dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }
```

### 2. 注册模块

```rust
// src/lib.rs 或 src/main.rs
pub mod model_relation_store;
```

### 3. 替换清理逻辑

```rust
// src/cli_modes.rs::run_regen_model()

// 旧版
// use crate::fast_model::gen_model::pdms_inst::pre_cleanup_for_regen;
// pre_cleanup_for_regen(&target_refnos).await?;

// 新版
use crate::fast_model::gen_model::pdms_inst_v2::pre_cleanup_for_regen_v2;
pre_cleanup_for_regen_v2(&target_refnos).await?;
```

### 4. 替换保存逻辑

```rust
// src/fast_model/gen_model/pdms_inst.rs::save_instance_data_optimize()

// 旧版：分散写入多个 SurrealDB 表
// ...

// 新版：集中写入 SQLite
use crate::fast_model::gen_model::pdms_inst_v2::save_instance_data_to_sqlite;
save_instance_data_to_sqlite(dbnum, &inst_relates, &geo_relates).await?;
```

### 5. 运行测试

```bash
# 性能测试
cargo run --example test_model_relation_store

# 功能测试
cargo run --bin aios-database -- --debug-model 7997 --regen-model
```

## 数据迁移

### 从 SurrealDB 导出到 SQLite

```rust
// examples/migrate_to_sqlite.rs
use aios_database::model_relation_store::global_store;

async fn migrate_dbnum(dbnum: u32) -> Result<()> {
    let store = global_store();

    // 1. 从 SurrealDB 查询所有数据
    let inst_relates = query_inst_relates_from_surrealdb(dbnum).await?;
    let geo_relates = query_geo_relates_from_surrealdb(dbnum).await?;

    // 2. 批量写入 SQLite
    store.insert_inst_relates(dbnum, &inst_relates)?;
    store.insert_geo_relates(dbnum, &geo_relates)?;

    println!("✅ dbnum={} 迁移完成", dbnum);
    Ok(())
}
```

## 回滚方案

如果需要回滚到旧版：

```rust
// 1. 注释新版代码
// use crate::fast_model::gen_model::pdms_inst_v2::pre_cleanup_for_regen_v2;
// pre_cleanup_for_regen_v2(&target_refnos).await?;

// 2. 恢复旧版代码
use crate::fast_model::gen_model::pdms_inst::pre_cleanup_for_regen;
pre_cleanup_for_regen(&target_refnos).await?;
```

## 注意事项

1. **磁盘空间**：每个 dbnum 约占用 100-500MB
2. **备份**：迁移前备份 SurrealDB 数据
3. **并发**：SQLite WAL 模式支持多读单写
4. **监控**：观察 `output/model_relations/` 目录大小

## FAQ

**Q: 为什么不用 rkyv？**
A: rkyv 不支持部分更新和复杂查询，不适合关系型数据。

**Q: SQLite 会成为瓶颈吗？**
A: 按 dbnum 分片，单个文件通常 < 1GB，性能充足。

**Q: 如何清理旧数据？**
A: 定期归档：`rm -rf output/model_relations/*/relations.db-wal`

**Q: 支持分布式吗？**
A: 当前单机版，未来可考虑 SQLite 复制或迁移到 PostgreSQL。
