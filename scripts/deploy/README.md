# 一键部署脚本

通过 GitHub Actions Deploy workflow 把 **plant-model-gen** + **plant3d-web** 同步到生产服务器（`123.57.182.243`），并自动验证部署成功。

## 适用场景

- 已在本地完成 RUS-244 / 同类业务修复，commit 落到对应分支
- 想要"一键"完成 push → 等 CI 部署 → 远端验证 → 输出汇总
- 不想手动盯 GitHub Actions 页面 + 手动 SSH 验证

## 工作机制

```
┌──────────────────────────────────────────────────────────────────────────┐
│ 1. 检查 git 工作区 / 当前分支                                             │
│ 2. 记录远端产物 mtime 基线（/root/web_server, /var/www/plant3d-web/...) │
│ 3. git push origin <branch> 把代码推到 GitHub                              │
│ 4. 触发 GitHub Actions workflow_dispatch deploy workflow：                │
│    - 优先：gh workflow run（gh CLI 已登录） + gh run watch 直到完成        │
│    - 兜底：打印 GitHub UI Run workflow 链接 + git_ref 让用户手动触发       │
│ 5. 轮询远端产物 mtime，验证 CI 已经替换 binary / dist                      │
│ 6. 健康检查：                                                              │
│    - backend: curl http://localhost:3100/api/review/workflow/sync (query) │
│    - frontend: curl http://123.57.182.243/version.json + index.html       │
│    - systemd: systemctl is-active web-server.service                      │
│ 7. 输出 JSON summary                                                       │
└──────────────────────────────────────────────────────────────────────────┘
```

### 关于 GitHub CLI（gh）

deploy workflow 是 `workflow_dispatch` 手动触发模式（push 不会自动启动 CI）。
脚本支持两种触发方式：

- **自动模式（gh 已登录）**：`gh workflow run <workflow.yml> --ref <branch> -f git_ref=<branch>` 触发，
  然后 `gh run watch` 阻塞到 run completed。整个过程无人工。
- **手动模式（gh 不可用 / 未登录 / 传 `-NoGh`）**：脚本输出 GitHub Actions URL + git_ref 提示，
  用户去 UI 手动点 Run workflow，脚本继续轮询远端 mtime 等 CI 完成。

**安装并登录 gh**（推荐，全自动一键部署）：

```powershell
winget install --id GitHub.cli
# 重启 PowerShell 让 PATH 生效
gh auth login          # 浏览器交互登录
# 或：用 PAT（至少 repo + workflow scope）
echo "ghp_xxx..." | gh auth login --hostname github.com --with-token
gh auth status         # 验证
```

## 前置条件

```powershell
pip install paramiko
```

GitHub 端：
- 仓库已配置好 Deploy to Ubuntu workflow（参考 `/root/setup-deploy-server.sh`）
- 本地 git 能 push 到 origin（已配置 SSH key 或 GitHub Token）
- 推荐：`gh` CLI 已 `gh auth login` 登录，token 含 `repo` + `workflow` scope

SSH 端：
- 默认用户/密码已写在脚本中（与 setup-deploy-server.sh 一致）
- 可用环境变量覆盖：`DEPLOY_HOST`、`DEPLOY_USER`、`DEPLOY_PASSWORD`

## 用法

### 标准发布（部署两个仓库）

```powershell
cd D:\work\plant-code\plant-model-gen
.\scripts\deploy\deploy.ps1
```

预期输出：

```
[STEP] ==== [plant-model-gen] 开始 ====
[OK]  当前分支 feat/collab-api-consolidation
[INFO] 部署前 /root/web_server mtime=2026-05-09 18:44:29
[STEP] git push origin feat/collab-api-consolidation
[OK]  push 成功
[STEP] 等待 GitHub Actions Deploy workflow 完成（最长 1800s）
[INFO] 进度可在 https://github.com/happyrust/plant-model-gen/actions 查看
[OK]  /root/web_server mtime 已更新 → 2026-05-09 20:12:17
[OK]  backend sync(query) HTTP 200
[OK]  web-server.service active
[STEP] ==== [plant3d-web] 开始 ====
...
[OK]  全部仓库部署成功 ✅
```

### 仅发布前端

```powershell
.\scripts\deploy\deploy.ps1 -Repo plant3d-web
```

### 仅发布后端

```powershell
.\scripts\deploy\deploy.ps1 -Repo plant-model-gen
```

### Dry-run（只检查状态，不 push）

```powershell
.\scripts\deploy\deploy.ps1 -NoPush
```

### push 但不等 CI 完成

```powershell
.\scripts\deploy\deploy.ps1 -SkipVerify
```

### 自定义超时（默认 1800s）

```powershell
.\scripts\deploy\deploy.ps1 -Timeout 3600
```

## 失败排查

| 现象 | 原因 | 解决 |
|---|---|---|
| `当前分支 X ≠ 预期 Y` | 你切到了别的分支 | `git checkout <expected-branch>` 后重跑 |
| `本地 behind upstream N commit` | 远程有新提交 | `git pull --rebase origin <branch>` 后重跑 |
| `push 失败：…` | 鉴权过期 / 网络问题 | `git push` 看 git 报错；通常重新登录 GitHub |
| `超时未更新：/root/web_server` | CI 失败或耗时超过 timeout | 看 GitHub Actions 页面具体错误；调大 `-Timeout` |
| `backend sync(query) HTTP 5xx` | 部署后服务起不来 | SSH 远端 `journalctl -u web-server.service -n 200` |
| `web-server.service 状态=failed` | systemd 启动 binary 失败 | 同上，看 journal |

## 修改部署目标

编辑 `deploy_to_production.py` 里的 `REPOS` 字典：

```python
REPOS: dict[str, RepoSpec] = {
    "plant-model-gen": RepoSpec(
        name="plant-model-gen",
        local_path=WORKSPACE_ROOT / "plant-model-gen",
        expected_branch="feat/collab-api-consolidation",  # ← 改这里
        remote_artifacts=["/root/web_server"],            # ← 改远端产物路径
        ...
    ),
    ...
}
```

## 已知边界

- 不主动调 GitHub API 看 workflow 状态（避免依赖 gh / token）
- 仅通过远端文件 mtime 判断 CI 完成，**如果 deploy workflow 替换 binary 但服务起不来，脚本仍会显示"产物更新"**，需要靠 health check 兜底
- 健康检查只覆盖 backend `/api/review/workflow/sync` 与前端静态资源；业务级 e2e 验证请单独跑 `npm run test:pms:simulator`
