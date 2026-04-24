# 站点管理 · Flow Demo 开发计划

> 交付形态：**单文件 inline React HTML 原型**（双击打开 `index.html` 即可体验），配套 Playwright 自动化验证脚本。
>
> 设计基准：`../站点管理.pen`（pencil MCP 绘制的 8 个静态 hi-fi mockup）。
>
> 目的：把 pencil 里的静态稿升级为「真正能点」的桌面 Web 原型，让设计 review / 演示 / 上下游沟通都有一个可交互的证据。

---

## 1. 范围与非范围

### In scope —— 必做
- 列表页（默认 / 加载骨架 / 空态 / 筛选后 / 错误横幅 / 行 Loading）
- 机器资源概览（正常 / 警告 / 严重 三态切换）
- 新建/编辑抽屉（带关闭、保存、字段校验、解析范围 checkbox 联动）
- 详情页（运行概览 Tab + 配置信息 Tab + 日志三 Tab + 操作按钮）
- 删除二次确认弹框
- Toast 反馈（启动成功 / 解析完成 / 保存失败 等）
- 端到端流程走通：`列表 → 新建 → 回列表 → 详情 → 解析 → 启动 → 日志 → 编辑 → 停止 → 删除`

### Out of scope —— 不做
- 真实后端 / 真实数据库（全部 mock in-memory state）
- 生产级 router（只用单页 state 切换视图，不上 vue-router / react-router）
- 响应式 / 移动端适配（只针对 1440×900 桌面 mockup）
- 国际化 / 多主题切换
- a11y 无障碍彻底达标（保持基础语义即可）

---

## 2. 技术选型（遵循 huashu-design 单文件守则）

| 项 | 选择 | 理由 |
|----|------|------|
| UI 框架 | React 18 + ReactDOM (UMD, pinned) | 组件化 + 状态机表达清晰；pinned 版本避免升级翻车 |
| JSX 编译 | Babel Standalone (pinned) | 单文件场景不装 build 工具 |
| 样式 | CSS Variables + inline style + 少量 className | shadcn/Tailwind 的味道但不引入 Tailwind JIT |
| Icons | Lucide SVG（用 unpkg 直接取图标代码 inline 成组件） | 和 pencil 稿保持一致 |
| 字体 | `-apple-system`, `Inter`, `JetBrains Mono` | 和 admin 项目一致 |
| 动效 | `transition: all 0.18s ease` + `keyframes` spin / slide-in | 保持克制 |
| 加载方式 | 单文件 inline，`<script type="text/babel">` | 双击即开，无 `file://` 跨 origin 坑 |

硬约束（来自 huashu-design）：
1. **禁用** `const styles = {...}` 裸命名 —— 必须加前缀如 `tableStyles`, `drawerStyles`
2. **禁用** `scrollIntoView` —— 会破坏容器滚动
3. Stage 固定 1440×900 内容区 + auto-scale letterbox，避免笔记本小屏溢出

---

## 3. 状态机设计

单一 `useReducer` 驱动整个 demo。**一个 state 管所有**：

```
state = {
  route: 'list' | 'detail',
  detailSiteId?: string,
  drawer: { open: boolean, mode: 'create' | 'edit', siteId?: string },
  dialog: null | { type: 'delete', siteId: string },
  toast: null | { tone: 'success' | 'warning' | 'error', message: string },
  quickFilter: 'all' | 'running' | 'busy' | 'error' | 'pending_parse',
  search: string,
  statusFilter: '' | 'Running' | 'Stopped' | ...,
  riskFilter: '' | 'warning' | 'critical',
  detailTab: 'overview' | 'config',
  logTab: 'parse' | 'db' | 'web',
  resourceRisk: 'normal' | 'warning' | 'critical',  // devtools 可强制切
  sites: Site[],             // 主数据
  pendingActions: Record<siteId, 'start'|'stop'|'parse'>,
}
```

Actions:
- `OPEN_DRAWER`, `CLOSE_DRAWER`, `SAVE_SITE`
- `START_SITE`, `STOP_SITE`, `PARSE_SITE`（触发 mock 耗时 1.5s 的 pending）
- `OPEN_DIALOG`, `CONFIRM_DELETE`
- `SHOW_TOAST`, `HIDE_TOAST`
- `SET_QUICK_FILTER`, `SET_SEARCH`, `SET_STATUS`, `SET_RISK`
- `OPEN_DETAIL`, `BACK_TO_LIST`, `SET_DETAIL_TAB`, `SET_LOG_TAB`
- `SET_RESOURCE_RISK`（devtools 下拉切换）

异步用 `setTimeout` 模拟：`dispatch pending → 1.5s → dispatch resolve + toast`。

---

## 4. 文件结构

```
design/site-admin-flow-demo/
├── PLAN.md                  ← 本文件
├── README.md                ← 打开指南 + 快捷键 + 已知边界
├── index.html               ← 单文件原型（所有 JSX 内联）
├── screenshots/             ← Playwright 截图输出（10 张关键屏）
└── tests/
    └── smoke.spec.mjs       ← Playwright 点击测试 + pageerror=0 校验
```

