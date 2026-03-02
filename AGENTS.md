# AGENTS.md

本文件为 AI 编码助手提供项目级指令，适用于 OpenAI Codex、GitHub Copilot 等 agent。

## 构建与调试

- 调试时使用 debug 模式，不要编译 release
- 不要使用 `cargo clean`
- 执行 `cargo check` 时，使用独立的 target 目录以避免与正在运行的 build 产生文件锁冲突：
  ```powershell
  $env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
  ```

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

## Cursor Cloud specific instructions

### 环境概述

本项目是一个 Rust nightly 项目（PDMS/E3D 3D 模型处理平台），主要包含：
- `aios-database` CLI：解析 PDMS 数据库、生成 3D 模型
- `web_server` 二进制：Axum HTTP 服务，端口 8080，提供 Web UI 和 REST API

### 依赖结构（重要）

项目依赖三个本地 sibling 仓库（通过 `Cargo.toml` 中的 `[patch]` 块引用）：
- `/rs-core` → `aios_core` crate（从 `happyrust/rs-core` dev-3.1 分支克隆）
- `/pdms-io-fork` → `pdms_io` + `parse_pdms_db` crate（`pdms_io` 为 stub 实现，`parse_pdms_db` 从 `happyrust/aios-parse-pdms` dev-3.1 克隆）

多个 GitHub 上的 fork 仓库（`indextree`、`rstar`、`calamine`、`cavalier_contours`、`id_tree`、`rust-ploop-processor`）已被删除。环境通过 git URL 重定向到 `/opt/cargo-mirrors/` 下的本地镜像来解决：
- `~/.gitconfig` 中配置了 `url.*.insteadOf` 规则
- 本地镜像基于上游仓库 + 所需特性修改（如 indextree 的 rkyv 支持、rstar 的 serde 特性）
- `ploop-rs` 为 stub 实现

### 构建命令

```bash
# 必须设置 C++ 编译器（clang 默认无法找到 libstdc++ 头文件）
export CXX=g++-13 CC=gcc-13

# 检查 lib（默认 features）
CARGO_TARGET_DIR="target-check" cargo check --lib

# 构建 web_server（需要额外启用 mqtt feature 以解决现有 cfg gate 缺失问题）
cargo build --bin web_server --no-default-features --features "ws,sqlite-index,surreal-save,web_server,mqtt,gen_model,manifold,kv-rocksdb"

# 运行测试
CARGO_TARGET_DIR="target-check" cargo test --lib
```

### 已知问题

1. **web_server feature 编译需要 mqtt**：`sync_control_handlers.rs` 和 `remote_runtime.rs` 引用了 `#[cfg(feature = "mqtt")]` 门控的函数但自身未加门控，构建 web_server 需同时启用 `mqtt` feature。
2. **缺失源文件**：`src/bin/meili_reindex.rs` 和 `src/fast_model/export_model/duckdb_exporter.rs` 不存在，`cargo fmt` 会报 warning。
3. **SurrealDB 未运行**：大部分需要数据库的测试和功能会报 connection refused，这是预期的。Web 服务器可正常启动并提供 UI。
4. **PDMS 数据不可用**：`pdms_io` 为 stub 实现，涉及真实 PDMS 文件 I/O 的功能会返回错误。

### 运行 web_server

```bash
export CXX=g++-13 CC=gcc-13
./target/debug/web_server
# 访问 http://localhost:8080
```

服务器会报数据库连接失败的错误日志，但 Web UI 和 API 端点可正常使用。
