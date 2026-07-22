# Quickstart: Validate Room Tree Compute And Display

## Prerequisites

- Use `D:\work\plant-code\plant-model-gen-cata-closure` as the backend workspace.
- Use `D:\work\plant-code\plant3d-web` as the frontend workspace.
- Pick a site/model database where room compute has already populated `room_relate`.
- Do not run `cargo test` for `web_server`.

## 1. Confirm Backend Routes Are Mounted

Start the backend with route printing when available:

```powershell
$env:AIOS_PRINT_ROUTES = "1"
# Start the existing web_server command used by this project/site.
```

Expected route list includes:

```text
GET    /api/room-tree/root
GET    /api/room-tree/children/{id}
GET    /api/room-tree/ancestors/{id}
POST   /api/room-tree/search
```

## 2. Validate Root

```powershell
Invoke-RestMethod -Method GET -Uri "http://localhost:<port>/api/room-tree/root" | ConvertTo-Json -Depth 8
```

Expected:

- `success = true`
- `node.id = room-root`
- `node.noun = ROOM_ROOT`

## 3. Validate Room Groups

```powershell
Invoke-RestMethod -Method GET -Uri "http://localhost:<port>/api/room-tree/children/room-root?limit=2000" | ConvertTo-Json -Depth 12
```

Expected when room data exists:

- `success = true`
- `children[]` contains `ROOM_GROUP` nodes.
- Each group ID starts with `room-group:`.

If no room compute has run yet, record the actual empty/error response. This is not a display wiring failure by itself.

## 4. Validate One Room Path

Pick one returned room group ID and URL-encode it:

```powershell
$group = [uri]::EscapeDataString("room-group:<group>")
Invoke-RestMethod -Method GET -Uri "http://localhost:<port>/api/room-tree/children/$group?limit=2000" | ConvertTo-Json -Depth 12
```

Expected:

- `children[]` contains `ROOM` nodes.
- Room nodes have refno-like IDs after frontend normalization.

Pick one room ID:

```powershell
Invoke-RestMethod -Method GET -Uri "http://localhost:<port>/api/room-tree/children/<room-refno>?limit=2000" | ConvertTo-Json -Depth 12
```

Expected:

- `children[]` contains `COMP_GROUP` nodes.
- Group IDs start with `comp-group:<room-refno>:`.

Pick one component group ID and URL-encode it:

```powershell
$compGroup = [uri]::EscapeDataString("comp-group:<room-refno>:BRAN")
Invoke-RestMethod -Method GET -Uri "http://localhost:<port>/api/room-tree/children/$compGroup?limit=2000" | ConvertTo-Json -Depth 12
```

Expected:

- `children[]` contains component leaf nodes.
- Leaf `children_count` is `0`.

## 5. Validate Search

```powershell
Invoke-RestMethod `
  -Method POST `
  -Uri "http://localhost:<port>/api/room-tree/search" `
  -ContentType "application/json" `
  -Body (@{ keyword = "<room-keyword>"; limit = 50 } | ConvertTo-Json) |
  ConvertTo-Json -Depth 12
```

Expected:

- `success = true`
- `items[]` contains `ROOM` nodes when the keyword matches a room group/name/code.

## 6. Validate Ancestors

Use a known room-contained component refno from the component group response:

```powershell
Invoke-RestMethod -Method GET -Uri "http://localhost:<port>/api/room-tree/ancestors/<component-refno>" | ConvertTo-Json -Depth 12
```

Expected:

- `success = true`
- `ids[]` contains a path ending with `room-root`.
- Path includes a room refno and a `room-group:` node.

## 7. Validate `plant3d-web`

Start or open `../plant3d-web` against the same backend:

- Same-origin/proxy dev mode is acceptable.
- Alternatively use `?backendPort=<port>` or `?backend=http://localhost:<port>` when supported.

Smoke steps:

1. Open the model tree panel.
2. Switch from PDMS tree to room tree.
3. Confirm root and room groups appear.
4. Expand group -> room -> component group -> component.
5. Search by room keyword and select a result.
6. Use visibility, isolate/xray, and fly-to on a loaded room subtree.
7. Switch back to PDMS tree and confirm PDMS tree state still works.

## 8. Missing Room Compute Check

Repeat root/children calls against a site without `room_relate` rows.

Expected:

- Backend response is empty or `success=false` with clear `error_message`.
- Frontend room tab does not corrupt PDMS tree state.
- Operator knows to run room compute explicitly before expecting populated room tree contents.

## Appendix: 2026-07-16 HTTP smoke on `:8080`

Base: `http://127.0.0.1:8080`

| Step | Result |
|------|--------|
| `GET /api/room-tree/root` | **Pass** — `success=true`, `node.id=room-root`, `noun=ROOM_ROOT` |
| `GET /api/room-tree/children/room-root` | **Fail** — `success=false` after ~30s; `error_message`: `query_arch_room_groups failed: query timeout after 30s at D:\work\plant-code\rs-core\src\room\algorithm.rs:64:51` |
| `POST /api/room-tree/search` (`keyword=301`) | **Fail** — same 30s timeout via `query_arch_room_groups` |
| `GET /api/room-tree/ancestors/1_1` | **Inconclusive** — client canceled at 10s (likely same slow path) |

**Implication (updated)**: Route mounting and root contract are healthy. The 30s error is **not** a slow SurrealQL scan — it is the `SUL_DB` query-timeout wrapper converting a **TCP hang** into an observable error.

**Root cause (2026-07-16)**:

| Check | Result |
|-------|--------|
| Running `web_server` | `plant-model-gen-cata-closure` · `--config runtime/admin_sites/avevamarinesample/quicktest-7997-8080/DbOption` · `:8080` |
| Configured Surreal | `surreal_ip=198.18.0.1` · `surreal_port=8021` · user `siteadmin7997` |
| Surreal process | Listening on `198.18.0.1:8021` (rocksdb site data path) |
| Local TCP to `198.18.0.1:8021` | **TIMEOUT** (SYN_SENT); `127.0.0.1:8021` refused (not bound there) |
| `web_server` sockets | Multiple `SYN_SENT` to `198.18.0.1:8021` |

So `/api/room-tree/root` works (no SUL_DB), while children/search/ancestors hang until the 30s `query_ext` timeout.

**Fix applied (2026-07-16, local ops)**:

1. Patched site `DbOption.toml`: `surreal_ip` / `surreal_bind` → `127.0.0.1:8021`
2. Restarted Surreal bound to `127.0.0.1:8021` (same rocksdb path)
3. Restarted `web_server` with the same config

**After reconnect**:

| Step | Result |
|------|--------|
| `GET .../children/room-root` | Fast fail (~15ms): `table 'room_panel_relate' does not exist` |
| `POST .../search` | Fast fail (~9ms): same |
| Surreal CLI `SELECT count() FROM pe` | **174194** rows — model data present |
| `room_relate` / `room_panel_relate` | **tables absent** — room compute not run on this site |

**Next for T015–T018**: run room compute for `AvevaMarineSample` / dbnum `7997` on this site, then re-smoke children/search/ancestors.
