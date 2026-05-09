# 一键部署脚本

通过 GitHub Actions Deploy workflow 把 **plant-model-gen** + **plant3d-web** 同步到生产服务器（`123.57.182.243`），并自动验证部署成功。

## 适用场景

- 已在本地完成 RUS-244 / 同类业务修复，commit 落到对应分支
- 想要"一键"完成 push → 等 CI 部署 → 远端验证 → 输出汇总
- 不想手动盯 GitHub Actions 页面 + 手动 SSH 验证

## 工作机制

```
┌──────────────────────────────────────────────────────────────────────────┐
│ 1. 检查 git 工作区 / 当前分支匹配预期                                      │
│ 2. 记录远端产物 mtime 基线（/root/web_server, /var/www/plant3d-web/...）  │
│ 3. git push origin <branch> → GitHub Actions Deploy workflow 自动触发     │
│ 4. 轮询远端产物 mtime，直到所有 artifact 都比基线新（CI 完成）             │
│ 5. 健康检查：                                                              │
│    - backend: curl http://localhost:3100/api/review/workflow/sync (query) │
│    - frontend: curl http://123.57.182.243/version.json + index.html       │
│    - systemd: systemctl is-active web-server.service                      │
│ 6. 输出 JSON summary                                                       │
└──────────────────────────────────────────────────────────────────────────┘
```

不依赖 GitHub CLI（gh），纯 SSH（paramiko）+ git 命令实现。

## 前置条件

```powershell
pip install paramiko
```

GitHub 端：
- 仓库已配置好 Deploy to Ubuntu workflow（参考 `/root/setup-deploy-server.sh`）
- 本地 git 能 push 到 origin（已配置 SSH key 或 GitHub Token）

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
