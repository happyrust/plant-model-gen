<!-- This entire block is your raw intelligence report for other agents. It is NOT a final document. -->

### Code Sections (The Evidence)

- `src/fast_model/loop_model.rs` (gen_loop_geos function, lines 23-308): Main entry point for PANE/FLOOR/GWALL geometry generation. Handles loop owner types including PANE, creates Extrusion objects with vertices and height parameters.

- `src/fast_model/loop_model.rs` (lines 184-230): PANE/FLOOR/GWALL/EXTR/NXTR/AEXTR type handling. Creates Extrusion struct with `verts` (loop vertices) and `height` parameters, converts to PdmsGeoParam.

- `src/fast_model/loop_processor.rs` (process_loop_refno_page function, lines 27-43): High-level processor for Loop Owner types, supports PANE/FLOOR/GWALL/SCREED types, delegates to loop_model::gen_loop_geos.

- `src/fast_model/mesh_generate.rs` (gen_inst_meshes function, lines 625-900+): Core mesh generation orchestrator. Queries inst_geo parameters, calls generate_csg_mesh function from aios_core library.

- `src/fast_model/mesh_generate.rs` (lines 747-753): Unified CSG mesh generation pipeline using `generate_csg_mesh(&g.param, &profile.csg_settings, non_scalable_geo, refno_for_mesh)`. This is the central function that converts geometric parameters (including Extrusion for PANE) into GeneratedMesh.

- `src/fast_model/mesh_generate.rs` (handle_csg_mesh function, lines 946-985): Post-processing of generated CSG mesh. Extracts vertices and indices from GeneratedMesh, serializes PlantMesh to file, updates database with meshed flag and AABB.

- `src/fast_model/mesh_generate.rs` (derive_csg_points function, lines 987-1002): Extracts unique vertices from PlantMesh by iterating over `mesh.vertices` field and generating hash-based point references.

- `aios_core` library (external, referenced in Cargo.toml line 78): Contains Extrusion struct definition (`aios_core::prim_geo::Extrusion`) and `generate_csg_mesh` function that handles actual mesh generation from geometric parameters.

### Report (The Answers)

#### result

**1. PANE 类型 mesh 生成方式**

PANE（面板）类型的 mesh 生成流程：

1. **参数提取阶段** (`src/fast_model/loop_model.rs:184-230`)
   - 从 PDMS 数据库查询 PANE 类型元素的 loop 轮廓 (`aios_core::fetch_loops_and_height`)
   - 获取拉伸高度 (`height` 属性)
   - 创建 Extrusion 对象：`Extrusion { verts, height, ..Default::default() }`
   - 转换为 PdmsGeoParam（几何参数格式）

2. **几何生成阶段** (`src/fast_model/mesh_generate.rs:748-753`)
   - 所有几何类型（包括 PANE 对应的 Extrusion）统一使用 CSG 方式
   - 调用 `generate_csg_mesh(&g.param, &profile.csg_settings, ...)`
   - 该函数来自 `aios_core` 外部库，完成实际的 mesh 生成

3. **网格后处理** (`src/fast_model/mesh_generate.rs:946-985`)
   - 从生成的 GeneratedMesh 中提取 PlantMesh 对象
   - PlantMesh 包含 `vertices` 字段（顶点坐标数组）和 `indices` 字段（三角形索引）
   - 序列化到文件：`mesh.ser_to_file(&dir.join(format!("{}.mesh", mesh_id)))`

**2. Mesh 是否为封闭 manifold**

基于代码分析，**无法从本项目源代码直接确认** PANE mesh 是否生成为完全封闭的 manifold。原因：

- **关键决策在 aios_core 库**：实际的 mesh 拓扑（是否包含顶面、底面、侧面）由 `aios_core::generate_csg_mesh` 函数决定
- **本项目仅负责参数准备**：loop_model.rs 只收集 vertices 和 height 参数，不涉及面片生成逻辑
- **Extrusion 结构**：本项目使用的 `aios_core::prim_geo::Extrusion` 仅定义了 `verts`（轮廓顶点）和 `height`（拉伸距离），如何从这两个参数生成完整的封闭 mesh 是 aios_core 内部实现

