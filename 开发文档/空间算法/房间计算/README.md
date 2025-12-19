# 房间计算系统文档

## 概述

本目录包含 gen-model-fork 项目房间计算系统的完整文档。房间计算用于确定工厂设计中哪些构件（管道、设备等）位于特定房间（由面板围成的空间）内部。

## 文档索引

| 文件 | 说明 |
|------|------|
| [01_计算流程.md](./01_计算流程.md) | 详细计算流程、两阶段算法 |
| [02_数据模型.md](./02_数据模型.md) | 数据结构、数据库表关系 |
| [03_流程图.md](./03_流程图.md) | Mermaid 流程图 |
| [04_注意事项.md](./04_注意事项.md) | 已知问题、注意事项、问题排查 |

## 快速入口

- **房间计算入口**: `src/fast_model/room_model.rs`
- **空间索引查询**: `aios_core::spatial::sqlite`
- **测试程序**: `examples/room_tee_containment.rs`

## 核心算法：两阶段计算

房间计算采用 **粗算 + 精算** 两阶段策略，在保证准确性的同时优化性能：

### 第一阶段：粗算（Coarse Calculation）

```
┌─────────────────────────────────────────────────────────┐
│  SQLite RTree 空间索引                                    │
│  ┌─────────┐                                             │
│  │ 面板 AABB │ ──查询重叠──▶ 候选构件列表 (通常 100-500 个) │
│  └─────────┘                                             │
│  时间复杂度: O(log n + k)                                  │
└─────────────────────────────────────────────────────────┘
```

### 第二阶段：精算（Fine Calculation）

```
┌─────────────────────────────────────────────────────────┐
│  射线投射法 (Ray Casting)                                 │
│  ┌─────────┐     ┌──────────┐                           │
│  │ 构件关键点 │ ──射线──▶ │ 面板 TriMesh │ ──判断内外──▶ 结果  │
│  └─────────┘     └──────────┘                           │
│  阈值策略: >50% 关键点在内 = 属于该房间                     │
└─────────────────────────────────────────────────────────┘
```

## 关键发现 (2024-12)

### parry3d `is_inside` 不可靠

**问题**: `parry3d::TriMesh::project_point().is_inside` 对于某些封闭网格返回错误结果，即使：
- 网格是有效的封闭流形（0 边界边）
- 绕向一致（法向量全部朝外）
- pseudo-normals 已正确计算

**解决方案**: 使用 **射线投射法** 替代 `is_inside`：
```rust
// 向 ±Z 方向发射射线，两侧都有交点则在内部
let ray_pos_z = Ray::new(point, Vector::new(0.0, 0.0, 1.0));
let ray_neg_z = Ray::new(point, Vector::new(0.0, 0.0, -1.0));
let inside = hit_pos_z.is_some() && hit_neg_z.is_some();
```

详见 [04_注意事项.md](./04_注意事项.md#1-parry3d-is_inside-不可靠)

## 数据库关系

```
FRMW (房间)
  └── SBFR (子框架)
       └── PANE (面板) ──shapes──▶ geo_relate ──▶ mesh 文件
                │
                └── room_relate ──▶ 构件 (TEE, ELBO, VALV...)
```

## 性能指标

| 阶段 | 典型耗时 | 说明 |
|------|---------|------|
| 粗算 | 100-200ms | SQLite RTree 查询 |
| 精算 | 5-100ms/构件 | 射线投射检测 |
| 总计 | 1-90s | 取决于候选构件数量 |

## 相关文档

- [llmdoc/agent/room_spatial_algorithm_investigation_20251211.md](../../../llmdoc/agent/room_spatial_algorithm_investigation_20251211.md)
- [llmdoc/agent/room_data_model_and_storage_20251211.md](../../../llmdoc/agent/room_data_model_and_storage_20251211.md)
