# Quickstart: 测量数据提供与前端捕捉闭环(spec 010)

## 后端:导出与验证

```powershell
# 1. 导出样本 dbnum 包(含 primitive_keypoints.parquet)
cd D:\work\plant-code\plant-model-gen-cata-closure
cargo run --release --bin aios-database -- `
  --export-dbnum-instances-parquet --dbnum 250160 --verbose

# 2. 抽查新表(DuckDB CLI)
duckdb -c "SELECT kind, count(*) FROM 'output/parquet/250160/primitive_keypoints.parquet' GROUP BY kind;"
duckdb -c "SELECT * FROM 'output/parquet/250160/primitive_keypoints.parquet' LIMIT 10;"

# 3. manifest 校验:两份均应含 tables.primitive_keypoints 与 primitive_keypoint_unit
#    output/parquet/250160/manifest.json
#    output/parquet/manifest_250160.json
```

预期:

- `primitive_keypoints.parquet` rows > 0;
- kind 仅出现 Endpoint/Midpoint/Center/Intersection/SurfacePoint;
- manifest `primitive_keypoint_unit.coordinate_space == "geo_local"`。

## 后端:ptset 双通道一致性

```powershell
# web_server 起服后(站点 quicktest-250160-8080):
curl "http://127.0.0.1:8080/api/pdms/ptset/2013286704/477"
# 对照 DuckDB:
duckdb -c "SELECT p.* FROM 'output/parquet/250160/ptsets.parquet' p JOIN 'output/parquet/250160/instances.parquet' i ON p.cata_hash=i.cata_hash WHERE i.refno_str='2013286704/477' ORDER BY p.point_number;"
```

预期:point_number 集合一致,坐标差 ≤1e-3mm。

## 前端:捕捉联动(plant3d-web)

```powershell
cd D:\work\plant-code\plant3d-web
npm run dev
```

1. 加载样本 dbnum=250160 包;
2. 进入测量模式,打开测量面板的点源矩阵;
3. 勾选 Primitive Key Point 的 display+snap → hover 构件出现图元
   关键点 marker,点击可捕捉成测量点;
4. 取消勾选 PTSET snap、勾选 Mesh Pick Point snap → 表面点可测;
5. 删除包内 primitive_keypoints.parquet 重载 → 该源显示"不可用",
   其余源正常;
6. 创建各源测量记录后刷新 → 旧/新记录均正常渲染,新记录带来源标签。

详细场景以 `plant3d-web/specs/001-measurement-pick-sources/quickstart.md`
为准。
