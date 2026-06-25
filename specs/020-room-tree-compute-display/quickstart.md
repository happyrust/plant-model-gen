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
