# GitHub Artifact Deployment Guide

This guide explains how to deploy the web_server backend to production using GitHub-built binaries instead of local builds.

## Overview

The CI/CD pipeline has been configured to build a deploy-ready Linux artifact with all production features enabled. This eliminates the need to build Rust binaries locally when deploying to the production server.

## Build Artifacts

### Linux Deploy-Ready Artifact

The GitHub Actions workflow builds a special **linux-x64-release** artifact with the full production feature set:

**Features:** `ws,gen_model,manifold,project_hd,surreal-save,sqlite-index,web_server,parquet-export`

This matches the default feature set expected by the deployment scripts.

### Other Platform Artifacts

The workflow also builds verification artifacts for macOS and Windows with a minimal feature set for CI testing purposes only:

- `macos-x64-release`
- `macos-arm64-release`  
- `windows-x64-release`

**Features:** `ws,sqlite-index,surreal-save,web_server` (minimal set, not deploy-ready)

## Deployment Methods

### Method 1: Deploy from GitHub Actions Artifact (Recommended for CI/CD)

Use this when deploying from a specific workflow run:

```bash
# Get the run ID from GitHub Actions UI or CLI
gh run list --limit 5

# Deploy using that run's artifact
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=12345678 \
ARTIFACT_NAME=linux-x64-release \
./shells/deploy_all_with_frontend.sh
```

**When to use:**
- Deploying from a specific CI build
- Testing a PR build before merging
- Rolling back to a previous build

### Method 2: Deploy from GitHub Release

Use this when deploying from a tagged release:

```bash
# Deploy from a specific version tag
BINARY_SOURCE=github-release \
GITHUB_TAG=v1.2.3 \
./shells/deploy_all_with_frontend.sh
```

**When to use:**
- Deploying official releases
- Production deployments
- Version-controlled deployments

### Method 3: Deploy from Local Build (Default)

Traditional local build and deploy:

```bash
# Build locally and deploy
./shells/deploy_all_with_frontend.sh

# Or explicitly
BINARY_SOURCE=local ./shells/deploy_all_with_frontend.sh
```

**When to use:**
- Local development
- Testing local changes
- When GitHub Actions is unavailable

## Backend-Only Deployment

If you only need to deploy the backend without the frontend:

```bash
# From GitHub artifact
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=12345678 \
./shells/deploy_web_server_bundle.sh

# From GitHub release
BINARY_SOURCE=github-release \
GITHUB_TAG=v1.2.3 \
./shells/deploy_web_server_bundle.sh
```

## Environment Variables

### Required for All Methods

- `REMOTE_HOST` - Target server IP (default: 123.57.182.243)
- `REMOTE_USER` - SSH user (default: root)
- `REMOTE_PASS` - SSH password (default: Happytest123_)

### Required for GitHub Artifact Method

- `BINARY_SOURCE=github-artifact`
- `GITHUB_RUN_ID` - The workflow run ID
- `ARTIFACT_NAME` - Artifact name (default: linux-x64-release)

### Required for GitHub Release Method

- `BINARY_SOURCE=github-release`
- `GITHUB_TAG` - The release tag (e.g., v1.2.3)

### Optional

- `BUILD_BINARY` - Whether to build locally (default: true for local source)
- `SERVICE_NAME` - Systemd service name (default: web-server)
- `DB_OPTION_FILE` - Path to DbOption.toml

## Finding GitHub Run IDs

### Using GitHub CLI

```bash
# List recent workflow runs
gh run list --workflow multi-platform-build.yml --limit 10

# Get run ID for a specific commit
gh run list --commit abc123def

# View run details
gh run view 12345678
```

### Using GitHub Web UI

1. Go to repository → Actions tab
2. Click on the "Multi-Platform Build" workflow
3. Click on a specific run
4. The run ID is in the URL: `https://github.com/happyrust/plant-model-gen/actions/runs/[RUN_ID]`

## Verification

After deployment, the script automatically verifies:

1. Systemd service is active
2. Health check endpoints respond:
   - `http://127.0.0.1:8080/`
   - `http://127.0.0.1:8080/api/projects`
3. Remote endpoints accessible:
   - `http://123.57.182.243/`
   - `http://123.57.182.243/api/projects`

## Build Info

Each artifact includes a `BUILD_INFO.txt` file with:

- Build date and time
- Git commit hash
- Git branch name
- Target platform
- Enabled features
- Deploy-ready status

The deployment script displays this information during deployment.

## Troubleshooting

### Artifact Not Found

```
Error: Binary not found in artifact
```

**Solution:** Verify the artifact name and run ID are correct. Use `gh run view [RUN_ID]` to see available artifacts.

### Missing Features

If the deployed binary is missing features, check the BUILD_INFO.txt:

```bash
# Download and inspect artifact locally
gh run download 12345678 -n linux-x64-release
cat linux-x64-release/BUILD_INFO.txt
```

### GitHub Authentication

The deployment script requires GitHub CLI (`gh`) to be authenticated:

```bash
# Login to GitHub CLI
gh auth login

# Verify authentication
gh auth status
```

## CI/CD Integration Example

```yaml
# .github/workflows/deploy-production.yml
name: Deploy to Production

on:
  push:
    tags:
      - 'v*.*.*'

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Wait for build
        run: |
          RUN_ID=$(gh run list --workflow multi-platform-build.yml --commit ${{ github.sha }} --json databaseId --jq '.[0].databaseId')
          gh run watch $RUN_ID
      
      - name: Deploy to production
        env:
          REMOTE_PASS: ${{ secrets.PROD_SSH_PASSWORD }}
        run: |
          RUN_ID=$(gh run list --workflow multi-platform-build.yml --commit ${{ github.sha }} --json databaseId --jq '.[0].databaseId')
          BINARY_SOURCE=github-artifact \
          GITHUB_RUN_ID=$RUN_ID \
          ./shells/deploy_all_with_frontend.sh
```

## Summary

- **Linux deploy-ready artifact** is built with full production features
- **Three deployment methods** supported: artifact, release, local
- **Automatic verification** ensures successful deployment
- **Build info** provides traceability
- **Preserves local deploy path** while adding remote artifact capability