**代码证据**（`src/fast_model/loop_model.rs:206-215`）：
```rust
let extrusion = Box::new(Extrusion {
    verts,
    height,
    ..Default::default()
});
geo_param = extrusion
    .convert_to_geo_param()
    .unwrap_or(PdmsGeoParam::Unknown);
```

此处只是创建参数对象，不生成任何 mesh。真实的 mesh 生成发生在 `generate_csg_mesh` 调用时。

**3. Mesh 的三角形索引生成逻辑**

本项目代码中 **未实现三角形索引生成**。详细分析：

- **索引提取而非生成** (`src/fast_model/mesh_generate.rs:987-1002`)：
  ```rust
  fn derive_csg_points(mesh: &PlantMesh, pts_json_map: &Arc<DashMap<u64, String>>) -> Vec<String> {
      let mut hashes = HashSet::new();
      for vertex in &mesh.vertices {
          let rs_vec = RsVec3(*vertex);
          let hash = rs_vec.gen_hash();
          // 提取顶点，不处理索引
      }
  }
  ```

- **PlantMesh 的 indices 字段**：在 mesh_generate.rs 中被认为已存在（由 generate_csg_mesh 生成）
- **三角形索引生成权属**：由 `aios_core` 库的 `generate_csg_mesh` 函数完成
- **本项目职责**：从生成的 PlantMesh 中提取顶点进行去重和哈希化处理

**证据**（`src/fast_model/mesh_generate.rs:959-968`）：
```rust
let mesh_aabb = generated
    .mesh
    .aabb
    .ok_or_else(|| anyhow!("CSG mesh 缺少有效的 AABB"))?;

let pt_refs = derive_csg_points(&generated.mesh, pts_json_map);

generated
    .mesh
    .ser_to_file(&dir.join(format!("{}.mesh", mesh_id)))?;
```

#### conclusions

**关键事实：**

1. **PANE 生成流程清晰**：PANE/FLOOR/GWALL 被统一作为 Extrusion 类型处理，创建 `Extrusion { verts, height }` 对象，然后通过 CSG 管道生成 mesh。

2. **Manifold 封闭性未知**：无法从本项目代码确认 PANE mesh 是否为封闭的 manifold，这取决于 aios_core 库中 `generate_csg_mesh` 的实现细节（是否自动闭合顶面和底面）。

3. **索引生成由 aios_core 负责**：三角形索引（PlantMesh.indices）由外部库 aios_core 的 CSG 引擎生成，本项目不处理索引生成逻辑。

4. **代码分层明确**：
   - 本项目层：参数收集 → 几何参数转换
   - aios_core 层：CSG 网格生成 → 三角形索引生成 → PlantMesh 创建
   - 本项目层：网格后处理 → 文件序列化 → 数据库更新

#### relations

- `src/fast_model/loop_processor.rs` (process_loop_refno_page) 调用 `loop_model::gen_loop_geos` 处理 PANE 类型
- `gen_loop_geos` 创建 Extrusion 对象并转换为 PdmsGeoParam，发送数据到通道
- `src/fast_model/mesh_generate.rs` (gen_inst_meshes) 消费这些参数，调用 `generate_csg_mesh` 生成 PlantMesh
- `handle_csg_mesh` 从生成的 PlantMesh 提取顶点并序列化
- PANE/FLOOR/GWALL 三种类型处理方式相同（行 89, 219），都使用 Extrusion + 高度参数

**性能优化点**：
- PANE 处理支持并发批处理（最多 16 个并发线程）
- SJUS 调整在 loop_model 处理阶段完成（行 89-102）
- 布尔运算（负体处理）支持 PANE 类型（行 104-142）
