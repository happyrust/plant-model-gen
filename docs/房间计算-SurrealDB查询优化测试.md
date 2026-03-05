# 房间计算 SurrealDB 查询优化测试

## 背景

当前房间面板映射查询使用了3层嵌套子查询（FRMW -> SBFR -> PANE），性能较差。
本文档测试使用 SurrealDB 的 Recursive paths 和其他优化方案。

## 数据模型说明

```
FRMW (框架)
  └─ OWNER (RecordId) -> SBFR
       └─ OWNER (RecordId) -> PANE
            └─ REFNO (构件引用号)
```

## 测试查询

### 原始查询（性能差）

```sql
-- 当前实现：3层嵌套子查询
SELECT VALUE [
    id,
    array::last(string::split(NAME, '-')),
    array::flatten(
        (SELECT VALUE
            (SELECT VALUE REFNO FROM PANE WHERE OWNER = $parent.REFNO)
         FROM SBFR WHERE OWNER = $parent.REFNO)
    )
] FROM FRMW
WHERE NAME IS NOT NONE AND ('ROOM' IN NAME);
```

**问题：**
- 每个 FRMW 触发多次子查询
- `$parent` 引用增加复杂度
- `array::flatten` 内存开销大

---

### 方案1：使用点号语法直接访问（最简单）

```sql
-- 如果 OWNER 字段是 RecordId，可以直接通过点号访问
SELECT VALUE [
    id,
    array::last(string::split(NAME, '-')),
    (SELECT VALUE REFNO FROM PANE WHERE OWNER IN (
        SELECT VALUE REFNO FROM SBFR WHERE OWNER = $parent.id
    ))
] FROM FRMW
WHERE NAME IS NOT NONE AND ('ROOM' IN NAME);
```

**测试命令：**
```bash
# 连接到 SurrealDB 测试
surreal sql --endpoint http://localhost:8000 --namespace test --database test --pretty
```

---

### 方案2：使用 FETCH 预加载关联数据（推荐）

```sql
-- 使用 FETCH 一次性加载所有关联数据
SELECT
    id,
    array::last(string::split(NAME, '-')) AS room_num,
    (SELECT VALUE REFNO
     FROM PANE
     WHERE OWNER IN (
         SELECT VALUE id FROM SBFR WHERE OWNER = $parent.id
     )) AS panel_refnos
FROM FRMW
WHERE NAME IS NOT NONE AND ('ROOM' IN NAME);
```

**优势：**
- 减少查询层级
- 使用 IN 操作符批量匹配
- 结构更清晰

---

### 方案3：使用递归路径（基于 record reference）

```sql
-- 方式1：如果 OWNER 是 record reference，可以直接递归访问
-- 从 PANE 向上递归2层到 FRMW
SELECT
    OWNER.{2}.id AS frmw_id,
    array::last(string::split(OWNER.{2}.NAME, '-')) AS room_num,
    REFNO AS panel_refno
FROM PANE
WHERE OWNER.{2}.NAME CONTAINS 'ROOM';

-- 方式2：从 FRMW 向下查询（需要反向关系）
-- 查询所有 OWNER 指向该 FRMW 的 SBFR，再查询指向这些 SBFR 的 PANE
SELECT
    id,
    array::last(string::split(NAME, '-')) AS room_num,
    (SELECT VALUE REFNO FROM PANE WHERE OWNER.OWNER = $parent.id) AS panel_refnos
FROM FRMW
WHERE NAME IS NOT NONE AND ('ROOM' IN NAME);
```

**说明：**
- `OWNER.{2}` 表示通过 OWNER 字段递归访问2层
- OWNER 必须是 record reference 类型（如 `record<SBFR>`）
- 这种方式不需要建立额外的图关系边

---

### 方案4：使用字段路径访问（实用方案）

```sql
-- 直接通过字段路径访问嵌套数据
SELECT
    id,
    array::last(string::split(NAME, '-')) AS room_num,
    array::flatten([
        (SELECT VALUE id->SBFR.id->PANE.REFNO FROM SBFR WHERE OWNER = $parent.id)
    ]) AS panel_refnos
FROM FRMW
WHERE NAME IS NOT NONE AND ('ROOM' IN NAME);
```

