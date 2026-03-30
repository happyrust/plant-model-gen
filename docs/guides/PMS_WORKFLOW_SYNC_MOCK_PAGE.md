# PMS 模拟联调页

用于把当前 `workflow/sync` 的真实返回，整理成本地可点击的 PMS 联调页，方便快速核对：

- `records`
- `annotation_comments`
- `attachments`
- `attachments[].type`
- `attachments[].route_url`

## 相关文档导航

- **Guides 总入口**：`docs/guides/README.md`
- **完整接口 / 数据库测试模拟**：`docs/guides/WORKFLOW_SYNC_FORM_ID_TEST_SIMULATION.md`
- **Playwright 教程 + 截图验收**：`docs/guides/WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md`

## 用法

### 1. 连接本地 web_server

```bash
BASE_URL=http://127.0.0.1:3100 ./shells/run_pms_workflow_mock_page.sh
```

### 2. 连接生产环境

```bash
BASE_URL=http://123.57.182.243 ./shells/run_pms_workflow_mock_page.sh
```

默认会：

1. 调真实 `embed-url / tasks / records / comments / attachments / workflow/sync`
2. 在本地生成：
   - `index.html`
   - `response.json`
   - `summary.json`
3. 启动本地静态服务：`http://127.0.0.1:8765/index.html`
4. 默认自动删除本次生成的测试 `form_id`

## 常用环境变量

```bash
PORT=8877 OUT_DIR=/tmp/pms_page OPEN_BROWSER=false CLEANUP_FORM=false BASE_URL=http://123.57.182.243 ./shells/run_pms_workflow_mock_page.sh
```

- `PORT`：本地静态服务端口，默认 `8765`
- `OUT_DIR`：输出目录，默认 `/tmp/pms_workflow_mock_page`
- `OPEN_BROWSER`：是否自动打开浏览器，默认 `true`
- `CLEANUP_FORM`：是否自动删除测试单据，默认 `true`

## 页面内容

页面会展示：

- 原始 `workflow/sync` 返回 JSON 入口
- 附件 / 截图链接（拼接 `BASE_URL + route_url`）
- `records` 原始 JSON
- `annotation_comments` 原始 JSON

## 注意事项

1. 页面是“展示层联调页”，不依赖旧的 `opinions`。
2. 附件展示按 `attachments[].type` / `route_url` / `file_ext` 渲染，不要依赖数组顺序。
3. 若开启 `CLEANUP_FORM=true`，数据库记录会被删，但已生成到 `response.json` 的展示内容仍会保留在本地输出目录中。
