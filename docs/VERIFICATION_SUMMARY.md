# 验证 sync-to-db 数据写入 - 总结

## 基于 plant-surrealdb skill 的验证方案

### 核心要点

1. **Record ID 格式**
   - 格式：`pe:`ref0_ref1``（使用反引号）
   - 示例：`pe:`24381_145018``
   - 注意：ref0 不是 dbnum
   - ref0→dbnum 映射来源：`output/<project>/scene_tree/db_meta_info.json` 的 `ref0_to_dbnum`
     - 示例：`ref0=24381 -> dbnum=7997`
   - 约束：涉及 `dbnum` 的任何过滤/文件路径（如 `{dbnum}.tree`）**不得**直接使用 ref0，映射缺失应先补齐元数据，而非回退 ref0。

2. **查询语法**
   - 计数：`SELECT VALUE count() FROM table WHERE ...`
   - 批量：`WHERE in IN [pe:`...`, pe:`...`]`
   - tubi_relate：使用 ID Range `tubi_relate:[pe:`...`, 0]..[pe:`...`, ..]`

3. **验证工具**
   - Rust 程序：`cargo run --example verify_sync_to_db_24381_145018`
   - PowerShell 脚本：`scripts/verify_sync_to_db_24381_145018.ps1`

### 验证步骤

#### 步骤 1：基础计数验证
```sql
-- inst_relate
SELECT VALUE count() FROM inst_relate WHERE in = pe:`24381_145018`;

-- geo_relate
SELECT VALUE count() FROM geo_relate;

-- tubi_relate（使用 ID Range）
SELECT VALUE count() FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..];
```

#### 步骤 2：详细记录验证
```sql
-- 检查实际写入的记录
SELECT in, out, owner FROM inst_relate WHERE in = pe:`24381_145018` LIMIT 5;

-- 检查 tubi_relate 详细信息
SELECT id[0] as bran, id[1] as idx, in as leave, out as arrive 
FROM tubi_relate:[pe:`24381_145018`, 0]..[pe:`24381_145018`, ..] LIMIT 5;
```

#### 步骤 3：pe 表验证
```sql
-- 确认 pe 记录存在
SELECT id, noun, name FROM pe:`24381_145018`;
```

### 常见问题

**Q: 查询返回 0 但日志显示已写入？**
- 检查事务是否提交
- 确认 pe key 格式（使用反引号）
- 验证命名空间/数据库配置

**Q: tubi_relate 查询失败？**
- 确认使用 ID Range 语法
- 检查 bran_refno 是否正确

### 参考文档

- [验证指南](./VERIFY_SYNC_TO_DB_GUIDE.md)
- [plant-surrealdb skill](../../.claude/skills/plant-surrealdb/SKILL.md)
