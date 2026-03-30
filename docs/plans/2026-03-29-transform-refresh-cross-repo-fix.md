# TG-107 管嘴错位跨仓修复说明

**日期**: 2026-03-29  
**状态**: 已修复、已推送、待 issue 归档  
**涉及仓库**:
- `happyrust/plant-model-gen`
- `happyrust/rs-core`

---

## 1. 问题现象

在 `DbOption-zsy` 配置下，针对设备 `21485_26 /TG-107` 做定向生成与导出时，4 个管嘴中：

- `21485_80`
- `21485_23195`
- `21485_23196`

位置正常；但：

- `21485_75 /TG-107油品进出口管嘴`

会明显偏离设备主体，表现为：

- `pe_transform.world_trans` 异常
- 导出的 `TG-107.obj` 中该 group 远离设备主体 bbox
- 截图中该管嘴与设备装配关系错误

---

## 2. 复现与验证样例

### 2.1 样例设备

- 设备：`21485_26 /TG-107`
- 管嘴：`21485_75 /TG-107油品进出口管嘴`

### 2.2 验证命令

```bash
./target/debug/aios-database \
  -c db_options/DbOption-zsy \
  --debug-model 21485_26 \
  --export-obj \
  --capture output/screenshots/equi_21485_26_after_fix2 \
  --capture-width 1200 \
  --capture-height 900 \
  --capture-views 4 \
  --use-surrealdb \
  -v
```

导出产物：

- OBJ: `output/YCYK-E3D/TG-107.obj`
- 截图：
  - `output/screenshots/equi_21485_26_after_fix2/TG-107.png`
  - `output/screenshots/equi_21485_26_after_fix2/TG-107_view02.png`
  - `output/screenshots/equi_21485_26_after_fix2/TG-107_view03.png`
  - `output/screenshots/equi_21485_26_after_fix2/TG-107_view04.png`

---

## 3. 根因拆分

这次问题不是单点 bug，而是两段式问题：

### 3.1 `plant-model-gen` 侧问题

refno 定向生成链路（`--debug-model` / `--regen-model`）在进入 `gen_all_geos_data()` 前，没有先对目标 root 子树做 `pe_transform` 一致性刷新。

结果会出现：

- 同一设备子树里混用旧缓存与新计算结果
- 某些 NOZZ/子件位置正确，某些位置错乱

### 3.2 `rs-core` 侧问题

即使重新计算出了正确的 transform，`save_pe_transform_entries()` 在写 `trans:*` 记录时使用的是：

```rust
INSERT IGNORE INTO trans ...
```

这会导致：

- 如果 `trans:*` 记录已存在但内容脏了
- refresh 重新算出的正确值不会覆盖旧值
- 于是表面上看“refresh 已执行”，实际装配位置仍然错误

---

## 4. 修复方案

### 4.1 `plant-model-gen`

#### 修改文件
- `src/pe_transform_refresh.rs`
- `src/cli_modes.rs`

#### 修复点
1. 新增按 roots 子树刷新的入口：
   - `refresh_pe_transform_for_root_refnos_compat()`
2. 新增沿 owner 链重算 root world transform 的逻辑：
   - `compute_world_mat_from_owner_chain()`
3. 在以下生成入口中，正式生成前先刷新目标子树：
   - `run_generate_model()`
   - `run_regen_model()`

#### 相关提交
- `cf48bce fix(transform): refresh refno subtree before generation`

---

### 4.2 `rs-core`

#### 修改文件
- `src/rs_surreal/pe_transform.rs`

#### 修复点
把：

```rust
INSERT IGNORE INTO trans ...
```

改为：

```rust
UPSERT trans:⟨hash⟩ SET d = ...
```

确保：

- 已存在但内容错误的 `trans:*` 记录会被覆盖
- 子树 refresh 的结果能够真实落库

#### 相关提交
- `5379a84 fix(transform): upsert cached trans records`

---

## 5. 真实回归结论

为了做强验证，这次使用了“故意造脏 + CLI 回归”的方式：

1. 先手动把 `21485_75` 对应 `trans:*` 的 translation 写坏为 `[1, 2, 3]`
2. 跑修复后的 `--debug-model 21485_26`
3. 再查库与导出 OBJ

### 5.1 回归后数据库恢复正确

`pe_transform:21485_75` 回到：

- `local_trans = [-7707.464, 7707.464, -7470]`
- `world_trans = [1085792.5, 1112707.5, 77000]`

### 5.2 回归后装配位置恢复正确

重新解析 `output/YCYK-E3D/TG-107.obj` 后，4 个 NOZZ 到设备主体 bbox 的距离均为：

- `21485_75 -> 0.000`
- `21485_80 -> 0.000`
- `21485_23195 -> 0.000`
- `21485_23196 -> 0.000`

这说明：

- 错位问题已经被真实修复
- 不只是日志层面“看起来执行了 refresh”

---

## 6. 后续建议

1. 后续所有 refno scoped generation / export 的装配问题，优先先看：
   - 子树 transform 是否已 refresh
   - `trans:*` 是否允许覆盖旧值
2. 对于 `DbOption-zsy` 默认未覆盖的 dbnum（如 `5101`），建议继续保留：
   - 定向子树 refresh 优先于整库 refresh
3. 若后续需要补 issue / PR / release note，可直接复用本文档内容
