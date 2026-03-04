# SurrealDB 自定义函数参考

本文档列出了 aios-core 中定义的所有 SurrealDB 自定义函数。

---

## 层级查询函数

### fn::ancestor - 祖先节点查询

获取指定节点的所有祖先节点。

```sql
-- 语法
fn::ancestor(pe_record) -> array<pe_record>

-- 示例
RETURN fn::ancestor(pe:⟨21491_10000⟩);

-- 获取祖先的 refno 列表
RETURN fn::ancestor(pe:⟨21491_10000⟩).refno;

-- 筛选特定类型的祖先
RETURN fn::ancestor(pe:⟨21491_10000⟩)[WHERE noun = 'ZONE'];
```

**Rust 调用**:
```rust
let sql = format!("return fn::ancestor({}).refno;", pe_key);
let ancestors: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await?;
```

---

### fn::collect_children - 子节点收集

获取指定节点的直接子节点。

```sql
-- 语法
fn::collect_children(pe_record, types_array?) -> array<pe_record>

-- 示例：获取所有子节点
SELECT * FROM fn::collect_children(pe:⟨21491_10000⟩, none);

-- 示例：筛选特定类型
SELECT * FROM fn::collect_children(pe:⟨21491_10000⟩, ['EQUI', 'PIPE']);
```

**Rust 调用**:
```rust
let sql = format!(
    "SELECT {} FROM fn::collect_children({}, {})",
    select_expr, pe_key, types_array
);
```

---

### fn::collect_descendant_ids_by_types - 子孙 ID 收集

递归收集指定类型的所有子孙节点 ID。

```sql
-- 语法
fn::collect_descendant_ids_by_types(pe_record, types_array, include_self?, range_str) -> array<pe_record>

-- 示例：收集所有 EQUI 和 PIPE 类型的子孙（无限深度）
RETURN fn::collect_descendant_ids_by_types(pe:⟨21491_10000⟩, ['EQUI', 'PIPE'], none, "..");

-- 示例：限制深度 1-5 层
RETURN fn::collect_descendant_ids_by_types(pe:⟨21491_10000⟩, ['EQUI'], none, "1..5");

-- 示例：固定 3 层
RETURN fn::collect_descendant_ids_by_types(pe:⟨21491_10000⟩, [], none, "3");
```

**range_str 参数**:
- `".."` - 无限深度（默认）
- `"1..5"` - 1 到 5 层
- `"3"` - 固定 3 层

---

### fn::collect_descendants_filter_spre - 过滤 SPRE/CATR 子孙

收集子孙节点，并过滤具有 SPRE/CATR 属性的节点。

```sql
-- 语法
fn::collect_descendants_filter_spre(pe_record, types_array, filter_inst, include_self?, range_str) -> array<pe_record>

-- 示例：过滤有 SPRE 的节点
RETURN fn::collect_descendants_filter_spre(pe:⟨21491_10000⟩, [], true, none, "..");
```

**filter_inst 参数**:
- `true` - 同时过滤掉有 inst_relate 或 tubi_relate 的节点
- `false` - 仅过滤 SPRE/CATR

---

### fn::collect_descendants_filter_inst - 过滤已生成实例子孙

收集子孙节点，过滤掉已有几何实例的节点。

```sql
-- 语法
fn::collect_descendants_filter_inst(pe_record, types_array, filter, include_self, include_branch) -> array<pe_record>

-- 示例
RETURN fn::collect_descendants_filter_inst(pe:⟨21491_10000⟩, ['EQUI'], true, true, false);
```

**参数说明**:
- `filter`: 是否过滤已有实例的节点
- `include_self`: 是否包含自身
- `include_branch`: 是否包含分支节点

---

### fn::collect_descendant_ids_has_inst - 查询有实例的子孙

收集有 inst_relate 关系的子孙节点。

```sql
-- 语法
fn::collect_descendant_ids_has_inst(pe_record, types_array, include_self?) -> array<{id, has_inst}>

-- 示例
RETURN fn::collect_descendant_ids_has_inst(pe:⟨21491_10000⟩, [], true)[? has_inst];
```

---

## 几何类型查询函数

### fn::visible_geo_descendants - 可见几何子孙

获取所有可见几何类型的子孙节点。

```sql
-- 语法
fn::visible_geo_descendants(pe_record, include_self, range_str) -> array<pe_record>

-- 示例
SELECT VALUE fn::visible_geo_descendants(pe:⟨21491_10000⟩, false, "..");
```

