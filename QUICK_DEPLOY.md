# Quick Deploy Reference

## Prerequisites

- **For local macOS builds (default path):** `cargo install cargo-zigbuild` and [Zig toolchain](https://ziglang.org/download/)
- **For GitHub artifact/release deploys:** `gh` CLI installed and authenticated
- **Remote access:** `sshpass` and `rsync` installed locally
- **Remote credentials:** Default target is `123.57.182.243` (root/Happytest123_) — override via environment variables if needed

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

# On remote server:
ssh root@123.57.182.243
systemctl status web-server
systemctl status nginx
curl -fsS http://127.0.0.1:3100/
curl -fsS http://127.0.0.1:3100/api/projects
```

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
| `REMOTE_PASS` | `Happytest123_` | SSH password |
| `BACKEND_ORIGIN` | `http://127.0.0.1:3100` | Backend URL for nginx proxy |

## Retry Behavior

All deployment scripts include automatic retry logic with exponential backoff (5 attempts, starting at 2s delay) for:
- SSH authentication and remote command execution
- rsync/scp file transfers
- Remote health check verification

This handles intermittent SSH auth failures and network instability without manual intervention.

## Additional Documentation

- [Complete GitHub Artifact Deployment Guide](docs/GITHUB_ARTIFACT_DEPLOYMENT.md)
- [Deployment Implementation Details](DEPLOYMENT_IMPLEMENTATION_SUMMARY.md)
