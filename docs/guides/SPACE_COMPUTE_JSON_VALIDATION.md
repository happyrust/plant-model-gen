# 空间计算 web_server JSON 验证指南

## 目标

RUS-233 作为总览 issue，当前统一覆盖这 4 条空间计算链路：

- `RUS-229`：`POST /api/space/wall-distance`
- `RUS-230`：`POST /api/space/fitting-offset`
- `RUS-231`：`POST /api/space/fitting`
- `RUS-232`：`POST /api/space/tray-span`

验证方式不走 Rust `test`，统一走：

- 真实启动 `web_server`
- 读取 JSON fixture
- 逐条 `POST` 并校验响应

---

## Fixture 位置

```text
verification/space/compute/web_server_validation.json
```

当前基线包含：

1. `24383/89904 -> /api/space/fitting`
2. `24383/88342 -> /api/space/fitting-offset`
3. `17496/172026 -> /api/space/fitting-offset` 无命中
4. `24383/88342 -> /api/space/wall-distance`（S2）
5. `15207/2636 -> /api/space/wall-distance`（S1）
6. `24383/87412 -> /api/space/tray-span` 当前稳定基线
7. `24383/86525 -> /api/space/tray-span` issue 目标样例（当前记为已知差距）

---

## 启动 web_server

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen
WEB_SERVER_PORT=3185 cargo run --bin web_server --features web_server
```

---

## 执行 JSON 验证

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen
python3 scripts/verify-space-compute-api.py \
  --base-url http://127.0.0.1:3185 \
  --input verification/space/compute/web_server_validation.json
```

也可以直接用环境变量：

```bash
cd /Volumes/DPC/work/plant-code/plant-model-gen
BASE_URL=http://127.0.0.1:3185 python3 scripts/verify-space-compute-api.py
```

---

## 校验规则

当前 fixture 支持：

- `status`
- `message`
- `data_null`
- `equals`
- `approx`
- `gt`
- `expected_failure`
- `expected_failure_reason`

示例：

```json
{
  "case_id": "rus-230-fitting-offset-24383-88342",
  "endpoint": "/api/space/fitting-offset",
  "request": {
    "suppo_refno": "24383/88342"
  },
  "expect": {
    "status": "success",
    "equals": {
      "data.anchor_kind": "S2"
    },
    "approx": {
      "data.vector.dx": {
        "value": 501.0625,
        "tolerance": 0.01
      }
    }
  }
}
```

---

## 已知差距记录方式

当 issue 里已经有明确目标值，但当前 `web_server` 还没满足时，不删除 case，直接在 fixture 里标记：

```json
{
  "case_id": "rus-232-tray-span-24383-86525-issue-target",
  "expected_failure": true,
  "expected_failure_reason": "当前响应仍是 no tray matched，尚未满足 issue 样例 [1100.0, 1600.0]"
}
```

这样做的目的：

1. 真实目标不会丢
2. 脚本仍可持续执行
3. 一旦接口修好，`expected_failure` case 会变成提醒项，提示及时更新 fixture

---

## 维护原则

1. 只放真实业务基线 case
2. 期望值优先来自 Linear issue、现有回归样例和真实 POST 结果
3. 新增 case 时，先手工 POST 确认，再写入 fixture
4. 若响应结构变更，先更新脚本支持，再改 fixture
