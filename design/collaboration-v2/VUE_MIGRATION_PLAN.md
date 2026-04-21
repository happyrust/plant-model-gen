# 异地协同 v2 · 原型能力迁入 Vue 生产代码

> 制定日期：2026-04-21
>
> 目标：将 Hi-Fi 原型中最高价值的交互增强移植到 Vue 生产组件，每步 vue-tsc + build 验证。

## 3 个迁移项

### V-P1 · CollaborationWorkbenchView 快捷键 + 抽屉 Escape

**改动**：
1. onMounted 注册全局 keydown：1-4 切换 Tab、? 弹帮助、D 切主题
2. CollaborationEnvDrawer / CollaborationSiteDrawer 添加 Escape 关闭
3. Tab 切换添加 CSS transition

**目标文件**：
- `ui/admin/src/views/CollaborationWorkbenchView.vue`
- `ui/admin/src/components/collaboration/CollaborationEnvDrawer.vue`
- `ui/admin/src/components/collaboration/CollaborationSiteDrawer.vue`

**风险**：低 · 纯追加，不改现有逻辑

### V-P2 · GroupLogsPanel CSV 导出 + 搜索高亮

**改动**：
1. 添加「导出 CSV」按钮，Blob 下载当前筛选后的日志
2. 日志列表中搜索关键词 `<mark>` 高亮
3. 日志卡片点击展开完整错误信息

**目标文件**：
- `ui/admin/src/components/collaboration/GroupLogsPanel.vue`

**风险**：低 · 扩展模板和方法

### V-P3 · CollaborationTopologyPanel 全屏模式

**改动**：
1. 添加全屏按钮，点击后面板 `position:fixed` 全屏
2. Escape 退出全屏
3. 节点 hover tooltip

**目标文件**：
- `ui/admin/src/components/collaboration/CollaborationTopologyPanel.vue`

**风险**：低 · 扩展现有 SVG 面板

## 验证

- 每步 `npx vue-tsc -b` + `npm run build`
- 浏览器手动验收
