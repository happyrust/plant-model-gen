# 任务进展轨迹

## 当前任务：分析 archetypes 目录
- [x] 列出 `archetypes` 目录内容
- [x] 在代码中搜索 `archetypes` 关键字
- [x] 分析 `export_instanced_bundle.rs` 源码
- [x] 总结 `archetypes` 的设计意图
- [x] 撰写分析报告并回复用户

## 分析结果总结
`archetypes` 目录的存在主要是为了支持 **WebGL 实例化渲染 (GPU Instancing)**。
- **核心文件**：GLB 模型（几何原型）。
- **工作流**：导出器识别出场景中的重复部件 -> 提取为原型存入 `archetypes` -> 其他位置仅存储 `matrix` 引用该原型。
- **优势**：显著减少显存占用和 Draw Call 次数，支持大规模工业场景流畅渲染。
