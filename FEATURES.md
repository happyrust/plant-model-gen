# Feature 配置说明

本文档说明了如何使用项目中的可选 features,以减少编译时间和二进制文件大小。

## 已优化的 Features

### 1. DuckDB 支持 (`duckdb-feature`)

**说明**: DuckDB 是一个用于模型数据存储和导出的列式数据库。

**默认状态**: ❌ 已从默认 features 中移除

**何时启用**:

- 需要使用 DuckDB 进行模型数据导出
- 运行 `check_collision` binary

**启用方法**:

```bash
# 编译时启用
cargo build --features duckdb-feature

# 运行特定 binary
cargo run --bin check_collision --features duckdb-feature
```

---

### 2. MeiliSearch 支持 (`meilisearch`)

**说明**: MeiliSearch 是一个搜索引擎,用于 PDMS 元素的全文检索。

**默认状态**: ❌ 已设为可选依赖(在 `pdms_io` 项目中)

**何时启用**:

- 需要使用 MeiliSearch 进行元素搜索
- 运行搜索相关的测试程序

**启用方法**:

```bash
# 在 pdms_io 项目中启用
cd pdms-io
cargo build --features meilisearch

# 运行 MeiliSearch 测试程序
cargo run --bin test_meilisearch --features meilisearch
cargo run --bin test_meilisearch_simple --features meilisearch
cargo run --bin test_search_integration --features meilisearch
```

**注意**: 使用 MeiliSearch 功能前,需要先启动 MeiliSearch 服务器:

```bash
# 下载: https://github.com/meilisearch/meilisearch/releases
# 运行
./meilisearch --master-key=your-master-key

# 或使用 Docker
docker run -it --rm -p 7700:7700 getmeili/meilisearch:latest
```

---

## 默认 Features

当前默认启用的 features:

```toml
default = [
    "ws",
    "gen_model",
    "manifold",
    "project_hd",
    "surreal-save",
]
```

## 编译时间对比

**优化前** (包含 `duckdb-feature` 和 `meilisearch`):

- 首次编译: ~5-8 分钟
- 增量编译: ~1-2 分钟

**优化后** (不包含这两个 features):

- 首次编译: ~3-5 分钟
- 增量编译: ~30-60 秒

## 如何组合使用多个 Features

```bash
# 同时启用 duckdb 和其他 features
cargo build --features "duckdb-feature,gen_model"

# 在 gen_model-dev 中使用 pdms_io 的 meilisearch feature
# 需要在 gen_model-dev 的 Cargo.toml 中配置:
# pdms_io = { path = "../pdms-io", features = ["meilisearch"] }
```

## 常见问题

### Q: 我运行项目时遇到 "未找到模块 search" 错误?

A: 这是因为 `search` 模块需要 `meilisearch` feature。请使用 `--features meilisearch` 启用。

### Q: 为什么编译时会跳过某些 binary?

A: 某些 binary 需要特定的 features。例如:

- `check_collision` 需要 `duckdb-feature`
- `test_meilisearch*` 需要 `meilisearch` feature

### Q: 如何恢复到优化前的配置?

A: 在 `gen_model-dev/Cargo.toml` 的 `default` features 中添加回 `"duckdb-feature"` 即可。

---

## 更新日志

**2026-01-16**:

- ✅ 将 `duckdb` 从 default features 中移除
- ✅ 将 `meilisearch-sdk` 设为 optional 依赖
- ✅ 为相关 binary targets 添加 `required-features`
- ✅ 减少默认编译依赖,提升开发效率
