# 单模型导出流程（默认纯导出，`--regen-model` 前置生成）

本文说明如何在 `gen_model-dev` 中导出单个模型（OBJ/GLB/GLTF/SVG），以及在需要时触发“先生成后导出”流程。

## 1. 语义约定

1. 默认导出命令只读数据库/缓存，不触发生成（等价默认 `--skip-gen`）。
2. `--regen-model` 单独使用时，只执行生成，不执行导出。
3. `--regen-model` 与任一 `--export-*` 同时使用时，执行：
   - 生成模型
   - 若 `defer_db_write=true`，自动执行 import + 后处理（reconcile/boolean/aabb）
   - 执行导出

## 2. 常用命令

### 2.1 纯导出（不生成）

```bash
cargo run --bin aios-database -- --debug-model 17496_106028 --export-obj
```

### 2.2 先生成再导出（推荐用于验证修复）

```bash
cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj
```

### 2.3 仅生成（不导出）

```bash
cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model
```

## 3. 输出位置

1. 主 OBJ 输出：
   - `output/<project>/<name_or_refno>.obj`
2. defer SQL（当启用 `defer_db_write=true`）：
   - `output/<project>/deferred_sql/<timestamp>_<dbnum|all>.surql`
3. Debug 布尔中间产物（`--debug-model` 常见）：
   - `test_output/debug_<refno>_pos.obj`
   - `test_output/debug_<refno>_neg.obj`
   - `test_output/debug_<refno>_result.obj`

## 4. 验收检查清单

1. 日志中出现 `✅ 模型重新生成完成`（仅当使用 `--regen-model`）。
2. 若 defer 生效，日志中出现 `开始自动导入并后处理` 与 `✅ import-sql 全部完成`。
3. 日志中出现 `✅ 导出成功`。
4. 目标 OBJ 文件存在且大小大于 0。

## 5. 常见问题

1. 控制台日志较少：
   - 程序可能启用了 stdio 重定向，日志写入 `logs/*.log`。
2. 出现单个元素生成错误但总体成功：
   - 例如 `CAT引用不存在`，需区分“局部元素失败”与“目标 refno 导出成功”。
3. 关于 `dbnum`：
   - 严禁将 `ref0` 当作 `dbnum`。
   - 必须通过 `db_meta().get_dbnum_by_refno(refno)` 推导。
