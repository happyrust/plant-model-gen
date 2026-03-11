# Quick Deploy Reference

## Deploy from GitHub Actions (Most Common)

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

```bash
./shells/deploy_all_with_frontend.sh
```

## Backend Only

```bash
# From artifact
BINARY_SOURCE=github-artifact GITHUB_RUN_ID=<RUN_ID> ./shells/deploy_web_server_bundle.sh

# From release
BINARY_SOURCE=github-release GITHUB_TAG=v1.2.3 ./shells/deploy_web_server_bundle.sh
```

## Full Example with Custom Server

```bash
REMOTE_HOST=123.57.182.243 \
REMOTE_USER=root \
REMOTE_PASS=Happytest123_ \
BINARY_SOURCE=github-artifact \
GITHUB_RUN_ID=12345678 \
./shells/deploy_all_with_frontend.sh
```

See [docs/GITHUB_ARTIFACT_DEPLOYMENT.md](docs/GITHUB_ARTIFACT_DEPLOYMENT.md) for detailed documentation.
