调试时，一定要使用debug模式，不要编译release.
不要使用 cargo clean.

## ⚠️ ref0 ≠ dbnum（严重易错点）

### 概念区分

- **refno**：PDMS 元素的唯一标识，格式为 `ref0/sesno`（如 `24381/145018`）
- **ref0**：refno 的第一部分（如 `24381`），是 PDMS 数据库内部的引用编号
- **dbnum**：数据库编号，标识一个 PDMS 物理数据库文件（如 `7997` 对应 `DB7997`）

**ref0 和 dbnum 是完全不同的概念。** 一个 dbnum 下可以有多个不同的 ref0，多个 ref0 也可以映射到同一个 dbnum。

### 映射关系

映射存储在 `output/<project>/scene_tree/db_meta_info.json` 的 `ref0_to_dbnum` 字段：

| ref0 | dbnum | 说明 |
|------|-------|------|
| 24381 | 7997 | ref0=24381 属于 DB7997 |
| 25688 | 1112 | ref0=25688 属于 DB1112 |
| 9304 | 1112 | ref0=9304 也属于 DB1112 |

### 正确用法

```rust
// ✅ 正确：通过 db_meta 映射
let dbnum = db_meta().get_dbnum_by_refno(refno);

// ✅ 正确：映射缺失时报错或跳过
let dbnum = db_meta().get_dbnum_by_refno(refno)
    .ok_or_else(|| anyhow!("缺少 ref0->dbnum 映射: refno={}", refno))?;
```

### 禁止的用法

```rust
// ❌ 错误：直接取 ref0 当 dbnum
let dbnum = refno.refno().get_0();

// ❌ 错误：字符串分割取第一段当 dbnum
let dbnum = refno.to_string().split_once('_').unwrap().0;

// ❌ 错误：映射失败时回退用 ref0
let dbnum = db_meta().get_dbnum_by_refno(refno).unwrap_or(ref0);

// ❌ 错误：映射失败时兜底用 get_0()
let dbnum = db_meta().get_dbnum_by_refno(refno)
    .unwrap_or_else(|| refno.refno().get_0());
```

### 约束

- CLI `--dbnum` 参数必须传 dbnum（如 7997），**不能**传 ref0（如 24381）
- PE 表 ID 格式 `pe:'24381_145018'` 中的 `24381` 是 ref0，不是 dbnum
- 映射缺失时应先补齐 `db_meta_info.json`（或重建 scene_tree 元数据），**禁止**回退把 ref0 当 dbnum
- dbnum 用于：缓存分桶、文件目录命名、TreeIndex 加载、Parquet 输出分区
- ref0 仅用于：PE 表 ID 构造、refno 内部编码
