# 房间计算 CLI + JSON 验证指南

## 目标

房间计算的验收与回归，统一走：

- **CLI 命令**
- **JSON fixture**

不再以 Rust `test` 作为主验证入口。

---

## 推荐入口

### 1. 一键脚本

推荐直接使用：

```powershell
.\scripts\verify-room-compute.ps1 -DbNums 24383
```

默认行为：

1. `room clean`
2. `room compute`
3. `room verify-json`

默认 fixture：

```text
verification/room_compute_validation.json
```

默认关键词：

```text
-RM,-ROOM
```

---

## 脚本参数

### 最常用

- `-DbNums 24383,24384`
  - 指定房间计算范围，避免误跑全量
- `-RefnoRoot 24383_83477`
  - 按子树范围执行房间计算
- `-Fixture verification/room_compute_validation.json`
  - 指定校验 JSON
- `-ExportOutput output/room-export`
  - 计算并校验后，再导出房间结果 JSON

### 控制开关

- `-SkipClean`
  - 不清空现有 `room_relate / room_panel_relate`
- `-SkipCompute`
  - 不执行计算，只校验已有持久化结果
- `-SkipVerify`
  - 不做 JSON 校验
- `-GenPanelsMesh`
  - 预生成缺失面板 mesh
- `-Release`
  - 使用 `cargo run --release`
- `-DryRun`
  - 只打印将执行的命令，不真正运行

---

## 直接使用 CLI

如不走脚本，也可直接运行 CLI。

### 1. 清理旧结果

```powershell
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room clean
```

### 2. 按范围计算

#### 按 dbnum

```powershell
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room compute --keywords -RM,-ROOM --db-nums 24383
```

#### 按 refno 子树

```powershell
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room compute --keywords -RM,-ROOM --refno-root 24383_83477
```

### 3. 用 JSON fixture 校验

```powershell
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room verify-json --input verification/room_compute_validation.json
```

### 4. 导出结果供人工检查

```powershell
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room export --output output/room-export
```

---

## fixture 格式

`verification/room_compute_validation.json` 使用如下结构：

```json
{
  "description": "房间计算 CLI 验证基线",
  "test_cases": [
    {
      "case_id": "room-540-panel-24381-35798",
      "description": "示例说明",
      "room_number": "540",
      "panel_refno": "24381/35798",
      "expected_components": ["24381/145019"],
      "notes": "可选说明"
    }
  ]
}
```

字段含义：

- `room_number`
  - 目标房间号
- `panel_refno`
  - 该房间下需要验证的 panel
- `expected_components`
  - 该 panel 对应 `room_relate` 中必须命中的构件集合

---

## 建议维护方式

1. 先选取**真实业务基线 case**
2. 放入 `verification/room_compute_validation.json`
3. 每次房间计算链路调整后，跑脚本复验
4. 若业务基线扩充，继续追加 `test_cases`

建议优先纳入：

- 历史已知问题 panel
- scope rebuild 容易误删的 case
- 房间边界复杂、容易漏算的 case

---

## 注意事项

1. `room verify-json` 是**只读校验**
   - 不会重算
   - 不会写库
2. 若 `room_relate / room_panel_relate` 为空，校验会直接失败
3. 为避免误跑全量，脚本默认要求：
   - `-DbNums`
   - 或 `-RefnoRoot`
   - 或者显式 `-SkipCompute`

---

## 推荐命令

### 先看命令，不执行

```powershell
.\scripts\verify-room-compute.ps1 -DbNums 24383 -DryRun
```

### 做一次 scoped 真实验证

```powershell
.\scripts\verify-room-compute.ps1 -DbNums 24383 -ExportOutput output/room-export
```

### 仅校验已有结果

```powershell
.\scripts\verify-room-compute.ps1 -SkipClean -SkipCompute
```