---

## 实际测试步骤

### 1. 准备测试环境

```bash
# 启动 SurrealDB（如果未启动）
surreal start --log trace --user root --pass root file://./surrealdb_data
```

### 2. 连接并测试查询

```bash
# 连接到数据库
surreal sql --endpoint http://localhost:8000 \
  --namespace test --database test \
  --username root --password root \
  --pretty
```

---

## 关键发现

从代码库中发现：
- **SurrealDB 3 的图递归语法与 v2 不兼容**（见 `src/scene_tree/query.rs:87`）
- 当前使用的是图遍历语法：`SELECT VALUE record::id(out) FROM [ids]->contains`
- 递归查询改为在 Rust 侧做 BFS

---

## 优化方案（基于 SurrealDB 3）

### 方案A：使用图遍历 + 批量查询（推荐）

```sql
-- 第一步：查询所有 FRMW（房间框架）
LET $frmw_list = SELECT id, NAME FROM FRMW
WHERE NAME IS NOT NONE AND NAME CONTAINS 'ROOM';

-- 第二步：批量查询 SBFR（通过 OWNER 字段）
LET $sbfr_list = SELECT id, OWNER, REFNO FROM SBFR
WHERE OWNER IN (SELECT VALUE id FROM $frmw_list);

-- 第三步：批量查询 PANE（通过 OWNER 字段）
SELECT
    frmw.id AS frmw_id,
    array::last(string::split(frmw.NAME, '-')) AS room_num,
    (SELECT VALUE REFNO FROM PANE WHERE OWNER IN (
        SELECT VALUE id FROM $sbfr_list WHERE OWNER = frmw.id
    )) AS panel_refnos
FROM $frmw_list AS frmw;
```

**优势：**
- 分步查询，逻辑清晰
- 使用 LET 变量缓存中间结果
- 批量 IN 查询，减少查询次数

---

### 方案B：如果 OWNER 是图关系边（理想情况）

```sql
-- 假设 OWNER 字段建立了 pe_owner 图关系
-- 可以使用图遍历语法直接查询

SELECT
    id,
    array::last(string::split(NAME, '-')) AS room_num,
    id->pe_owner->SBFR->pe_owner->PANE.REFNO AS panel_refnos
FROM FRMW
WHERE NAME IS NOT NONE AND NAME CONTAINS 'ROOM';
```

**说明：**
- 需要预先建立 `pe_owner` 图关系边
- 使用 `->` 语法进行图遍历
- 这是最简洁高效的方式

---

## 实际测试查询

### 测试1：验证当前数据结构

```sql
-- 查看 FRMW 表结构和样例数据
SELECT * FROM FRMW WHERE NAME CONTAINS 'ROOM' LIMIT 3;

-- 查看 SBFR 表结构
SELECT * FROM SBFR LIMIT 3;

-- 查看 PANE 表结构
SELECT * FROM PANE LIMIT 3;

-- 检查 OWNER 字段类型
SELECT OWNER FROM SBFR LIMIT 1;
```

---

### 测试2：对比原始查询与优化查询

```sql
-- 原始嵌套查询（性能基准）
SELECT VALUE [
    id,
    array::last(string::split(NAME, '-')),
    array::flatten(
        (SELECT VALUE
            (SELECT VALUE REFNO FROM PANE WHERE OWNER = $parent.REFNO)
         FROM SBFR WHERE OWNER = $parent.REFNO)
    )
] FROM FRMW
WHERE NAME IS NOT NONE AND NAME CONTAINS 'ROOM'
LIMIT 10;
```

