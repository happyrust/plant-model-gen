# Guides 文档导航

> 面向 `plant-model-gen` 日常联调、部署、接口核查与回归验证的快速入口页。

---

## 1. workflow / PMS / reviewer 联调入口

如果你当前关注的是：

- `workflow/sync`
- `form_id`
- 校审面板保存数据如何落库
- reviewer 页面如何恢复历史批注 / 测量
- 如何用 Playwright 做截图验收

建议按下面顺序阅读：

### 第一步：接口与数据库事实

1. [workflow sync 按 form_id 返回校审数据：完整测试模拟文档](./WORKFLOW_SYNC_FORM_ID_TEST_SIMULATION.md)
   - 适合先确认：
     - `records` 如何保存
     - `workflow/sync` 如何按 `form_id` 返回
     - 如何用 CLI + JSON 查库

2. [Platform API — HTTP 请求示例](./PLATFORM_API_HTTP_EXAMPLES.md)
   - 适合手工 curl / PMS 后端联调
   - 包含 `embed-url`、`workflow/sync`、`delete` 示例

### 第二步：页面与截图验收

3. [workflow sync / form_id 联调教程（附 Playwright 截图）](./WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md)
   - 适合前后端联调和页面恢复验证
   - 包含 reviewer 页面 URL 规则、Playwright 示例脚本、截图说明

4. [PMS 模拟联调页](./PMS_WORKFLOW_SYNC_MOCK_PAGE.md)
   - 适合快速生成本地 mock 展示页
   - 用于查看 `records / annotation_comments / attachments`

---

## 2. workflow 相关文档之间的关系

```text
WORKFLOW_SYNC_FORM_ID_TEST_SIMULATION.md
  -> 说明接口 / 数据库 / form_id 事实源

WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md
  -> 说明 reviewer 页面恢复与 Playwright 截图验收

PMS_WORKFLOW_SYNC_MOCK_PAGE.md
  -> 说明如何把 workflow/sync 返回渲染成可点击 mock 页面

PLATFORM_API_HTTP_EXAMPLES.md
  -> 说明 PMS 后端如何直接调用接口
```

---

## 3. 典型阅读路径

### 路径 A：后端 / 数据联调

推荐顺序：

1. `WORKFLOW_SYNC_FORM_ID_TEST_SIMULATION.md`
2. `PLATFORM_API_HTTP_EXAMPLES.md`

适合问题：

- 为什么 `workflow/sync` 没返回 records
- `review_records.form_id` 有没有真的落库
- 如何手工 POST 验证送审返回

### 路径 B：前端 / 页面恢复联调

推荐顺序：

1. `WORKFLOW_SYNC_FORM_ID_TEST_SIMULATION.md`
2. `WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md`

适合问题：

- reviewer 页面为什么没恢复到目标单据
- 历史批注 / 测量是否重新显示
- Playwright 应该怎么截图验收

### 路径 C：PMS 展示层联调

推荐顺序：

1. `PLATFORM_API_HTTP_EXAMPLES.md`
2. `PMS_WORKFLOW_SYNC_MOCK_PAGE.md`
3. `WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md`

适合问题：

- PMS 如何消费 `workflow/sync`
- 如何快速生成本地 mock 页
- 如何做带截图的联调交付

---

## 4. 配套截图资源

当前 workflow / Playwright 教程已配套的截图资源在：

- `assets/workflow_sync_form_id_playwright/reviewer-page.png`
- `assets/workflow_sync_form_id_playwright/viewer-canvas.png`

---

## 5. 说明

1. 本目录以“运行态事实”和“可复跑联调”为优先，不以 Rust `test` 为主要验证入口。
2. 对 `web_server` / review 子系统的验证，优先：
   - 启动服务
   - 真实 HTTP POST / GET
   - Surreal CLI + JSON
3. 如果某份文档与运行态不一致，以：
   - 当前后端代码
   - 当前前端运行态
   - 当前生产验收结果
   为准。