---

## 5. 分阶段执行（每阶段结束都 playwright 截图推给用户）

### Phase A · 骨架搭建（目标：页面能打开，CSS Variables 生效）
- 写 `<head>`：CSS Variables（复用 pencil 里的 33 个 token）、字体、reset
- `<body>` 挂 `<div id="root">` + `<script type="text/babel">`
- React + ReactDOM + Babel CDN 引入
- 顶层 `<App />` 渲染"占位主标题"+ "loading..." —— Junior pass 先让页面能开
- **交付：能打开的空壳** ✅ show 给用户

### Phase B · 列表页完整还原（目标：视觉对齐 pencil 稿）
- WorkbenchHeader 组件
- StatsGrid 4 张统计卡
- ResourceSection（含风险 banner 三态切换）
- SiteToolbar（快速筛选 chips + 搜索 + 下拉 + 新建按钮）
- SiteDataTable（5 行典型站点 + 5 列完整数据）
- 筛选联动（选 chips 实时过滤）

### Phase C · 抽屉交互
- SiteDrawer 从右滑入（`transform: translateX(100%) → 0`）+ 遮罩淡入
- 4 fieldset 完整表单
- 解析范围 checkbox（DESI / PROP / EQUP / PADD / SYST / DICT / CATA / 强制重建 SYST）联动
- 保存按钮：1.2s pending → 关闭抽屉 → 列表插入新行 + 成功 toast
- 空必填禁用保存

### Phase D · 详情页
- 点击列表行跳转详情（route=detail）
- SiteDetailHeader（返回、状态、parse_plan、操作按钮组）
- Tab 切换：运行概览 / 配置信息
- 运行概览：4 runtime 卡 + 解析计划 + 风险摘要 + 3 进程卡 + 目录 + 访问地址 + 日志面板（3 tab）
- 配置信息：readonly fieldset 复用新建抽屉布局

### Phase E · 状态机动作
- 启动 / 停止 / 解析：按钮点击 → pending spin 1.5s → resolve + toast + 状态切换 + 生成新日志
- 删除：弹框确认 → pending → 从 sites 移除 + toast + 回列表
- 错误注入：按住 `Shift` 点击"启动"→ 触发失败 toast + 错误横幅（演示错误态）

### Phase F · Devtools + 风险切换
- 右下角浮动 devtools 面板（pointer-events: auto）
- 可切：机器资源风险（正常/警告/严重）、模拟当前时间、重置数据
- 目的：让 reviewer 不点来点去也能看到所有状态

### Phase G · Playwright 校验 + 交付
- `smoke.spec.mjs` 覆盖：
  1. 列表页加载 pageerror=0
  2. 点击"运行中" chip → 表格只剩 Running
  3. 点击"新建站点" → 抽屉打开 → 填写最小字段 → 保存 → 列表 +1 行
  4. 点击任意行 → 详情页 → Tab 切换 → 日志 tab 切换
  5. 点击 Viewer 按钮不抛错
  6. devtools 切换风险到"严重" → banner 变红
- 每个关键屏输出 1 张 PNG 到 `screenshots/`
- 推送给用户验收

---

## 6. 交付清单

- [ ] `PLAN.md` ← 本文件（已完成）
- [ ] `index.html` ← 单文件可运行原型
- [ ] `README.md` ← 打开指南
- [ ] `screenshots/*.png` ← 10 张关键屏
- [ ] `tests/smoke.spec.mjs` ← Playwright 测试脚本
- [ ] 最终 show 给用户：在浏览器打开 + 关键流程截图

---

## 7. 时间预算

| 阶段 | 预计 batch 数 | 主要复杂度 |
|------|----|------|
| A 骨架 | 1 批 write | 配置 CSS 变量 + React CDN |
| B 列表页 | 1 批 write（大文件分块） | StatsGrid + Table 最重 |
| C 抽屉 | 1-2 批 edit | 表单字段多 |
| D 详情页 | 1-2 批 edit | 卡片组很多 |
| E 状态机 | 1-2 批 edit | reducer 扩展 |
| F devtools | 1 批 edit | 轻量浮动面板 |
| G 验证 | 1 批 shell | playwright 跑测试 |

---

## 8. 审美守则（反 AI slop）

- ❌ 禁用紫色渐变 / emoji 图标 / 圆角卡片+左彩 border accent / SVG 画人
- ✅ 沿用 pencil 稿的 shadcn 配色（neutral + 状态色 6 种）
- ✅ 一处做到 120%：状态标签的配色 + 日志面板的 mono 等宽排版
- ✅ 数据用领域真实感名字（AvevaPlantSample-18330 / KronosRefineryModel-22901），不用 Lorem ipsum
- ✅ 所有 Loading / empty / error 都是"诚实 placeholder"而非编造数据
