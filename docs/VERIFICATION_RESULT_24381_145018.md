# 24381_145018 模型回写数据库验证结果

## 关键发现

### 1. dbnum 映射问题
- **refno**: `24381_145018`
- **实际 dbnum**: `7997`（不是 24381）
- **日志证据**: `📌 使用 dbnum: 7997 (从 refno 24381_145018 解析)`

### 2. 缓存写入情况

根据日志分析：

#### dbnum=7997 的缓存 flush
```
[cache_flush] dbnum=7997 batches=75/857 inst_info=10818 inst_geos=3057 inst_tubi=57 neg=88 ngmr=0 bool=0
save_instance_data_optimize flushing inst_relate from inst_info_map: 18
[cache_flush] dbnum=7997 tubi_relate 写入 57 条
```

**说明**：
- inst_info: 10818 条
- inst_geos: 3057 条
- inst_tubi: 57 条（包含 24381_145018 的 11 条）
- inst_relate: 18 条（但验证时显示的 refno 列表中没有 24381_145018）

#### 误将 ref0 当 dbnum（dbnum=24381）的缓存 flush（应为空）
```
[cache_flush] dbnum=24381 batches=0/4 inst_info=0 inst_geos=0 inst_tubi=0 neg=0 ngmr=0 bool=0
```

**说明**：dbnum=24381 的缓存为空，没有数据被 flush。
这通常意味着你把 `ref0=24381` 误当成了 `dbnum`；对该数据集而言，真实 `dbnum` 为 `7997`。

### 3. 数据生成情况

日志显示 24381_145018 的数据已生成：
- `[BRAN_TUBI] 分支处理完成: refno=24381_145018, 生成 tubi 段数=11`
- `[cache] insert_from_shape 调用: dbnum=7997, inst_cnt=11, inst_info=0, inst_geos=0, inst_tubi=11`

### 4. 数据库查询结果

**当前状态**：
- inst_relate 总数：1 条（但不是 24381_145018）
- tubi_relate 总数：1 条（但不是 24381_145018）
- pe:`24381_145018` 记录：不存在

## 问题分析

### 可能的原因

1. **数据未写入 SurrealDB**
   - 虽然日志显示 flush 了 57 条 tubi_relate，但查询结果只有 1 条
   - 可能是事务冲突导致写入失败（日志中有大量 Transaction conflict）

2. **pe key 格式问题**
   - 查询时使用的 pe key 格式可能不正确
   - 需要确认 SurrealDB 中实际存储的格式

3. **数据被覆盖或删除**
   - `replace_exist=true` 可能导致旧数据被删除但新数据未写入
   - 事务冲突可能导致部分数据丢失

### 日志中的事务冲突

```
⚠️ [DEBUG] Transaction conflict, retry 1/8 after 50ms
⚠️ [DEBUG] Transaction conflict, retry 2/8 after 100ms
```

大量事务冲突可能导致：
- 部分数据写入失败
- 数据不一致
- 需要检查事务重试是否成功

## 验证建议

### 1. 检查事务提交状态
```sql
-- 查询最近的写入记录
SELECT in, out, dt FROM inst_relate ORDER BY dt DESC LIMIT 10;
```

### 2. 检查 pe 表记录
```sql
-- 尝试不同的 pe key 格式
SELECT * FROM pe:`24381_145018`;
SELECT * FROM pe WHERE id = pe:`24381_145018`;
SELECT * FROM pe WHERE refno = 24381_145018;
```

### 3. 检查 tubi_relate
```sql
-- 查询所有 tubi_relate
SELECT id, in, out FROM tubi_relate LIMIT 20;

-- 查询包含 24381_145018 的记录
SELECT * FROM tubi_relate WHERE id[0] = pe:`24381_145018` OR in = pe:`24381_145018` OR out = pe:`24381_145018`;
```

### 4. 重新运行生成和同步
```bash
# 重新生成并同步
cargo run --bin aios-database -- --debug-model 24381/145018 --regen-model --export-obj --sync-to-db
```

## 下一步行动

1. ✅ 确认 dbnum 映射（24381_145018 → dbnum=7997）
2. ⚠️ 检查事务冲突是否导致写入失败
3. ⚠️ 验证 pe key 格式是否正确
4. ⚠️ 检查数据是否真的写入到 SurrealDB

## 参考

- [验证指南](./VERIFY_SYNC_TO_DB_GUIDE.md)
- [plant-surrealdb skill](../../.claude/skills/plant-surrealdb/SKILL.md)