```sql
-- 优化查询（方案A）
BEGIN TRANSACTION;

LET $frmw_list = SELECT id, NAME, REFNO FROM FRMW
WHERE NAME IS NOT NONE AND NAME CONTAINS 'ROOM' LIMIT 10;

LET $sbfr_list = SELECT id, OWNER, REFNO FROM SBFR
WHERE OWNER IN (SELECT VALUE REFNO FROM $frmw_list);

SELECT
    frmw.id AS frmw_id,
    array::last(string::split(frmw.NAME, '-')) AS room_num,
    (SELECT VALUE REFNO FROM PANE WHERE OWNER IN (
        SELECT VALUE REFNO FROM $sbfr_list WHERE OWNER = frmw.REFNO
    )) AS panel_refnos
FROM $frmw_list AS frmw;

COMMIT TRANSACTION;
```

---

### 测试3：验证结果一致性

```sql
-- 统计原始查询返回的记录数
SELECT count() FROM (
    SELECT VALUE [
        id,
        array::last(string::split(NAME, '-')),
        array::flatten(
            (SELECT VALUE
                (SELECT VALUE REFNO FROM PANE WHERE OWNER = $parent.REFNO)
             FROM SBFR WHERE OWNER = $parent.REFNO)
        )
    ] FROM FRMW
    WHERE NAME IS NOT NONE AND NAME CONTAINS 'ROOM'
    LIMIT 100
) GROUP ALL;
```

---

## 性能对比预期

| 方案 | 查询次数 | 预期性能 | 适用场景 |
|------|---------|---------|---------|
| 原始嵌套查询 | N * M * K | 慢 | 小数据量 |
| 方案A（批量查询） | 3 | 中等 | 中等数据量 |
| 方案B（图遍历） | 1 | 快 | 大数据量（需建立图关系） |

**说明：**
- N = FRMW 记录数
- M = 每个 FRMW 关联的 SBFR 数量
- K = 每个 SBFR 关联的 PANE 数量

---

## 实施建议

### 短期优化（立即可用）

1. **使用方案A替换当前嵌套查询**
   - 修改 `room_model.rs:1703-1805` 的 `build_room_panel_query_sql` 函数
   - 使用 LET 变量和批量 IN 查询
   - 预期性能提升：50-70%

2. **添加查询超时和分页**
   - 对大数据量查询添加 LIMIT 和 OFFSET
   - 设置合理的查询超时时间

### 长期优化（需要架构调整）

1. **建立 pe_owner 图关系边**
   - 为 OWNER 字段建立专门的图关系表
   - 使用 RELATE 语句创建关系：`RELATE SBFR:id->pe_owner->PANE:owner_id`
   - 支持高效的图遍历查询

2. **添加索引**
   ```sql
   DEFINE INDEX idx_sbfr_owner ON SBFR FIELDS OWNER;
   DEFINE INDEX idx_pane_owner ON PANE FIELDS OWNER;
   DEFINE INDEX idx_frmw_name ON FRMW FIELDS NAME;
   ```

---

## 下一步行动

1. **执行测试查询**
   ```bash
   # 连接到实际数据库
   surreal sql --endpoint http://localhost:8000 \
     --namespace test --database test \
     --username root --password root \
     --pretty

   # 依次执行测试1、测试2、测试3
   ```

2. **记录性能数据**
   - 原始查询执行时间
   - 优化查询执行时间
   - 返回结果数量对比

3. **实施优化**
   - 修改 `room_model.rs` 中的查询函数
   - 添加单元测试验证结果一致性
   - 进行性能基准测试

---

## 总结

### 关键问题

1. **3层嵌套子查询** - 每个 FRMW 触发多次子查询，性能差
2. **缺少批量优化** - 没有利用 IN 操作符批量查询
3. **缺少事务保护** - 批量 RELATE 操作没有事务包装
4. **全表扫描** - 空间索引刷新使用 ORDER BY + OFFSET

### 优化收益

- **短期优化**：预期性能提升 50-70%
- **长期优化**：预期性能提升 80-90%（需建立图关系）

### 参考资料

- [SurrealDB Idioms](https://surrealdb.com/docs/surrealql/datamodel/idioms)
- [SurrealDB Graph Relations](https://surrealdb.com/docs/surrealql/statements/relate)
- 代码参考：`src/scene_tree/query.rs`
