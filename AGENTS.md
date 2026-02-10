# AGENTS.md

本文件为 AI 编码助手提供项目级指令，适用于 OpenAI Codex、GitHub Copilot 等 agent。

## 构建与调试

- 调试时使用 debug 模式，不要编译 release
- 不要使用 `cargo clean`

## ⚠️ 核心概念：ref0 ≠ dbnum

这是本项目最容易出错的地方，**必须严格遵守**。

### 三个概念

| 术语 | 含义 | 示例 |
|------|------|------|
| **refno** | PDMS 元素唯一标识，格式 `ref0/sesno` | `24381/145018` |
| **ref0** | refno 的第一部分，PDMS 内部引用编号 | `24381` |
| **dbnum** | 数据库编号，标识一个 PDMS 物理数据库文件 | `7997` |

### 关键规则

**ref0 和 dbnum 是完全不同的值。** 不能互相替代。

映射关系存储在 `output/<project>/scene_tree/db_meta_info.json`：

```json
{
  "ref0_to_dbnum": {
    "24381": 7997,
    "25688": 1112,
    "9304": 1112
  }
}
```

一个 dbnum 可以对应多个 ref0（如 1112 对应 25688 和 9304）。

### Rust 代码中的正确做法

```rust
// ✅ 唯一正确方式：通过 db_meta 映射
let dbnum = db_meta().get_dbnum_by_refno(refno);

// ✅ 映射缺失时报错
let dbnum = db_meta().get_dbnum_by_refno(refno)
    .ok_or_else(|| anyhow!("缺少 ref0->dbnum 映射: refno={}", refno))?;

// ✅ 映射缺失时跳过（适用于过滤场景）
if let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) {
    // 使用 dbnum
}
```

### 绝对禁止的写法

```rust
// ❌ 直接取 ref0 当 dbnum
let dbnum = refno.refno().get_0();

// ❌ 字符串分割取第一段当 dbnum
let dbnum = refno.to_string().split_once('_').unwrap().0;

// ❌ 映射失败时回退用 ref0
let dbnum = get_dbnum(ref0).unwrap_or(ref0);

// ❌ 映射失败时兜底用 get_0()
let dbnum = db_meta().get_dbnum_by_refno(refno)
    .unwrap_or_else(|| refno.refno().get_0());
```

### dbnum 的使用场景

- 缓存分桶（instance_cache、transform_cache 按 dbnum 分区）
- 文件目录命名（Parquet 输出 `{dbnum}/instance.parquet`）
- TreeIndex 加载（`tree_index_{dbnum}.json`）
- CLI `--dbnum` 参数

### ref0 的使用场景

- PE 表 ID 构造（`pe:'24381_145018'`，其中 24381 是 ref0）
- refno 内部编码（RefnoEnum 的 get_0() 返回的是 ref0）
- 仅用于标识，不用于分桶或目录

### 映射缺失时的处理

映射缺失说明 `db_meta_info.json` 不完整，应该：
1. 先生成/更新 `output/<project>/scene_tree/db_meta_info.json`
2. 或重建 scene_tree 元数据

**禁止**用 ref0 值作为 dbnum 的兜底。
