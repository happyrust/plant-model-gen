# 24381_145018 模型回写数据库验证总结

## 验证结果

### ✅ 已确认的事实

1. **dbnum 映射正确**
   - refno `24381_145018` 的实际 dbnum 是 `7997`（不是 24381）
   - 日志：`📌 使用 dbnum: 7997 (从 refno 24381_145018 解析)`

2. **数据已生成**
   - tubi_relate: 11 条（日志确认）
   - 日志：`[BRAN_TUBI] 分支处理完成: refno=24381_145018, 生成 tubi 段数=11`
   - 缓存写入：`[cache] insert_from_shape 调用: dbnum=7997, inst_cnt=11, inst_tubi=11`

3. **缓存 flush 执行**
   - dbnum=7997: `inst_info=10818, inst_geos=3057, inst_tubi=57`
   - 日志：`[cache_flush] dbnum=7997 tubi_relate 写入 57 条`

### ⚠️ 发现的问题

1. **数据库查询结果异常**
   - inst_relate 总数：1 条（但查询不到 24381_145018 的记录）
   - tubi_relate 总数：1 条（但查询不到 24381_145018 的记录）
   - pe:`24381_145018` 记录：不存在

2. **事务冲突**
   - 日志中有大量 `Transaction conflict, retry` 警告
   - 可能导致部分数据写入失败

3. **数据不一致**
   - 日志显示 flush 了 57 条 tubi_relate，但数据库只有 1 条
   - 可能是事务冲突导致写入失败

## 基于 plant-surrealdb skill 的验证方法

### 正确的查询方式

1. **使用正确的 pe key 格式**
   ```sql
   -- pe key 格式：pe:`ref0_ref1`
   SELECT * FROM pe:`24381_145018`;
   ```

2. **使用 ID Range 查询 tubi_relate**
   ```sql
   SELECT * FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..];
   ```

3. **通过 dbnum 查询**
   ```sql
   SELECT VALUE count() FROM inst_relate WHERE in.dbnum = 7997;
   ```

### 验证工具

- **Rust 程序**: `cargo run --example verify_sync_to_db_24381_145018`
- **PowerShell 脚本**: `scripts/verify_sync_to_db_24381_145018.ps1`
- **详细验证**: `scripts/verify_detailed_24381_145018.ps1`

## 建议的下一步

1. **检查事务提交状态**
   - 查看是否有事务失败的错误日志
   - 确认 TransactionBatcher 的 finish 是否成功

2. **重新运行生成和同步**
   ```bash
   cargo run --bin aios-database -- --debug-model 24381/145018 --regen-model --export-obj --sync-to-db
   ```

3. **检查缓存文件**
   - 查看 `output/AvevaMarineSample/instance_cache/` 目录
   - 确认缓存中是否有 24381_145018 的数据

4. **直接查询 SurrealDB**
   - 使用 SurrealDB Web UI 或命令行工具
   - 查询实际的记录格式和内容

## 参考文档

- [验证指南](./VERIFY_SYNC_TO_DB_GUIDE.md)
- [验证结果详情](./VERIFICATION_RESULT_24381_145018.md)
- [plant-surrealdb skill](../../.claude/skills/plant-surrealdb/SKILL.md)