**可见几何类型**:
BOX, CYLI, SLCY, CONE, DISH, CTOR, RTOR, PYRA, SNOU, POHE, POLYHE, EXTR, REVO, 
FLOOR, PANE, ELCONN, CMPF, WALL, GWALL, SJOI, FITT, PFIT, FIXING, PJOI, GENSEC, 
RNODE, PRTELE, GPART, SCREED, PALJ, CABLE, BATT, CMFI, SCOJ, SEVE, SBFI, STWALL, 
SCTN, NOZZ

---

### fn::negative_geo_descendants - 负实体几何子孙

获取所有负实体几何类型的子孙节点。

```sql
-- 语法
fn::negative_geo_descendants(pe_record, include_self, range_str) -> array<pe_record>

-- 示例
SELECT VALUE fn::negative_geo_descendants(pe:⟨21491_10000⟩, false, "..");
```

**负实体类型**:
NBOX, NCYL, NLCY, NSBO, NCON, NSNO, NPYR, NDIS, NXTR, NCTO, NRTO, NREV, NSCY, 
NSCO, NLSN, NSSP, NSCT, NSRT, NSDS, NSSL, NLPY, NSEX, NSRE

---

## 名称与属性函数

### fn::default_name - 默认名称

获取节点的默认显示名称。

```sql
-- 语法
fn::default_name(pe_record) -> string

-- 示例
RETURN fn::default_name(pe:⟨21491_10000⟩);
```

---

### fn::ses_date - 会话日期

获取节点的会话日期。

```sql
-- 语法
fn::ses_date(pe_record) -> datetime

-- 示例
SELECT fn::ses_date(id) as date FROM pe:⟨21491_10000⟩;
```

---

## 布尔运算函数

### fn::query_negative_entities - 查询负实体

查询与指定节点关联的所有负实体。

```sql
-- 语法
fn::query_negative_entities(pe_record) -> array<pe_record>

-- 示例
RETURN fn::query_negative_entities(pe:⟨21491_10000⟩);
```

**Rust 调用**:
```rust
let sql = format!("RETURN fn::query_negative_entities({})", pe_key);
let negs: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await?;
```

---

## 拓扑连接函数

### fn::prev_connect_pe - 上一个连接节点

获取管道拓扑中的上一个连接节点。

```sql
-- 语法
fn::prev_connect_pe(pe_record) -> pe_record?

-- 示例
RETURN fn::prev_connect_pe(pe:⟨21491_10000⟩);
```

---

### fn::next_connect_pe - 下一个连接节点

获取管道拓扑中的下一个连接节点。

```sql
-- 语法
fn::next_connect_pe(pe_record) -> pe_record?

-- 示例
RETURN fn::next_connect_pe(pe:⟨21491_10000⟩);
```

---

### fn::has_leave_tubi - 是否有离开管道

检查节点是否作为管道的离开点。

```sql
-- 语法
fn::has_leave_tubi(pe_record) -> bool

-- 示例
RETURN fn::has_leave_tubi(pe:⟨21491_10000⟩);
```

---

### fn::has_arrive_tubi - 是否有到达管道

检查节点是否作为管道的到达点。

```sql
-- 语法
fn::has_arrive_tubi(pe_record) -> bool

-- 示例
RETURN fn::has_arrive_tubi(pe:⟨21491_10000⟩);
```

---

## 数据备份函数

### fn::backup_data - 数据备份

将指定节点的数据备份到历史表。

```sql
-- 语法
fn::backup_data(pe_keys_array, is_deleted, sesno)

-- 示例
fn::backup_data([pe:⟨21491_10000⟩, pe:⟨21491_10001⟩], false, 100);
```

---

### fn::backup_owner_relate - 备份 Owner 关系

备份指定节点的 owner 关系。

```sql
-- 语法
fn::backup_owner_relate(pe_keys_array, flag)

-- 示例
fn::backup_owner_relate([pe:⟨21491_10000⟩], true);
```

---

## 函数定义位置

函数定义脚本位于：
- `src/rs_surreal/schemas/functions/db.surql`
- 加载脚本：`src/rs_surreal/schemas/functions/load_spine_functions.sh`

**运行时加载**:
```rust
use aios_core::rs_surreal::define_common_functions;

// 从指定目录加载函数
define_common_functions(Some("resource/surreal")).await?;

// 或从配置读取目录
define_common_functions(None).await?;
```

---

## 事件定义

### update_dbnum_event - 数据库统计更新事件

自动维护 `dbnum_info_table` 的统计数据。

```sql
DEFINE EVENT OVERWRITE update_dbnum_event ON pe 
WHEN $event = "CREATE" OR $event = "UPDATE" OR $event = "DELETE" 
THEN {
    -- 自动更新 dbnum_info_table 的 count, sesno, max_ref1, updated_at
};
```

**Rust 定义**:
```rust
use aios_core::rs_surreal::define_dbnum_event;

define_dbnum_event().await?;
```
