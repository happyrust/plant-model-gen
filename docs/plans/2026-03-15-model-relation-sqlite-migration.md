# 模型关系数据迁移到 SQLite 实施计划

**创建时间**: 2026-03-15
**状态**: 待实施
**优先级**: 高

## 目标

将分散在 SurrealDB 的模型关系数据集中到 SQLite，简化 `--regen-model` 清理逻辑。

## 收益

- ✅ 清理代码从 500+ 行降到 50 行（**90% 简化**）
- ✅ 清理性能提升 **6-7 倍**
- ✅ 消除并发竞态条件
- ✅ 自动级联删除，避免遗漏

## 实施阶段

### Phase 1: 基础设施（1-2 天）

**任务：**
1. 创建 `src/model_relation_store.rs` 核心模块 ✅
2. 添加 `rusqlite` 依赖
3. 实现表结构初始化
4. 实现基础 CRUD 操作

**验收标准：**
```bash
cargo run --example test_model_relation_store
# 输出: 插入/查询/删除性能指标
```

### Phase 2: 清理逻辑替换（2-3 天）

**任务：**
1. 创建 `pdms_inst_v2.rs` 新版清理函数 ✅
2. 在 `run_regen_model()` 中切换到新版
3. 保留旧版作为回滚备份
4. 添加功能开关 `ENABLE_SQLITE_STORE`

**代码修改：**
```rust
// src/cli_modes.rs::run_regen_model()
let use_sqlite = std::env::var("ENABLE_SQLITE_STORE").is_ok();

if use_sqlite {
    pre_cleanup_for_regen_v2(&target_refnos).await?;
} else {
    pre_cleanup_for_regen(&target_refnos).await?; // 旧版回滚
}
```

**验收标准：**
```bash
ENABLE_SQLITE_STORE=1 cargo run --bin aios-database -- --debug-model 7997 --regen-model
# 验证清理成功且耗时降低
```

### Phase 3: 保存逻辑迁移（3-4 天）

**任务：**
1. 修改 `save_instance_data_optimize()` 支持双写
2. 同时写入 SurrealDB 和 SQLite（过渡期）
3. 验证数据一致性
4. 逐步切换到 SQLite 单写

**代码修改：**
```rust
pub async fn save_instance_data_optimize(...) -> Result<()> {
    // 双写模式（过渡期）
    save_to_surrealdb(...).await?;
    save_to_sqlite(...).await?;

    // 验证一致性
    verify_consistency()?;
    Ok(())
}
```

**验收标准：**
- 双写模式运行 1 周无错误
- 数据一致性检查通过率 100%

### Phase 4: 数据迁移（1-2 天）

**任务：**
1. 编写迁移脚本 `examples/migrate_to_sqlite.rs`
2. 按 dbnum 批量迁移历史数据
3. 验证迁移完整性

**迁移脚本：**
```rust
async fn migrate_all() -> Result<()> {
    let dbnums = query_all_dbnums().await?;
    for dbnum in dbnums {
        println!("迁移 dbnum={}...", dbnum);
        migrate_dbnum(dbnum).await?;
    }
    Ok(())
}
```

**验收标准：**
```bash
cargo run --example migrate_to_sqlite
# 输出: 所有 dbnum 迁移成功
```

### Phase 5: 切换与清理（1 天）

**任务：**
1. 移除 `ENABLE_SQLITE_STORE` 开关，默认启用
2. 删除旧版 SurrealDB 清理代码
3. 更新文档和注释

**验收标准：**
- 所有测试通过
- 性能指标达标
- 文档更新完成

## 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 数据迁移失败 | 高 | 保留 SurrealDB 数据，支持回滚 |
| 性能不达预期 | 中 | 保留旧版代码，可快速切换 |
| 磁盘空间不足 | 低 | 监控目录大小，定期清理 WAL |

## 回滚计划

```bash
# 1. 恢复环境变量
unset ENABLE_SQLITE_STORE

# 2. 重启服务
systemctl restart aios-database

# 3. 验证旧版工作正常
cargo run --bin aios-database -- --debug-model 7997 --regen-model
```

## 监控指标

- 清理耗时: < 1000ms (1000 refnos)
- 插入耗时: < 500ms (10000 条)
- 磁盘占用: < 500MB/dbnum
- 错误率: < 0.1%

## 时间线

```
Week 1: Phase 1-2 (基础设施 + 清理逻辑)
Week 2: Phase 3 (保存逻辑迁移 + 双写验证)
Week 3: Phase 4-5 (数据迁移 + 切换)
```

**预计总工期**: 3 周
