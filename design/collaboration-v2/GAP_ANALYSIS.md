# Hi-Fi 原型 vs web-server 生产实现 · 差距分析

> 参考源：`D:\work\plant-code\web-server\frontend\src\`
>
> 参考对象：`design/collaboration-v2/index.html`（v0.3） + `ui/admin/src/components/collaboration/*`（当前 admin）

## 先定位差异（很重要）

**两者不是同一个产品**：

| 维度 | `web-server/frontend` | `admin + v2 原型` |
|---|---|---|
| 定位 | 专门的「增量同步监控台」 | 「部署/协同管理后台」的子模块 |
| 导航 | 独立侧栏 11 个一级视图 | 作为 admin 的一条子路由 `/collaboration` |
| 视角 | **文件/同步状态机** 驱动 | **站点/连接关系** 驱动 |
| 数据实时性 | WebSocket `/ws/tasks` + SSE `/api/sync/events/stream` 双通道 | 30s 轮询 |
| 核心价值 | 看得见每一个文件的同步进度 | 看得见站点间关系和异常 |

**原则**：v2 原型不应该"抄 web-server"，而应该**吸收**它那些"真正帮到运维人"的细节。否则会把一个轻量管理入口变成一个重型专业监控台，不合 admin 定位。

---

## 差距清单（按「该不该补进 v2 原型」分类）

### A · 应补进原型（属于 admin 定位范围内）

| # | 缺的功能 | web-server 里的参考 | 原因 | 补法建议 |
|---|---|---|---|---|
| A1 | **实时连接指示**（WebSocket/SSE 状态） | `App.vue` 顶栏 `ONLINE` ping 动画 + `IncrementalUpdateMonitor` 的 `实时连接/离线（轮询模式）` | 运维最担心"我看到的数据是实时还是过期的" | 指挥条右侧加一个 `ONLINE/OFFLINE` ping 徽标，断线时整条 statusbar 变暗 |
| A2 | **"进行中任务"实时列表** | `IncrementalUpdateMonitor` 的活跃任务卡片（名称 + 路径 + 进度条） | 当前原型日志 Tab 只显示"已发生"的事件，看不到"正在进行"的 | 在拓扑 Tab 的右侧详情面板增加一个 `进行中 · 2` 小块（或在日志 Tab 顶部） |
| A3 | **失败任务队列 + 重试/清理** | `IncrementalUpdateMonitor` 的"失败任务队列"块 | 原型现在对失败流只能看，不能操作 | 洞察 Tab 的"失败流向 Top"每项加"重试全部 / 清理已耗尽"按钮 |
| A4 | **站点状态机**（Idle / Scanning / ChangesDetected / Syncing / Completed / Error） | `SiteCard.vue` 的 5 状态 | 原型现在只有在线/缓存/离线 3 态，粒度不够 | 拓扑节点 + 站点表格增加 `检测中 / 已发现变更 / 同步中` 中间态和对应颜色 |
| A5 | **状态驱动的操作按钮**（检测变更 / 开始同步 / 中止） | `SiteCard.vue` 的条件按钮 | 原型只有"测试 / 编辑"，没有真正驱动同步流程的动作 | 站点抽屉底部 footer 按状态条件渲染：当前 Idle → 检测变更；ChangesDetected → 开始同步；Scanning/Syncing → 中止 |
| A6 | **桌面通知 + Toast** | `App.vue` 的 `enableNotifications` + toast-top-end | 异地同步失败要能"不盯屏幕也知道" | 指挥条增加"通知"开关，失败/完成时 `Notification + Toast` |
| A7 | **参数配置面板**（autoDetect / detectionInterval / batchSize / maxConcurrent / logRetention） | `SettingsManager.vue` | 原型只展示状态，没有调参入口 | 指挥条"更多"菜单加一项"参数配置"，打开全屏抽屉 |
| A8 | **暗色主题** | `App.vue` 的 `isDarkMode` + Naive UI theme overrides | 运维长时间盯屏幕，暗色是基本需求 | `--collab-*` token 增加 `.dark` 变体 |
| A9 | **站点 ID 极简角标** | `SiteCard.vue` 右上 `{{ site_id }}` uppercase letter-spaced | 地理印章是"地区"，缺了"这是哪个 site_id" | 站点卡右上角小字 uppercase site_id（补齐识别度） |

### B · 可选补进（要不要补看产品定位）

| # | 功能 | web-server 有 | v2 要不要补 |
|---|---|---|---|
| B1 | 增量元素级详情（查看具体 REFNO / noun / session / name） | `IncrementalUpdateMonitor` 的增量详情模态框（带分页 100/页） | 如果 admin 要支持"钻取到文件里的每一行变更"就补，否则留给 web-server 做 |
| B2 | 拓扑图可 pan / zoom / drag | `TopologyVisualization.vue` 的 SVG 互动 | 节点 ≤ 10 个时不需要；多站点场景（≥20）值得补 |
| B3 | MQTT 消息日志查看器（Topic / QoS / Payload） | `MqttMessageViewer.vue` | admin 已有通用日志 Tab，不必重复 MQTT 专用；如果 admin 要做"MQTT 调试台"再补 |
| B4 | MQTT 节点监控（哪个节点订阅了哪个 topic） | `MqttNodeMonitorEnhanced.vue` | 同 B3，留给 web-server |
| B5 | 归档管理（历史 CBA 文件浏览/下载） | `ArchivesManager.vue` | admin 已有 `/sites/:id/files` 代理，不必重复 |
| B6 | 同步趋势图 / 状态分布图（ECharts 或类似） | `SyncTrendChart.vue` + `SiteStatusChart.vue` | 洞察 Tab 现在只有简易堆叠条形图，**可以升级为折线+面积图**（不引入 chart 库的话用 SVG 手绘） |

### C · 不补（admin 不是那个定位）

| # | 功能 | 理由 |
|---|---|---|
| C1 | 全局概览 Dashboard | 跟 admin 的 `/sites` 总览页面重叠 |
| C2 | 独立的"任务队列"一级 Tab | admin 有 `/tasks` 已在做 |
| C3 | Session 范围 / 增量元素 REFNO 级钻取 | 属于调试工具范畴，不是协同管理 |
| C4 | Logs 页的自动 5s 刷新 | 原型的"URL 筛选 + cursor 分页"已经是更好的范式 |

---

## 关键"产品气质"差异

web-server 的视觉：**Naive UI + DaisyUI + FontAwesome + Tailwind**，用了大量蓝紫色 gradient（purple-50 / indigo-50 / from-blue 等）+ 圆角 + shadow。

v2 原型的视觉：**shadcn/ui 风格 + 自定义 token + EB Garamond serif**，冷静工业感。

**不建议**：把 web-server 的 purple/indigo/emerald 渐变和 FontAwesome 整套搬过来——会破坏 v2 原型已经建立的"反 AI slop"气质。

**建议**：只吸收 web-server 的**功能点**和**信息架构洞察**，不吸收视觉风格。

---

## 优先级推荐（如果要补进 v2）

### 高 · 立刻补（5 分钟 × 3 件，全是视觉+信息密度调整）

1. 指挥条加 `ONLINE/OFFLINE` ping 徽标（A1）
2. 站点表格 + 拓扑节点加 5 态 `status chip`（A4 部分，只加 status 枚举和颜色，不实现切换逻辑）
3. 站点卡右上角补 `site_id` uppercase 角标（A9）

### 中 · 原型迭代（30 分钟 × 3 件）

4. 站点抽屉 footer 按状态驱动的条件按钮（A5）
5. 日志 Tab 顶部新增「进行中」活跃任务条（A2）
6. 洞察 Tab 失败流向 Top 每项加「重试/清理」按钮（A3）

### 低 · 需要决策（要不要做）

7. 暗色主题（A8）→ 等 admin 全站决策
8. 参数配置面板（A7）→ 后端 API 已有？要确认
9. 桌面通知（A6）→ 侵入式，可选

---

## 更新后的原型结构（如果补齐 A 级）

```
CollaborationWorkbench v2.1
├── 指挥条
│   ├─ 面包屑 + 状态徽标
│   ├─ ● ONLINE (A1)  ← 新增
│   ├─ 主操作按状态变化 (A5)
│   └─ 更多菜单 + [参数配置] (A7)
│
├── 左栏 协同组
│
└── 4 Tab
    ├─ 拓扑 · 节点 5 态 + 进行中数量 (A2 A4)
    ├─ 站点 · 表格多一列状态机 + site_id 角标 (A4 A9)
    ├─ 洞察 · 失败流向可操作 (A3)
    └─ 日志 · 顶部"进行中"活跃任务条 (A2)
```

---

## 结论

**原型 v0.3 的骨架已经够用**。web-server 提供的最大启示是 3 个操作维度上的缺失：

1. **"看得见实时"**：ONLINE 徽标、进行中任务
2. **"可以操作"**：按状态驱动的按钮、失败任务重试
3. **"可以调参"**：参数配置面板

如果用户选择补齐 A 级清单里的 9 条，v2 原型在「运维可用性」上会达到与 web-server 近似的水平，同时保持 admin 的轻量定位和 huashu-design 的反 slop 气质。
