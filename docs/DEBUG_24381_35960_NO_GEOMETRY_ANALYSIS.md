# 24381/35960 "无几何可导出" 根因分析

## 现象

```
⚠️  跳过导出：无几何可导出 -> output\AvevaMarineSample/24381_35960.obj
```

## 因果链

```
① lod_L1 目录不存在
    ↓
② 生成 GLB 失败: 创建文件失败 ./assets/meshes\lod_L1\3080706177900460082_L1.glb
    ↓
③ 布尔运算 load_manifold 失败: 正实体 mesh 文件找不到
    ↓
④ 布尔运算失败，未生成 CatePos 结果 mesh (24381_35960_L1.glb)
    ↓
⑤ 导出时找不到 24381_35960 的几何体 → 跳过导出
```

## 根因

**`assets/meshes/lod_L1` 目录不存在**，导致：
- PANE 的 GLB (`3080706177900460082_L1.glb`) 写入失败
- 布尔运算无法加载正实体 manifold
- 无 CatePos 结果，导出无可用的 geo_hash

## 解决方案

**首次运行前确保目录存在**：

```powershell
cd D:\work\plant-code\plant-model-gen
New-Item -ItemType Directory -Force -Path assets/meshes/lod_L1
```

或由启动脚本/安装流程自动创建该目录。

## 验证

修复后重新运行：
```powershell
cargo run --bin aios-database -- --debug-model 24381/35960 --regen-model --export-obj -v
```

预期：
- `24381_35960_L1.glb` 生成于 `assets/meshes/lod_L1/`
- `✅ 导出成功: output\AvevaMarineSample/24381_35960.obj`
