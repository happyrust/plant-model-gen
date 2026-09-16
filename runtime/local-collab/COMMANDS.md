```powershell
# 本机双站点 smoke · 启动顺序（每条长驻命令各开一个终端）· SQLite-only 中继模式，不需要 SurrealDB
# 生成时间 2026-09-15 22:23:34 · 后端根目录 D:\work\plant-code\plant-model-gen
# 工程：每站一份副本 <site>/project/（SCB） · 演练文件 scb6000_0001

# 0. 前置（缺什么装什么，装完重开终端）
#    winget install --id EclipseFoundation.Mosquitto -e
#    cd D:\work\plant-code\plant-model-gen; cargo build --bin web_server --features web_server,relay-sync

# 1. MQTT broker（127.0.0.1:1883）
#    winget 装的 Mosquitto 会以服务 mosquitto 常驻 127.0.0.1:1883（local-only 模式、允许匿名），服务在跑就直接用；
#    要用下面这份自己的配置（stdout 出日志）先 Stop-Service mosquitto，否则 1883 绑不上。
Get-Service mosquitto -ErrorAction SilentlyContinue | Select-Object Status, StartType
powershell -ExecutionPolicy Bypass -File "D:\work\plant-code\plant-model-gen\runtime\local-collab\start-mosquitto.ps1"

# 2. Site A（:4100 · local-a · 中继 · 自有库 [6000]）
powershell -ExecutionPolicy Bypass -File "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-a\start.ps1"

# 3. Site B（:4101 · local-b · 中继 · 自有库 []）
powershell -ExecutionPolicy Bypass -File "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-b\start.ps1"

# 4. 验证两站都起来了、身份不同、CBA 目录可达
curl http://127.0.0.1:4100/api/site/identity
curl http://127.0.0.1:4101/api/site/identity
curl http://127.0.0.1:4101/files/output/metadata.json
curl http://127.0.0.1:4100/assets/archives/

# 5. API 级双站点 smoke（LS-01–LS-24；LS-23/24 = A 回退水位重广播 scb6000_0001 → B clone + 校验）
cd D:\work\plant-code\plant-collab-monitor
powershell -ExecutionPolicy Bypass -File scripts/local-remote-collab-smoke.ps1 `
  -SiteABase http://127.0.0.1:4100 -SiteBBase http://127.0.0.1:4101 `
  -FixtureDir "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-b\output" -MosquittoDir "C:\Program Files\mosquitto" `
  -SiteASqlite "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-a\deployment_sites.sqlite" -SiteBSqlite "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-b\deployment_sites.sqlite" `
  -RelayFileA "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-a\project\SCB\scb000\scb6000_0001" -RelayFileB "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-b\project\SCB\scb000\scb6000_0001"

# 6. 监控台 UI 闭环（LF-00–LF-08，对 Site A；会改写 site-a/DbOption.toml 并重启其 watcher + MQTT）
node scripts/topology-deploy-live-smoke.mjs --api http://127.0.0.1:4100 --mode full --confirm-writes

# 7. 手工看 UI
$env:VITE_API_TARGET = 'http://127.0.0.1:4100'; npm run dev    # → http://localhost:4000/topology

# 8. 看两站台账（中继模式的产物都在 SQLite）
sqlite3 -header -column "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-a\deployment_sites.sqlite" "SELECT direction, file_name, verify_status, diff_status, sesno_from, sesno_to, sesno_seen FROM e3d_sync_ledger ORDER BY created_at DESC LIMIT 10;"
sqlite3 -header -column "D:\work\plant-code\plant-model-gen\runtime\local-collab\site-b\deployment_sites.sqlite" "SELECT direction, file_name, verify_status, sesno_to, sesno_seen, verify_detail FROM e3d_sync_ledger ORDER BY created_at DESC LIMIT 10;"
```
