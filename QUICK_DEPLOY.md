# Quick Deploy Reference

## Deploy from GitHub Actions (Recommended)

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

## Deploy from Local Build (Development)

### macOS → Linux Cross-Compilation (Default)

Local deployment now automatically uses `cargo zigbuild` on macOS to cross-compile for Linux x86_64:

```bash
# Builds for Linux automatically on macOS, deploys to remote server
./shells/deploy_all_with_frontend.sh
```

**Prerequisites:** `cargo install cargo-zigbuild` and [Zig toolchain](https://ziglang.org/download/)

### Force Native Build

```bash
# Build for native platform instead of cross-compiling
USE_ZIGBUILD=false ./shells/deploy_all_with_frontend.sh
```

### Backend Only

```bash
# From local build (macOS → Linux with zigbuild)
./shells/deploy_web_server_bundle.sh

# From artifact
BINARY_SOURCE=github-artifact GITHUB_RUN_ID=<RUN_ID> ./shells/deploy_web_server_bundle.sh

# From release
BINARY_SOURCE=github-release GITHUB_TAG=v1.2.3 ./shells/deploy_web_server_bundle.sh

# Force native build
USE_ZIGBUILD=false ./shells/deploy_web_server_bundle.sh
```

## Deployment Resilience

All deployment scripts now include automatic retry logic with exponential backoff (5 attempts) for:
- SSH authentication and command execution
- rsync/scp file transfers
- Remote health check verification

This ensures reliable deployment over unstable connections or during transient auth failures.

## Full Example with Custom Server

```bash
REMOTE_HOST=123.57.182.243 \
REMOTE_USER=root \
REMOTE_PASS=Happytest123_ \
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
| `REMOTE_HOST` | `123.57.182.243` | Target server IP |
| `REMOTE_USER` | `root` | SSH username |
| `REMOTE_PASS` | `Happytest123_` | SSH password |

## Additional Documentation

- [Complete GitHub Artifact Deployment Guide](docs/GITHUB_ARTIFACT_DEPLOYMENT.md)
- [Deployment Implementation Details](DEPLOYMENT_IMPLEMENTATION_SUMMARY.md)
