# tubi_relate 查询指南

本文档描述了 `tubi_relate` 表的查询方法和最佳实践。

## 表结构回顾

`tubi_relate` 存储 BRAN/HANG 下的管道直段信息，使用**复合 ID** 格式：

```
tubi_relate:[pe:⟨bran_refno⟩, index]
```

例如：`tubi_relate:[pe:⟨21491_10000⟩, 0]`

其中 `bran_refno` 为 BRAN/HANG 的 refno，其 Record ID 形态为 `ref0_ref1`（下划线连接）；文中出现的 `leave_refno` / `arrive_refno` / `system_refno` 亦同此约定。

**字段说明**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id[0]` | pe record | BRAN/HANG 的 pe_key |
| `id[1]` | int | 管道段索引 |
| `in` | pe record | 起点构件 (leave_refno) |
| `out` | pe record | 终点构件 (arrive_refno) |
| `geo` | inst_geo record | 几何数据引用 |
| `aabb` | object | 包围盒 `{ d: Aabb }` |
| `world_trans` | object | 世界变换 `{ d: Transform }` |
| `bore_size` | string | 管径尺寸 |
| `bad` | bool | 是否为异常段 |
| `system` | pe record | 所属系统 |
| `dt` | datetime | 时间戳 |

---

## ID Range 查询（推荐）

使用 SurrealDB 的 **ID Range** 查询是最高效的方式：

### 基本语法

```sql
SELECT * FROM tubi_relate:[pe:⟨bran_refno⟩, 0]..[pe:⟨bran_refno⟩, ..]
```

- `[pe:⟨xxx⟩, 0]`: 起始 ID（index = 0）
- `[pe:⟨xxx⟩, ..]`: 结束 ID（index = 无穷大）

### Rust 实现

```rust
pub async fn query_tubi_insts_by_brans(
    bran_refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<TubiInstQuery>> {
    if bran_refnos.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_results = Vec::new();
    for bran_refno in bran_refnos {
        let pe_key = bran_refno.to_pe_key();
        let sql = format!(
            r#"
            SELECT
                id[0] as refno,
                in as leave,
                id[0].old_pe as old_refno,
                id[0].owner.noun as generic,
                aabb.d as world_aabb,
                world_trans.d as world_trans,
                record::id(geo) as geo_hash,
                id[0].dt as date
            FROM tubi_relate:[{}, 0]..[{}, ..]
            WHERE aabb.d != NONE
            "#,
            pe_key, pe_key
        );
        let mut results: Vec<TubiInstQuery> = SUL_DB
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        all_results.append(&mut results);
    }
    Ok(all_results)
}
```

### 字段提取说明

| 表达式 | 说明 |
|--------|------|
| `id[0]` | 取复合 ID 的第一个元素（BRAN pe record） |
| `id[0].old_pe` | 访问该 PE 的 old_pe 字段 |
| `id[0].owner.noun` | 访问该 PE 的 owner 的 noun 字段 |
| `in` / `out` | 直接返回 PE record，RefnoEnum 会自动反序列化 |
| `aabb.d` | 取包围盒的数据部分 |
| `world_trans.d` | 取变换矩阵的数据部分 |
| `record::id(geo)` | 提取 geo record 的 ID 字符串 |

---

## 与旧查询方式对比

### ❌ 旧方式（不推荐）

```sql
SELECT
    record::id(id)[0] as refno,
    in as leave,
    record::id(id)[0].old_pe as old_refno,
    record::id(id)[0].owner.noun as generic,
    ...
FROM tubi_relate 
WHERE record::id(id)[0] inside [pe:⟨xxx⟩, pe:⟨yyy⟩]
AND aabb.d != NONE
```

**问题**：
- 使用 `record::id(id)` 额外解析开销
- `WHERE` 条件需要遍历全表

### ✅ 新方式（推荐）

```sql
SELECT
    id[0] as refno,
    in as leave,
    id[0].old_pe as old_refno,
    id[0].owner.noun as generic,
    ...
FROM tubi_relate:[pe:⟨bran_refno⟩, 0]..[pe:⟨bran_refno⟩, ..]
WHERE aabb.d != NONE
```

**优势**：
- 使用 ID Range 直接索引查询
- 直接使用 `id[0]` 访问复合 ID
- 性能更优

---

## 创建 tubi_relate 记录

### 写入语法

```sql
RELATE pe:⟨leave_refno⟩->tubi_relate:[pe:⟨bran_refno⟩, index]->pe:⟨arrive_refno⟩
SET
    geo = inst_geo:⟨geo_hash⟩,
    aabb = aabb:⟨aabb_hash⟩,
    world_trans = trans:⟨trans_hash⟩,
    bore_size = 'DN100',
    bad = false,
    system = pe:⟨system_refno⟩,
    dt = fn::ses_date(pe:⟨leave_refno⟩);
```

### Rust 实现示例

```rust
let sql = format!(
    "RELATE {}->tubi_relate:[{}, {}]->{} \
    SET geo=inst_geo:⟨{}⟩, aabb=aabb:⟨{}⟩, world_trans=trans:⟨{}⟩, \
    bore_size={}, bad={}, system={}, dt=fn::ses_date({});",
    leave_refno.to_pe_key(),    // in
    branch_refno.to_pe_key(),   // id[0]
    index,                       // id[1]
    arrive_refno.to_pe_key(),   // out
    geo_hash,
    aabb_hash,
    trans_hash,
    bore_size,
    bad_flag,
    owner_refno.to_pe_key(),
    leave_refno.to_pe_key(),
);
```

---

## 相关类型定义

### TubiInstQuery（查询结果）

```rust
#[derive(Serialize, Deserialize, Debug, SurrealValue)]
pub struct TubiInstQuery {
    pub refno: RefnoEnum,           // BRAN/HANG refno (from id[0])
    pub leave: RefnoEnum,           // 起点构件 (from in)
    pub old_refno: Option<RefnoEnum>,
    pub generic: Option<String>,
    pub world_aabb: PlantAabb,
    pub world_trans: PlantTransform,
    pub geo_hash: String,
    pub date: Option<surrealdb::types::Datetime>,
}
```

### TubiRelate（表结构）

```rust
pub struct TubiRelate {
    pub id: String,
    pub input: RefnoEnum,           // in
    pub out: RefnoEnum,             // out
    pub geo: Option<String>,
    pub start_pt: Option<Vec3>,
    pub end_pt: Option<Vec3>,
    pub system: Option<RefnoEnum>,
    pub dt: Option<NaiveDateTime>,
}
```

---

## 代码位置

- **查询方法**: `src/rs_surreal/inst.rs` → `query_tubi_insts_by_brans()`
- **结构体定义**: `src/rs_surreal/inst_structs.rs` → `TubiRelate`
- **写入逻辑**: `gen-model-fork/src/fast_model/cata_model.rs`
- **导出查询**: `gen-model-fork/src/fast_model/export_model/export_common.rs`
