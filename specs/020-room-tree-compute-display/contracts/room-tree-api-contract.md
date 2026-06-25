# Contract: Room Tree API

## Base

All endpoints are served by the generated model backend under the frontend API base resolved by `getBackendApiBaseUrl()`.

## GET /api/room-tree/root

Returns the virtual root node.

### Response

```json
{
  "success": true,
  "node": {
    "id": "room-root",
    "name": "ROOM",
    "noun": "ROOM_ROOT",
    "owner": null,
    "children_count": null
  },
  "error_message": null
}
```

## GET /api/room-tree/children/{id}

Returns one lazy-loaded child level.

### Query Parameters

- `limit`: Optional integer. Backend clamps it to an allowed range.

### Root Children

`GET /api/room-tree/children/room-root?limit=2000`

Returns room group virtual nodes:

```json
{
  "success": true,
  "parent_id": "room-root",
  "children": [
    {
      "id": "room-group:301",
      "name": "301",
      "noun": "ROOM_GROUP",
      "owner": "room-root",
      "children_count": 12
    }
  ],
  "truncated": false,
  "error_message": null
}
```

### Room Group Children

`GET /api/room-tree/children/room-group%3A301?limit=2000`

Returns room refno nodes:

```json
{
  "success": true,
  "parent_id": "room-group:301",
  "children": [
    {
      "id": "2013286704_1035",
      "name": "R301",
      "noun": "ROOM",
      "owner": "room-group:301",
      "children_count": 128
    }
  ],
  "truncated": false,
  "error_message": null
}
```

### Room Children

`GET /api/room-tree/children/2013286704_1035?limit=2000`

Returns component group virtual nodes:

```json
{
  "success": true,
  "parent_id": "2013286704_1035",
  "children": [
    {
      "id": "comp-group:2013286704_1035:BRAN",
      "name": "BRAN",
      "noun": "COMP_GROUP",
      "owner": "2013286704_1035",
      "children_count": 48
    }
  ],
  "truncated": false,
  "error_message": null
}
```

### Component Group Children

`GET /api/room-tree/children/comp-group%3A2013286704_1035%3ABRAN?limit=2000`

Returns component refno leaf nodes:

```json
{
  "success": true,
  "parent_id": "comp-group:2013286704_1035:BRAN",
  "children": [
    {
      "id": "2013286704_476",
      "name": "/ZONE/PIPE/BRAN",
      "noun": "TUBI",
      "owner": "comp-group:2013286704_1035:BRAN",
      "children_count": 0
    }
  ],
  "truncated": false,
  "error_message": null
}
```

### Failure Response

Endpoint returns JSON with `success=false` when a node cannot be resolved:

```json
{
  "success": false,
  "parent_id": "bad-id",
  "children": [],
  "truncated": false,
  "error_message": "unknown node id: bad-id"
}
```

## GET /api/room-tree/ancestors/{id}

Returns a path from target toward root. Frontend reverses the list to attach and expand nodes.

### Room Response

```json
{
  "success": true,
  "ids": [
    "2013286704_1035",
    "room-group:301",
    "room-root"
  ],
  "error_message": null
}
```

### Component Response

```json
{
  "success": true,
  "ids": [
    "2013286704_476",
    "comp-group:2013286704_1035:BRAN",
    "2013286704_1035",
    "room-group:301",
    "room-root"
  ],
  "error_message": null
}
```

### Failure Response

```json
{
  "success": false,
  "ids": [],
  "error_message": "refno not found in room tree: 2013286704_476"
}
```

## POST /api/room-tree/search

Searches room group/name/code and returns room nodes.

### Request

```json
{
  "keyword": "R301",
  "limit": 50
}
```

### Response

```json
{
  "success": true,
  "items": [
    {
      "id": "2013286704_1035",
      "name": "R301",
      "noun": "ROOM",
      "owner": "room-group:301",
      "children_count": 0
    }
  ],
  "error_message": null
}
```

## Compatibility Notes

- `id`, `owner`, `parent_id`, and `ids[]` may serialize as plain strings, Surreal-style refno strings, arrays, or enum wrapper objects depending on `RefnoEnum` serialization. Frontend must normalize them through `normalizeRoomTreeId()`.
- Virtual node IDs are stable strings and must not be translated in the API.
- Search currently returns room nodes only.
- API reads existing room data; it must not start room compute.
