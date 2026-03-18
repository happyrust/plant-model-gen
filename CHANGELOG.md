# Changelog

## Unreleased

### Added

- 导出模型查询：增加对 `Neg` (负实体) 的导出支持，并放宽图元节点输出条件确保元素正常输出。
- 新增工具脚本：`scripts/export_dbnum_parquet_file.sh` 和 `test_ida_mcp.py` 辅助调测。

### Changed

- 调整环境默认配置：`db_options/DbOption.toml` 等修改默认数据库连接模式为 `file`。

- 房间树 API 支持展开 `COMP_GROUP`（构件分组）并返回该组下所有构件列表，同时支持查询节点下的有效子节点数量。
- 房间树 API 支持查询单个构件 refno 的祖先路径（正确关联至相应的 `COMP_GROUP` 和 `ROOM` 节点）。
- 提取网格生成状态管理至独立的 `mesh_state.rs`，优化流式模型生成与流形布尔运算的依赖调度。
- 提供 `query_component_insts` 示例以支持构件实例的快捷查询调测。

### Changed

- 优化 DB 模式和 File 模式下的 mesh 依赖判断逻辑，确保流式生成与强制生成的正确触发。
- 重构模型生成编排层（`orchestrator`）与相关查询接口，提升查询和并发图组生成的稳定性。

### Fixed

- 修复 `--regen-model` 未清理旧 `tubi_relate` 导致 BRAN/HANG 导出时混入历史局部坐标直段的问题。
- 修复三通元件库表达式 `TWICE PARAM 3` 被错误求值为 `0`，导致 `24381_145582` 一类 `TEE` 丢失支管几何的问题。
- 补齐 `--debug-model --export-obj` 的 PNG 预览输出，`CaptureConfig` 不再只是打印“自动启用截图”但没有实际文件产出。
