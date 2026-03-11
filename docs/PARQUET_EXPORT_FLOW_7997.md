# Parquet 导出流程分析（dbnum 7997）

## 一、触发入口

| 入口 | 参数 | 说明 |
|------|------|------|
| CLI | `--export-parquet` | 显式 Parquet 格式 |
| CLI | `--export-dbnum-instances` | 默认 Parquet（无 --export-dbnum-instances-json 时）|
| 模型生成后 | `--export-parquet-after-gen` | 按 manual_db_nums 逐 dbnum 导出 |
| MBD pipe | 后台异步 | `trigger_async_parquet_export(dbnum)` |

## 二、核心流程（`export_dbnum_instances_parquet`）

```
┌─────────────────────────────────────────────────────────────────┐
│  export_dbnum_instances_parquet(dbnum, output_dir, db_option)   │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 1. 加载 spec_info（BRAN/HANG/EQUI 专业信息，spec_value 回填）   │
│    - load_or_build_spec_info() 从 TreeIndex 或 SurrealDB         │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 2. 查询 inst_relate（按 dbnum 或 root_refno 子树过滤）           │
│    - 有 root_refno: query_deep_visible_inst_refnos → 分批查     │
│    - 无: query_inst_relate_by_dbnum()                           │
│    → 按 owner 分组 (BRAN/HANG/EQUI → grouped_children)          │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 3. 查询几何体实例（geo_relate / inst_relate_bool）               │
│    - aios_core::query_insts_for_export(in_refnos)                │
│    → export_inst_map: refno → ExportInstQuery                    │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 4. 查询 tubi_relate                                              │
│    - query_tubi_relate(tubi_owner_refnos)                        │
│    → tubings_map: owner_refno → Vec<TubiRelateRow>               │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 5. 构建 Parquet 行数据                                           │
│    - instance_rows, geo_instance_rows, tubing_rows              │
│    - 收集 trans_hashes, aabb_hashes                              │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 6. 查询 trans / aabb 表（按 hash 批量）                          │
│    - query_trans_rows(trans_hashes)                              │
│    - query_aabb_rows(aabb_hashes)                                │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│ 7. 单位转换 + 写 Parquet 文件                                    │
│    - instances.parquet                                           │
│    - geo_instances.parquet                                       │
│    - tubings.parquet                                             │
│    - transforms.parquet                                          │
│    - aabb.parquet                                                │
│    - manifest.json                                               │
│    - missing_mesh_report.json（若有缺失 mesh）                    │
└─────────────────────────────────────────────────────────────────┘
```

## 三、输出目录结构

```
output/<project>/parquet/<dbnum>/
├── instances.parquet      # 一行一个实例 refno
├── geo_instances.parquet # 几何引用 (refno × geo_index)
├── tubings.parquet       # TUBI 段
├── transforms.parquet    # 变换矩阵（去重）
├── aabb.parquet         # 包围盒（去重）
├── manifest.json         # 元信息
└── missing_mesh_report.json  # 可选，缺失 mesh 时生成
```

## 四、已知性能瓶颈（见 parquet_export_code_review.md）

- `fn::default_full_name(in)` 触发图遍历，批量导出耗时长
- `in->inst_relate_aabb[0].out` 图遍历
- 串行查询，无并发（步骤 2–6）
- trans/aabb 查询可并行

## 五、运行命令

```bash
# 导出 dbnum 7997 为 Parquet
cargo run --release -p aios-database -- --export-parquet --dbnum 7997 -v

# 等价
cargo run --release -p aios-database -- --export-dbnum-instances --dbnum 7997 -v

# 仅导出某根节点子树
cargo run --release -p aios-database -- --export-parquet --dbnum 7997 --root-refno 24381_145018 -v

# 指定输出目录
cargo run --release -p aios-database -- --export-parquet --dbnum 7997 --output ./my_parquet -v
```
