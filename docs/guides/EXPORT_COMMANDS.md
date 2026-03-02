# 导出命令使用说明

## 核心语义（2026-02-28）

- 默认 `--export-*` 是**纯导出**：只查询 DB/缓存，不触发模型生成。
- `--regen-model` 单独使用：只执行模型重建，不导出。
- `--regen-model + --export-*`：先重建，再导出；若 `defer_db_write=true` 产生 `.surql`，会自动导入并做后处理后再导出。

## 常用命令

```bash
# 1) 纯导出（默认）
cargo run --bin aios-database -- --export-obj --dbnum 7997
cargo run --bin aios-database -- --export-glb-refnos 24381_145018
cargo run --bin aios-database -- --export-gltf

# 2) 仅重建（不导出）
cargo run --bin aios-database -- --regen-model --dbnum 7997

# 3) 重建后导出
cargo run --bin aios-database -- --regen-model --export-obj --debug-model 24381_145018
```

## 调试与截图

```bash
# 调试导出（自动截图）
cargo run --bin aios-database -- --debug-model 24381_145018 --export-obj

# 调试 + 手动截图目录
cargo run --bin aios-database -- --debug-model 24381_145018 --capture output/screenshots
```

## 常见参数

```bash
--export-obj-output <PATH>          # 指定输出路径
--export-filter-nouns EQUI,PIPE     # 类型过滤
--export-include-descendants=false   # 是否包含子孙节点
--use-surrealdb                      # 强制实例数据走 SurrealDB 路径
--basic-materials                    # GLB/glTF 使用基础材质
--verbose                            # 详细日志
```
