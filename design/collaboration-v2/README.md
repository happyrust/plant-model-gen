# 异地协同 UI v2 · Hi-Fi 原型 (5 轮 24 项完善 · 1866 行)

> 由 [huashu-design skill](https://github.com/alchaincyf/huashu-design) 驱动的 Junior Designer 工作流产出。
>
> 对应旧版页面：`ui/admin/src/views/CollaborationWorkbenchView.vue` + 8 个 `components/collaboration/*.vue`
>
> 对应后端文档：`docs/development/admin/异地协同功能架构文档.md`

## 原型交互能力清单（2026-04-21 完成）

| 能力 | 轮次 | 描述 |
|---|---|---|
| 搜索过滤 | R1 | 左栏协同组按 name/location/mqtt 过滤 |
| 拓扑诊断历史 | R1 | 详情面板展示最近 3 次诊断 + 关联活跃任务 |
| 图表 tooltip | R1 | 洞察柱状图 hover 显示日期/成功/失败 |
| 空状态 UI | R1 | 日志筛选无结果时的 SVG 占位 |
| Tab 动画 | R1 | 切换时 fadeIn 过渡 |
| 流向动画粒子 | R2 | ok/warn 流向沿路径运动小圆点 |
| 实时模拟引擎 | R2 | Running 任务 3s 自增 + 完成 toast + 实时时钟 |
| 响应式布局 | R2 | 3 级断点 (1200/960/640) + hamburger |
| 搜索高亮 | R2 | 日志关键词黄色 mark 高亮 |
| 表格排序 | R3 | 站点 5 列 asc/desc 排序 |
| KPI sparkline | R3 | 迷你 SVG polyline 趋势线 |
| 日志展开 + CSV | R3 | 行展开详情 + Blob 下载 + 复制错误 |
| 拓扑全屏 | R3 | fixed 全屏 + Esc 退出 + 节点 hover tooltip |
| 新增站点表单 | R3 | Modal 6 字段 + 连接测试 |
| 无障碍 | R3 | focus-visible 全局样式 |
| Toast 自动消失 | R4 | 5s 倒计时 + 进度条 + hover 暂停 |
| 数据联动 | R4 | 协同组切换 Hero 动态更新 |
| 节点拖拽 | R4 | SVG 拖拽 + 流向自动跟随 + 重置布局 |
| 进度环 | R4 | ProgressRing 组件 (同步/文件/记录) |
| localStorage | R5 | usePersist hook (theme/notify/env) |
| 快捷键 | R5 | ?面板, 1-4=Tab, D=主题, N=通知 |
| 节点聚焦 | R5 | 选中时其他 dimmed (opacity:.4) |
| 脉冲动画 | R5 | bad 流向闪烁 + 确认弹窗 |

## 位置四问

| 项 | 答案 |
|---|---|
| 叙事角色 | 运维指挥台（hero + 数据混合） |
| 观众距离 | 1m 笔记本 · 1280–1920px 桌面端 |
| 视觉温度 | 冷静 + 权威（工业/运维气质，避 AI slop 紫渐变） |
| 容量估算 | 首屏同时看见：状态徽标 / 4 Tab 切换 / 拓扑图 / 主操作 |

## 信息架构（相对旧版的关键变化）

```
旧版: 一页竖向 6 段 (Header→状态卡→MQTT卡→Overview→Sites+Insights→Logs)
新版: Sticky 指挥条 + 左栏协同组列表 + 右侧 4 Tab 切换
      ├─ Tab 1 拓扑 (新增, 默认首屏)
      ├─ Tab 2 站点 (表格 + 详情抽屉, 密度降至 5 列)
      ├─ Tab 3 洞察 (指标卡 + 堆叠面积图 + 失败流向/近期异常)
      └─ Tab 4 日志 (筛选器 ↔ URL, cursor 分页)
```

## 假设清单 (Assumptions)

1. 数据结构沿用 `ui/admin/src/types/collaboration.ts` 的 9 个 interface，不发明新字段。
2. 拓扑数据 = `GET /api/remote-sync/envs/{id}/sites` + `GET /api/remote-sync/stats/flows`。
3. 运行时状态从 `GET /api/remote-sync/runtime/status` + `/runtime/config` 组合成单一徽标。
4. 主操作只保留「同步」；次操作「诊断」；更多菜单收纳：刷新 / 应用配置 / 停止运行时 / 编辑 / 删除。
5. 原型目标只覆盖**协同组详情**（单组视图）；多组管理页面后续迭代。
6. 单文件 inline React + Babel，双击 `index.html` 即可预览，无需起 http-server。

## 设计 Token

```css
--bg:         #FAFAF7
--ink-900:    #0F172A
--ink-700:    #334155
--ink-400:    #94A3B8
--line:       #E6E4DE
--brand:      #1F3A68     /* 深石板蓝，工业感 */
--brand-soft: #E8EDF3
--ok:   oklch(0.68 0.14 150)
--warn: oklch(0.80 0.14 80)
--bad:  oklch(0.65 0.20 25)
--radius: 10px
```

字体：

- Display：`EB Garamond / Source Serif Pro / Noto Serif SC`（页面主标题、数字指标）
- Body：`-apple-system, Segoe UI, PingFang SC, sans-serif`（延续 admin 一致性）
- Mono：`JetBrains Mono`（日志、host、时间戳）

## 反 AI slop 清单（本原型遵守）

| 不做 | 做 |
|---|---|
| 紫渐变 | 深石板蓝 + 暖灰 |
| 每标题配 icon | 只在操作按钮配小号 icon |
| 圆角卡 + 左彩色 border accent | 细 1px `--line` 边框 + 可选深色 heading bar |
| SVG 手画人/物 | 无——技术后台不需要 |
| Inter 做 Display | EB Garamond serif 做 Display |

## Junior Pass 进度

- [x] v0.1 骨架：指挥条 + 左栏 + 4 Tab 切换壳 + 拓扑 SVG
- [x] v0.2 拓扑填 mock（4 节点 / 4 方向 / 状态色分级 / 流量标签）
- [x] v0.2 站点 Tab 表格 + 详情抽屉（连接 / 元数据 / 诊断历史 / 危险操作）
- [x] v0.2 洞察 Tab 堆叠条形图 + 失败流向 Top3
- [x] v0.2 日志 Tab 筛选 + cursor 分页壳 + 错误内联展开
- [x] v0.2 Playwright 5 张截图（4 Tab + `#sites:s3` 抽屉深链）
- [x] v0.2 5 维评审（7.4/10） + Keep/Fix/Quick Wins
- [x] v0.3 Fix 全套：去重复徽标 · 层级重调 · tabular-nums · serif 稀缺化
- [x] **v0.4 补齐 web-server 对照的 A 级 9 条**（按 GAP_ANALYSIS.md 推进）：
  - A1 ONLINE/OFFLINE 实时连接徽标（带 ping 动画）
  - A2 日志 Tab 顶部活跃任务条（3 张卡 + 进度）
  - A3 失败流向可重试 + 失败任务队列紧凑列表
  - A4 站点 5 态状态机 chip（Idle/Scanning/ChangesDetected/Syncing/Completed/Error）
  - A5 站点抽屉 footer + 表格操作列按状态驱动（检测/同步/中止/重试）
  - A6 桌面通知开关 + Toast 系统（右上角）
  - A7 参数配置抽屉（4 分组 8 项：自动化 / 吞吐并发 / 连接重连 / 通知日志）
  - A8 暗色主题（`[data-theme="dark"]`，工业感而非 GitHub dark slop）
  - A9 site_id uppercase 角标（HBJ-SJZ-001 等 geo-coded）
  - 新增 URL 参数：`?theme=dark` · `?open=config`
- [x] Playwright 截图 7 张：拓扑 / 站点 / 洞察 / 日志 / 站点抽屉 / 参数抽屉 / 暗色拓扑
- [x] **Phase 1 入代码**：`ui/admin/src/style.css` 追加 `--collab-*` token 命名空间（opt-in, 不影响现有页面）
- [x] **Phase 2 入代码**：新增 `components/collaboration/CollaborationTopologyPanel.vue`（175 行模板 + scoped styles），对齐现有类型 `CollaborationSiteCard` / `CollaborationFlowStat`，动态放射状布局（本站居中、peer 均分圆周），未挂载到任何视图——零现有页面影响
- [x] **Phase 3A 挂载（最保守）**：在 `CollaborationWorkbenchView.vue` 的 OverviewPanel 和 SitesPanel 之间插入 `<details class="collab-topo-preview">`，默认折叠。完整 `npm run build` 通过：`vue-tsc` 零错误，`vite build` 0.79s 完成。用户访问 `/admin/#/collaboration` 可见「拓扑视图 · v2 预览」折叠块

## 代码迁移路线（Phase 1 已完成，后续按需推进）

| Phase | 内容 | 影响面 | 风险 |
|---|---|---|---|
| ✅ 1 | Design token 落入 `style.css`（`--collab-*` 命名空间） | 仅新增 | 零 |
| ✅ 2 | 新增 `components/collaboration/CollaborationTopologyPanel.vue` | 新文件 · 类型对齐现有 store | 零 |
| ✅ 3A | `CollaborationWorkbenchView.vue` 追加 `<details>` 折叠预览块 | +10 行模板 + 1 行 import | 极低 · 默认折叠 |
| 3B | `CollaborationWorkbenchView.vue` 改造为 4-Tab 壳（4 Tab + 路由 hash 同步） | 大改 | 中 · 影响 admin 可用性 |
| 4 | 把 `GroupSitesPanel.vue` 从卡片网格改表格 + 接入现有 `CollaborationSiteDrawer.vue` | 重构 | 中 |
| 5 | 日志筛选 ↔ URL query string + cursor-based 分页（对应架构文档 P3 #9） | 前后端协同 | 中 · 后端需加 `after_cursor` 参数 |
| 6 | Store 按领域拆分（对应架构文档 §9 方案） | 重构 1200 行 store | 高 · 单独 PR |

## 下一步选项

- (A) 继续做 Phase 2：新增 `CollaborationTopologyPanel.vue`
- (B) 回到原型态再迭代视觉（如换字体、换主色）
- (C) 生成 `.pen` 低保真稿（需配 `pencil-mcp`，当前环境暂缺）
- (D) 停在 Phase 1，交给团队 review 后再推 Phase 2+
