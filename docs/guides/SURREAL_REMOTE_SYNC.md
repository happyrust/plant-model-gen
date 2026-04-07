# SurrealDB 同步到 Ubuntu 服务器（说明）

## 后端「数据库」指什么

`plant-model-gen` 的 `web_server`（`/api/e3d/*`、图查询等）主要依赖 **SurrealDB**，连接信息来自部署到服务器的 `/root/DbOption.toml`（由 `shells/deploy/deploy_web_server_bundle.sh` 上传）。

- **网络访问**：浏览器里 `plant3d-web` 的 `.env.production` 使用 `VITE_SURREAL_URL=ws://<公网IP>:8020`，因此服务器上需要 **Surreal 监听 `0.0.0.0:8020`**（或前面加反代，本文按直连 8020）。
- **数据落盘**：Mac 上 `DbOption-mac.toml` 使用 RocksDB 存储目录（不是单文件）：
  - `path = "/Volumes/DPC/work/db-data/ams-8020.db"`（目录）
  - 若启用 SurrealKV，还可能有 `ams-8020.db.kv`（可选一并同步）

**仅部署 `web_server` + `output/` 不会带上 Surreal 里的 MDB/WORL/pe 等表**；若服务器上 Surreal 为空或未启动，会出现此前看到的 `get_world_refno failed` 一类错误。

## 同步策略（推荐）

1. **停库再拷**：复制前在远端 **停止** 占用 8020 的 Surreal 进程，避免 RocksDB 半写导致损坏。
2. **整目录 rsync**：本地 `ams-8020.db` 是目录，用 `rsync -a` 同步到远端固定路径（脚本默认 `/root/surreal_data/ams-8020.db`）。
3. **可选 `.kv`**：若本地存在 `ams-8020.db.kv`，一并同步到 `/root/surreal_data/`。
4. **拉起服务**：使用 **systemd** 常驻（脚本可写入 `surreal-8020.service`），保证重启后仍在。
5. **远端需安装 Surreal 二进制**：Ubuntu 上需有与数据兼容的 `surreal` 版本（`PATH` 中或设置 `REMOTE_SURREAL_BIN`）。

## 生产服务器固定监听 8020

### 与本地一致：RocksDB + `surreal-8020`（推荐用于 Aveva / MDB 等完整库）

本地 `ams-8020.db` 为 **RocksDB 目录**时，线上应使用 **`surreal-8020.service`**（`rocksdb:///root/surreal_data/ams-8020.db`），**不要**与仅含轻量数据的 SurrealKV 混用。

1. 同步数据（停库 → rsync → 启动）：

```bash
REMOTE_HOST=123.57.182.243 REMOTE_USER=root REMOTE_PASS='...' \
  REMOTE_SURREAL_BIN=/usr/local/bin/surreal \
  ./shells/sync_surreal_8020_to_remote.sh
```

2. 若数据已拷好，只需安装/切换 systemd 为 Rocks 并重启：

```bash
REMOTE_HOST=123.57.182.243 REMOTE_USER=root REMOTE_PASS='...' \
  ./shells/apply_surreal_rocks_8020_remote.sh
```

会 **disable** `surrealdb.service`（surrealkv），启用 **`surreal-8020.service`**，与 `shells/systemd/surreal-8020-rocksdb.service` 一致。

### 仅轻量 / 空库：SurrealKV + `surrealdb.service`

若使用 **`/var/lib/surrealdb/data`** 的 SurrealKV：

```bash
REMOTE_HOST=123.57.182.243 REMOTE_USER=root REMOTE_PASS='...' \
  ./shells/apply_surrealdb_8020_remote.sh
```

会安装 `/root/shells/run_surrealdb_kv_8020.sh` 与 `/etc/systemd/system/surrealdb.service`，并重启 `surrealdb`、`web-server`。**注意**：与 RocksDB 路线 **互斥**（争用 8020），同一时刻只应启用其一。

## 一键脚本（RocksDB 整库 rsync）

在 **plant-model-gen 仓库根目录**执行（与 `shells/deploy/deploy_web_server_bundle.sh` 相同，使用环境变量传密码，勿把密码写进仓库）：

```bash
# 使用默认本地路径（Mac）
REMOTE_HOST=123.57.182.243 REMOTE_USER=root REMOTE_PASS='你的密码' \
  ./shells/sync_surreal_8020_to_remote.sh

# 自定义本地库目录、远端路径
LOCAL_SURREAL_DB=/path/to/ams-8020.db \
REMOTE_SURREAL_DATA_DIR=/root/surreal_data \
REMOTE_HOST=... REMOTE_USER=root REMOTE_PASS='...' \
  ./shells/sync_surreal_8020_to_remote.sh
```

首次运行会安装 `/etc/systemd/system/surreal-8020.service` 并 `enable --now`。再次运行会先 `stop` 再 rsync 再 `start`。

脚本里 **rsync** 默认使用 `-az --delete --partial --info=progress2`（压缩、增量、断点续传、整体进度）。仅重启、不再同步时：

```bash
RESTART_ONLY=1 REMOTE_HOST=... REMOTE_USER=root REMOTE_PASS='...' ./shells/sync_surreal_8020_to_remote.sh
```

## 与 `DbOption.toml` 的关系

- `web_server` 在同一台机访问 Surreal 时，一般为 `surreal_ip = "127.0.0.1"`、`surreal_port = 8020`。
- 仓库里的 `db_options/DbOption.toml` / `DbOption-mac.toml` 可能仍是本机路径；部署时由 `shells/deploy/apply_dboption_deploy_paths.py` 按 `REMOTE_PROJECT_PATH`、`REMOTE_SURREAL_DATA_PATH`、`REMOTE_SURREALKV_DATA_PATH`、`REMOTE_MESHES_PATH`、`REMOTE_SURREAL_SCRIPT_DIR` 等写入对应键。**Surreal 数据路径**若写在 `[surrealdb].path` 且用于本机嵌入式打开，需与远端实际 rocksdb 路径一致；当前架构以 **独立 `surreal start` + 网络连接** 为主时，关键是 **8020 上有正确数据**。

## 体积与耗时

本地 `ams-8020.db` 目录常在 **数 GB** 量级，首次同步取决于上行带宽；可重复执行脚本做增量 rsync。

## 安全

- 不要把 `REMOTE_PASS` 写进 Git；用环境变量或 CI Secret。
- 生产环境建议 SSH 密钥 + 防火墙仅开放必要端口。
