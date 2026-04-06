# Quick Deploy Reference

## Prerequisites

- **For local macOS builds (default path):** `cargo install cargo-zigbuild` and [Zig toolchain](https://ziglang.org/download/)
- **For GitHub artifact/release deploys:** `gh` CLI installed and authenticated
- **Remote access:** `sshpass` and `rsync` installed locally
- **Remote credentials:** Target server defaults to `123.57.182.243` (`root`); `REMOTE_PASS` must be supplied explicitly

## Deploy from Local Build (Default Path)

### Full Stack (Backend + Frontend)

```bash
# macOS → Linux cross-compilation happens automatically
./shells/deploy_all_with_frontend.sh
```

This is the **verified default workflow** for local development:
1. Auto-detects macOS and uses `cargo zigbuild` to build for Linux x86_64
2. Deploys backend binary, assets, and configuration
3. Builds and deploys frontend bundle to nginx
4. Verifies remote services (systemctl, curl health checks)
5. Includes retry logic (5 attempts) for transient SSH/rsync failures

### Backend Only

```bash
./shells/deploy_web_server_bundle.sh
```

### Force Native Build (No Cross-Compilation)

```bash
USE_ZIGBUILD=false ./shells/deploy_all_with_frontend.sh
```

## Deploy from GitHub Actions (CI Artifacts)

```bash
# 1. Find the latest successful build
gh run list --workflow multi-platform-build.yml --status success --limit 5

# 2. Deploy using the run ID
BINARY_SOURCE=github-artifact GITHUB_RUN_ID=<RUN_ID> ./shells/deploy_all_with_frontend.sh
```

## GitHub Actions 直连 Ubuntu 部署

如需让后端完全由 GitHub CI 直接发布到 Ubuntu，请在仓库 Actions 手动触发 `Deploy Web Server To Ubuntu`。

先配置以下仓库变量与密钥：

- Variables:
  - `DEPLOY_REMOTE_HOST`
  - `DEPLOY_REMOTE_USER`（可选，默认 `root`）
- Secrets:
  - `DEPLOY_REMOTE_PASS`
  - `GH_PAT`（若 CI 仍需拉取私有 patch 依赖）

该 workflow 默认使用 `db_options/DbOption-aveva-1600.toml`，并在 Runner 上完成：

1. 构建 Linux `web_server`
2. 产出并下载同次 artifact
3. 调用 `deploy_web_server_bundle.sh` 上传二进制、`assets/`、`output/`、`DbOption`
4. 重启远端 `web-server` systemd
5. 通过 SSH 验证 `systemctl is-active web-server`、`/api/health`、`/api/projects`

## Deploy from GitHub Release

```bash
BINARY_SOURCE=github-release GITHUB_TAG=v1.2.3 ./shells/deploy_all_with_frontend.sh
```

## Post-Deployment Verification

After deployment completes, verify the services manually:

```bash
# On local machine:
curl -fsS http://123.57.182.243/
curl -fsS http://123.57.182.243/api/projects
curl -fsS http://123.57.182.243/api/version

# On remote server:
ssh root@123.57.182.243
systemctl status web-server
systemctl status nginx
ls -lh /root/web_server
curl -fsS http://127.0.0.1:3100/
curl -fsS http://127.0.0.1:3100/api/version
curl -fsS http://127.0.0.1:3100/api/projects
```

当前生产 `web-server.service` 仍通过 `/root/web_server` 启动，因此 tag/CI 部署除了维护
`/opt/plant-model-gen/releases/<version>` 与 `current` 软链之外，还必须同步刷新
`/root/web_server`。否则即便 `current` 已切换，线上进程仍可能继续运行旧二进制。

All of these checks are also run automatically by `deploy_all_with_frontend.sh` with retry logic.

## Custom Server Configuration

Override defaults for non-standard deployments:

```bash
REMOTE_HOST=your.server.ip \
REMOTE_USER=your_user \
REMOTE_PASS='your_password' \
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=12345678 \
./shells/deploy_all_with_frontend.sh
```

## Environment Variables Reference

| Variable | Default | Description |
|----------|---------|-------------|
| `BINARY_SOURCE` | `local` | Source: `local`, `github-artifact`, `github-release` |
| `USE_ZIGBUILD` | `auto` | Cross-compile control: `auto`, `true`, `false` |
| `ZIGBUILD_TARGET` | `x86_64-unknown-linux-gnu` | Target triple for zigbuild |
| `GITHUB_RUN_ID` | - | Required for `github-artifact` source |
| `GITHUB_TAG` | - | Required for `github-release` source |
| `ARTIFACT_NAME` | `linux-x64-release` | Artifact name for GitHub downloads |
| `REMOTE_HOST` | `123.57.182.243` | Target server IP |
| `REMOTE_USER` | `root` | SSH username |
| `REMOTE_PASS` | - | SSH password, must be provided explicitly |
| `BACKEND_ORIGIN` | `http://127.0.0.1:3100` | Backend URL for nginx proxy |
| `FRONTEND_PROJECT_DIR` | 自动：`plant-model-gen` 同级目录下的 `plant3d-web` | 前端仓库根目录（覆盖默认路径） |

## Retry Behavior

All deployment scripts include automatic retry logic with exponential backoff (5 attempts, starting at 2s delay) for:
- SSH authentication and remote command execution
- rsync/scp file transfers
- Remote health check verification

This handles intermittent SSH auth failures and network instability without manual intervention.

## SurrealDB（图数据）同步到服务器

仅部署 `web_server` 与 `output/` **不会**带上 Surreal 中的 MDB/WORL/pe 等数据。若 `/api/e3d/world-root` 报 `get_world_refno failed`，需要把本地 RocksDB 目录同步到服务器并启动 8020 端口上的 Surreal。

详见 [docs/guides/SURREAL_REMOTE_SYNC.md](docs/guides/SURREAL_REMOTE_SYNC.md)。

- **生产 SurrealKV 改绑 8020 + systemd 固定脚本**：`./shells/apply_surrealdb_8020_remote.sh`
- **整库 RocksDB rsync**（另一套数据路径）：`./shells/sync_surreal_8020_to_remote.sh`

```bash
REMOTE_HOST=123.57.182.243 REMOTE_USER=root REMOTE_PASS='...' ./shells/apply_surrealdb_8020_remote.sh
```

## Additional Documentation

- [Complete GitHub Artifact Deployment Guide](docs/GITHUB_ARTIFACT_DEPLOYMENT.md)
- [Deployment Implementation Details](DEPLOYMENT_IMPLEMENTATION_SUMMARY.md)
