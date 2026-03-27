# Deployment Checklist

## Pre-Deployment

- [ ] Changes merged to target branch (main/ci-multi-platform-build)
- [ ] GitHub Actions workflow completed successfully
- [ ] Linux artifact (`linux-x64-release`) available
- [ ] GitHub CLI (`gh`) installed and authenticated
- [ ] Server credentials available (REMOTE_PASS)
- [ ] Frontend ready if using deploy_all_with_frontend.sh

## Get Deployment Artifacts

### Option A: From Recent Workflow Run
```bash
# List recent successful runs
gh run list --workflow multi-platform-build.yml --status success --limit 5

# Note the RUN_ID you want to deploy
```

### Option B: From Tagged Release
```bash
# List releases
gh release list --repo happyrust/plant-model-gen

# Note the TAG you want to deploy
```

## Deployment Commands

### Deploy Everything (Backend + Frontend)

**From GitHub Artifact:**
```bash
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=<YOUR_RUN_ID> \
./shells/deploy_all_with_frontend.sh
```

**From GitHub Release:**
```bash
BINARY_SOURCE=github-release \
GITHUB_TAG=<YOUR_TAG> \
./shells/deploy_all_with_frontend.sh
```

### Deploy Backend Only

**From GitHub Artifact:**
```bash
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=<YOUR_RUN_ID> \
./shells/deploy_web_server_bundle.sh
```

**From GitHub Release:**
```bash
BINARY_SOURCE=github-release \
GITHUB_TAG=<YOUR_TAG> \
./shells/deploy_web_server_bundle.sh
```

## Post-Deployment Verification

The scripts automatically verify:
- [ ] Systemd service `web-server` is active
- [ ] Active binary `/root/web_server` has been refreshed from the selected CI artifact
- [ ] Nginx is active
- [ ] Health endpoint responds: http://127.0.0.1:8080/
- [ ] API endpoint responds: http://127.0.0.1:8080/api/projects
- [ ] Public endpoint responds: http://123.57.182.243/
- [ ] Public API responds: http://123.57.182.243/api/projects

## Manual Verification (Optional)

```bash
# SSH to server
sshpass -p "$REMOTE_PASS" ssh -o StrictHostKeyChecking=no root@123.57.182.243

# Check service status
systemctl status web-server

# Confirm the active binary was updated
ls -lh /root/web_server

# View recent logs
journalctl -u web-server -n 50

# Test local endpoints
curl http://127.0.0.1:8080/api/projects
curl http://127.0.0.1:3100/api/version
```

## Rollback (If Needed)

Deploy a previous successful build:

```bash
# Find previous successful run
gh run list --workflow multi-platform-build.yml --status success --limit 10

# Deploy older run
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=<PREVIOUS_RUN_ID> \
./shells/deploy_all_with_frontend.sh
```

## Troubleshooting

### "Binary not found in artifact"
- Verify artifact name: `gh run view <RUN_ID>`
- Check build completed successfully
- Ensure using `linux-x64-release` for production

### "GITHUB_RUN_ID is required"
- Set the environment variable: `export GITHUB_RUN_ID=12345678`
- Or inline: `GITHUB_RUN_ID=12345678 ./shells/deploy_...`

### "gh: command not found"
```bash
# Install GitHub CLI
# macOS: brew install gh
# Linux: https://github.com/cli/cli/blob/trunk/docs/install_linux.md

# Authenticate
gh auth login
```

### Service fails to start
```bash
# Check logs
ssh root@123.57.182.243 "journalctl -u web-server -n 100"

# Check binary permissions
ssh root@123.57.182.243 "ls -l /root/web_server"

# Verify DbOption.toml
ssh root@123.57.182.243 "cat /root/DbOption.toml"
```

## Quick Reference

| Method | Command Variable | Required Value |
|--------|-----------------|----------------|
| Local Build | `BINARY_SOURCE=local` | None (default) |
| GitHub Artifact | `BINARY_SOURCE=github-artifact` | `GITHUB_RUN_ID=<id>` |
| GitHub Release | `BINARY_SOURCE=github-release` | `GITHUB_TAG=<tag>` |

## Documentation

- Full Guide: [docs/GITHUB_ARTIFACT_DEPLOYMENT.md](docs/GITHUB_ARTIFACT_DEPLOYMENT.md)
- Quick Reference: [QUICK_DEPLOY.md](QUICK_DEPLOY.md)
- Implementation Details: [DEPLOYMENT_IMPLEMENTATION_SUMMARY.md](DEPLOYMENT_IMPLEMENTATION_SUMMARY.md)
