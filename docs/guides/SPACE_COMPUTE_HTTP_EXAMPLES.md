# 空间计算接口 HTTP 示例

> 适用于 `web_server` 本地联调。当前推荐直接传完整 `suppo_refno`，由后端自行映射 `dbnum`。兼容旧请求时，`dbnum + 低位 suppo_refno` 仍可继续使用。

## 启动

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen
WEB_SERVER_PORT=3182 cargo run --bin web_server --features web_server
```

## 1. 支架对应预埋板

### `POST /api/space/fitting`

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/fitting \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/89904"}' | jq '.'
```

当前样例要点：

- `panel_refno = "25688/47628"`
- `fitting = "1RS02TT0265P"`
- `match_method = "direct_contact"`

## 2. 支架与预埋板相对定位

### `POST /api/space/fitting-offset`

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/fitting-offset \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/88342"}' | jq '.'
```

当前样例要点：

- `vector.dx ≈ 501.0625`
- `vector.dy ≈ -174.7295`
- `vector.dz = 122.0`

无命中样例：

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/fitting-offset \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"17496/172026"}' | jq '.'
```

预期：

- `status = "success"`
- `data = null`
- `message = "no panel matched"`

## 3. 支架距墙/定位块

### `POST /api/space/wall-distance`

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/wall-distance \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/88342","suppo_type":"S2","search_radius":5000}' | jq '.'
```

当前样例要点：

- `anchor_kind = "S2"`
- `target.noun = "GWALL"`
- `target.refno = "17496/118997"`

S1 样例：

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/wall-distance \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"15207/2636","suppo_type":"S1","search_radius":5000}' | jq '.'
```

## 4. 支架对应桥架

### `POST /api/space/suppo-trays`

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/suppo-trays \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/89904"}' | jq '.'
```

当前样例要点：

- `anchor_kind = "S2"`
- 首个命中 `bran_refno = "24383/100128"`
- 首个命中 `tray_section_refno = "24383/100129"`

## 5. 支架与钢结构相对定位

### `POST /api/space/steel-relative`

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/steel-relative \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/89904","search_radius":8000}' | jq '.'
```

当前样例要点：

- `steel_refno = "24383/87412"`
- `steel_noun = "STRU"`
- `within = true`

## 6. 支架跨度

### `POST /api/space/tray-span`

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/tray-span \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/87412","neighbor_window":5000}' | jq '.'
```

当前样例要点：

- `bran_refno = "24383/100128"`
- `left_suppo_refno = "24383/87389"`
- `right_suppo_refno = "24383/87433"`

另一个样例：

```bash
curl -sS -X POST http://127.0.0.1:3182/api/space/tray-span \
  -H 'Content-Type: application/json' \
  -d '{"suppo_refno":"24383/89904","neighbor_window":5000}' | jq '.'
```

当前样例要点：

- `bran_refno = "24383/100128"`
- `left_suppo_refno = "24383/87389"`
- `right_suppo_refno = "24383/87412"`
