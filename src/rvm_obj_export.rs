use crate::fast_model::unit_converter::UnitConverter;
use aios_core::geometry::csg::{
    generate_csg_mesh, unit_box_mesh, unit_cylinder_mesh, unit_sphere_mesh,
};
use aios_core::mesh_precision::LodMeshSettings;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::prim_geo::ctorus::CTorus;
use aios_core::prim_geo::dish::Dish;
use aios_core::prim_geo::lpyramid::LPyramid;
use aios_core::prim_geo::rtorus::RTorus;
use aios_core::prim_geo::snout::LSnout;
use aios_core::shape::pdms_shape::{Edge, PlantMesh};
use anyhow::{Context, Result, anyhow};
use glam::{DMat4, DVec3, Vec3};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct RvmObjExportStats {
    pub refno_count: usize,
    pub geometry_count: usize,
    pub output_file_size: u64,
}

pub fn export_rvm_obj_from_relation_store(
    dbnum: u32,
    relation_store_root: &Path,
    output_path: &Path,
    unit_converter: &UnitConverter,
    verbose: bool,
) -> Result<RvmObjExportStats> {
    let db_path = relation_store_root.join(format!("{}/relations.db", dbnum));
    if !db_path.exists() {
        return Err(anyhow!("RVM 关系库不存在: {}", db_path.display()));
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("打开 RVM 关系库失败: {}", db_path.display()))?;
    let mut stmt = conn.prepare(
        "SELECT ir.refno, ig.geometry
         FROM inst_relate ir
         JOIN geo_relate gr ON gr.inst_id = ir.inst_id
         JOIN inst_geo ig ON ig.hash = gr.geo_hash
         ORDER BY ir.id, gr.id",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;

    let mut refnos = BTreeSet::new();
    let mut merged = PlantMesh::default();
    let mut geometry_count = 0usize;

    for row in rows {
        let (refno, geometry_blob) = row?;
        refnos.insert(refno);
        let payload: Value =
            serde_json::from_slice(&geometry_blob).context("解析 inst_geo.geometry JSON 失败")?;
        if let Some(mesh) = mesh_from_payload(&payload)? {
            merged.merge(&mesh);
            geometry_count += 1;
        }
    }

    if geometry_count == 0 || merged.vertices.is_empty() || merged.indices.is_empty() {
        return Err(anyhow!("RVM 关系库中没有可导出的 OBJ 几何"));
    }

    if unit_converter.needs_conversion() {
        for vertex in &mut merged.vertices {
            *vertex = unit_converter.convert_vec3(vertex);
        }
        for edge in &mut merged.edges {
            for vertex in &mut edge.vertices {
                *vertex = unit_converter.convert_vec3(vertex);
            }
        }
    }

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建 OBJ 输出目录失败: {}", parent.display()))?;
    }
    merged
        .export_obj(false, &output_path.to_string_lossy())
        .with_context(|| format!("导出 RVM OBJ 失败: {}", output_path.display()))?;

    let output_file_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);

    if verbose {
        println!("📦 RVM relation-store OBJ 导出完成");
        println!("   - 数据库: {}", db_path.display());
        println!("   - 输出: {}", output_path.display());
        println!("   - Refno 数量: {}", refnos.len());
        println!("   - 几何数量: {}", geometry_count);
        println!("   - 文件大小: {} bytes", output_file_size);
    }

    Ok(RvmObjExportStats {
        refno_count: refnos.len(),
        geometry_count,
        output_file_size,
    })
}

fn mesh_from_payload(payload: &Value) -> Result<Option<PlantMesh>> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let transform = parse_transform(payload)?;

    let local_mesh = match kind {
        "FacetGroup" => build_facet_group_mesh(payload)?,
        "Pyramid" => build_pyramid_mesh(payload)?,
        "RectangularTorus" => build_rectangular_torus_mesh(payload)?,
        "CircularTorus" => build_circular_torus_mesh(payload)?,
        "EllipticalDish" => build_elliptical_dish_mesh(payload)?,
        "SphericalDish" => build_spherical_dish_mesh(payload)?,
        "Snout" => build_snout_mesh(payload)?,
        "Line" => build_line_mesh(payload)?,
        "Box" => build_box_mesh(payload)?,
        "Cylinder" => build_cylinder_mesh(payload)?,
        "Sphere" => build_sphere_mesh(payload)?,
        _ => build_bbox_fallback_mesh(payload)?,
    };

    Ok(local_mesh.map(|mesh| mesh.transform_by(&transform)))
}

fn build_facet_group_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let facet_group = payload
        .get("detail")
        .and_then(|v| v.get("facet_group"))
        .or_else(|| payload.get("facet_group"));
    let Some(facet_group) = facet_group else {
        return Ok(None);
    };
    let Some(polygons) = facet_group.get("polygons").and_then(Value::as_array) else {
        return Ok(None);
    };

    let mut mesh = PlantMesh::default();

    for polygon in polygons {
        let Some(contours) = polygon.get("contours").and_then(Value::as_array) else {
            continue;
        };
        for contour in contours {
            let vertices = parse_vec3_array(contour.get("vertices"));
            if vertices.len() < 3 {
                continue;
            }
            let normals = parse_vec3_array(contour.get("normals"));
            let base = mesh.vertices.len() as u32;
            for vertex in &vertices {
                mesh.vertices.push(*vertex);
            }
            if normals.len() == vertices.len() {
                mesh.normals.extend(normals.iter().copied());
            }
            mesh.edges.push(Edge::new(vertices.clone()));
            for i in 1..(vertices.len() - 1) {
                mesh.indices
                    .extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
            }
        }
    }

    if mesh.indices.is_empty() {
        return Ok(None);
    }

    if mesh.normals.len() != mesh.vertices.len() {
        mesh.normals.clear();
    }
    mesh.sync_wire_vertices_from_edges();
    Ok(Some(mesh))
}

fn build_pyramid_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some(detail) = payload.get("detail").and_then(|v| v.get("pyramid")) else {
        return Ok(None);
    };
    let bottom = parse_f32_pair(detail.get("bottom"))?;
    let top = parse_f32_pair(detail.get("top"))?;
    let offset = parse_f32_pair(detail.get("offset"))?;
    let height = detail
        .get("height")
        .ok_or_else(|| anyhow!("缺少 pyramid.height 字段"))
        .and_then(value_to_f32)?;
    let Some((bbox_min, bbox_max)) = parse_bbox_local(payload) else {
        return Ok(None);
    };

    let param = PdmsGeoParam::PrimLPyramid(LPyramid {
        pbbt: bottom[0],
        pcbt: bottom[1],
        pbtp: top[0],
        pctp: top[1],
        pbof: offset[0],
        pcof: offset[1],
        ptdi: height,
        pbdi: 0.0,
        ..Default::default()
    });

    build_generated_param_mesh(
        &param,
        Some(DMat4::from_translation(DVec3::new(
            ((bbox_min.x + bbox_max.x) * 0.5) as f64,
            ((bbox_min.y + bbox_max.y) * 0.5) as f64,
            bbox_min.z as f64,
        ))),
    )
}

fn build_rectangular_torus_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some(detail) = payload
        .get("detail")
        .and_then(|v| v.get("rectangular_torus"))
    else {
        return Ok(None);
    };
    let inner_radius = detail
        .get("inner_radius")
        .ok_or_else(|| anyhow!("缺少 rectangular_torus.inner_radius 字段"))
        .and_then(value_to_f32)?;
    let outer_radius = detail
        .get("outer_radius")
        .ok_or_else(|| anyhow!("缺少 rectangular_torus.outer_radius 字段"))
        .and_then(value_to_f32)?;
    let height = detail
        .get("height")
        .ok_or_else(|| anyhow!("缺少 rectangular_torus.height 字段"))
        .and_then(value_to_f32)?;
    let angle = detail
        .get("angle")
        .ok_or_else(|| anyhow!("缺少 rectangular_torus.angle 字段"))
        .and_then(value_to_f32)?;

    let param = PdmsGeoParam::PrimRTorus(RTorus {
        rins: inner_radius,
        rout: outer_radius,
        height,
        angle,
    });

    build_generated_param_mesh(&param, parse_bbox_center_translation(payload))
}

fn build_circular_torus_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some(detail) = payload.get("detail").and_then(|v| v.get("circular_torus")) else {
        return Ok(None);
    };
    let offset = detail
        .get("offset")
        .ok_or_else(|| anyhow!("缺少 circular_torus.offset 字段"))
        .and_then(value_to_f32)?;
    let radius = detail
        .get("radius")
        .ok_or_else(|| anyhow!("缺少 circular_torus.radius 字段"))
        .and_then(value_to_f32)?;
    let angle = detail
        .get("angle")
        .ok_or_else(|| anyhow!("缺少 circular_torus.angle 字段"))
        .and_then(value_to_f32)?;

    let param = PdmsGeoParam::PrimCTorus(CTorus {
        rins: (offset - radius).max(f32::EPSILON),
        rout: offset + radius,
        angle,
    });

    build_generated_param_mesh(&param, parse_bbox_center_translation(payload))
}

fn build_elliptical_dish_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some(detail) = payload.get("detail").and_then(|v| v.get("elliptical_dish")) else {
        return Ok(None);
    };
    let base_radius = detail
        .get("base_radius")
        .ok_or_else(|| anyhow!("缺少 elliptical_dish.base_radius 字段"))
        .and_then(value_to_f32)?;
    let height = detail
        .get("height")
        .ok_or_else(|| anyhow!("缺少 elliptical_dish.height 字段"))
        .and_then(value_to_f32)?;

    let param = PdmsGeoParam::PrimDish(Dish {
        pdia: base_radius * 2.0,
        pheig: height,
        prad: base_radius,
        ..Default::default()
    });

    build_generated_param_mesh(&param, parse_bbox_floor_center_translation(payload))
}

fn build_spherical_dish_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some(detail) = payload.get("detail").and_then(|v| v.get("spherical_dish")) else {
        return Ok(None);
    };
    let base_radius = detail
        .get("base_radius")
        .ok_or_else(|| anyhow!("缺少 spherical_dish.base_radius 字段"))
        .and_then(value_to_f32)?;
    let height = detail
        .get("height")
        .ok_or_else(|| anyhow!("缺少 spherical_dish.height 字段"))
        .and_then(value_to_f32)?;

    let param = PdmsGeoParam::PrimDish(Dish {
        pdia: base_radius * 2.0,
        pheig: height,
        ..Default::default()
    });

    build_generated_param_mesh(&param, parse_bbox_floor_center_translation(payload))
}

fn build_snout_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some(detail) = payload.get("detail").and_then(|v| v.get("snout")) else {
        return Ok(None);
    };
    let offset_y = detail
        .get("offset_y")
        .ok_or_else(|| anyhow!("缺少 snout.offset_y 字段"))
        .and_then(value_to_f32)?;
    let bottom_shear_x = detail
        .get("bottom_shear_x")
        .ok_or_else(|| anyhow!("缺少 snout.bottom_shear_x 字段"))
        .and_then(value_to_f32)?;
    let bottom_shear_y = detail
        .get("bottom_shear_y")
        .ok_or_else(|| anyhow!("缺少 snout.bottom_shear_y 字段"))
        .and_then(value_to_f32)?;
    let top_shear_x = detail
        .get("top_shear_x")
        .ok_or_else(|| anyhow!("缺少 snout.top_shear_x 字段"))
        .and_then(value_to_f32)?;
    let top_shear_y = detail
        .get("top_shear_y")
        .ok_or_else(|| anyhow!("缺少 snout.top_shear_y 字段"))
        .and_then(value_to_f32)?;

    if offset_y.abs() > f32::EPSILON
        || bottom_shear_x.abs() > f32::EPSILON
        || bottom_shear_y.abs() > f32::EPSILON
        || top_shear_x.abs() > f32::EPSILON
        || top_shear_y.abs() > f32::EPSILON
    {
        return build_bbox_fallback_mesh(payload);
    }

    let radius_bottom = detail
        .get("radius_bottom")
        .ok_or_else(|| anyhow!("缺少 snout.radius_bottom 字段"))
        .and_then(value_to_f32)?;
    let radius_top = detail
        .get("radius_top")
        .ok_or_else(|| anyhow!("缺少 snout.radius_top 字段"))
        .and_then(value_to_f32)?;
    let height = detail
        .get("height")
        .ok_or_else(|| anyhow!("缺少 snout.height 字段"))
        .and_then(value_to_f32)?;
    let offset_x = detail
        .get("offset_x")
        .ok_or_else(|| anyhow!("缺少 snout.offset_x 字段"))
        .and_then(value_to_f32)?;

    let param = PdmsGeoParam::PrimLSnout(LSnout {
        pbdm: radius_bottom * 2.0,
        ptdm: radius_top * 2.0,
        pbdi: 0.0,
        ptdi: height,
        poff: offset_x,
        ..Default::default()
    });

    build_generated_param_mesh(&param, parse_bbox_floor_center_translation(payload))
}

fn build_generated_param_mesh(
    param: &PdmsGeoParam,
    local_transform: Option<DMat4>,
) -> Result<Option<PlantMesh>> {
    let Some(generated) = generate_csg_mesh(param, &LodMeshSettings::default(), false, false, None)
    else {
        return Ok(None);
    };
    let mesh = if let Some(local_transform) = local_transform {
        generated.mesh.transform_by(&local_transform)
    } else {
        generated.mesh
    };
    Ok(Some(mesh))
}

fn build_line_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some((bbox_min, bbox_max)) = parse_bbox_local(payload) else {
        return Ok(None);
    };
    let extents = bbox_max - bbox_min;
    if extents.x.abs() < f32::EPSILON
        || extents.y.abs() < f32::EPSILON
        || extents.z.abs() < f32::EPSILON
    {
        return Ok(None);
    }

    let unit_mesh = unit_cylinder_mesh(&LodMeshSettings::default(), false);
    let local_transform = DMat4::from_translation(DVec3::new(
        ((bbox_min.x + bbox_max.x) * 0.5) as f64,
        ((bbox_min.y + bbox_max.y) * 0.5) as f64,
        bbox_min.z as f64,
    )) * DMat4::from_scale(DVec3::new(
        extents.x as f64,
        extents.y as f64,
        extents.z as f64,
    ));
    Ok(Some(unit_mesh.transform_by(&local_transform)))
}

fn build_box_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    build_centered_bbox_mesh(payload, unit_box_mesh())
}

fn build_cylinder_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    let Some((bbox_min, bbox_max)) = parse_bbox_local(payload) else {
        return Ok(None);
    };
    let extents = bbox_max - bbox_min;
    if extents.x.abs() < f32::EPSILON
        || extents.y.abs() < f32::EPSILON
        || extents.z.abs() < f32::EPSILON
    {
        return Ok(None);
    }

    let unit_mesh = unit_cylinder_mesh(&LodMeshSettings::default(), false);
    let local_transform = DMat4::from_translation(DVec3::new(
        ((bbox_min.x + bbox_max.x) * 0.5) as f64,
        ((bbox_min.y + bbox_max.y) * 0.5) as f64,
        bbox_min.z as f64,
    )) * DMat4::from_scale(DVec3::new(
        extents.x as f64,
        extents.y as f64,
        extents.z as f64,
    ));
    Ok(Some(unit_mesh.transform_by(&local_transform)))
}

fn build_sphere_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    build_centered_bbox_mesh(payload, unit_sphere_mesh())
}

fn build_bbox_fallback_mesh(payload: &Value) -> Result<Option<PlantMesh>> {
    build_centered_bbox_mesh(payload, unit_box_mesh())
}

fn build_centered_bbox_mesh(payload: &Value, unit_mesh: PlantMesh) -> Result<Option<PlantMesh>> {
    let Some((bbox_min, bbox_max)) = parse_bbox_local(payload) else {
        return Ok(None);
    };
    let extents = bbox_max - bbox_min;
    if extents.x.abs() < f32::EPSILON
        || extents.y.abs() < f32::EPSILON
        || extents.z.abs() < f32::EPSILON
    {
        return Ok(None);
    }
    let center = (bbox_min + bbox_max) * 0.5;
    let local_transform = DMat4::from_translation(center.as_dvec3())
        * DMat4::from_scale(DVec3::new(
            extents.x as f64,
            extents.y as f64,
            extents.z as f64,
        ));
    Ok(Some(unit_mesh.transform_by(&local_transform)))
}

fn parse_transform(payload: &Value) -> Result<DMat4> {
    let transform = payload
        .get("transform")
        .ok_or_else(|| anyhow!("缺少 transform 字段"))?;
    let matrix = transform
        .get("matrix3")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("缺少 transform.matrix3 字段"))?;
    let translation = transform
        .get("translation")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("缺少 transform.translation 字段"))?;
    if matrix.len() != 9 || translation.len() != 3 {
        return Err(anyhow!("transform 数组长度无效"));
    }

    let m = matrix
        .iter()
        .map(value_to_f64)
        .collect::<Result<Vec<_>>>()?;
    let t = translation
        .iter()
        .map(value_to_f64)
        .collect::<Result<Vec<_>>>()?;

    Ok(DMat4::from_cols_array(&[
        m[0], m[1], m[2], 0.0, m[3], m[4], m[5], 0.0, m[6], m[7], m[8], 0.0, t[0], t[1], t[2], 1.0,
    ]))
}

fn parse_bbox_local(payload: &Value) -> Option<(Vec3, Vec3)> {
    let bbox = payload.get("bbox_local")?;
    let min = bbox.get("min").and_then(Value::as_array)?;
    let max = bbox.get("max").and_then(Value::as_array)?;
    if min.len() != 3 || max.len() != 3 {
        return None;
    }
    let min = Vec3::new(
        value_to_f32(&min[0]).ok()?,
        value_to_f32(&min[1]).ok()?,
        value_to_f32(&min[2]).ok()?,
    );
    let max = Vec3::new(
        value_to_f32(&max[0]).ok()?,
        value_to_f32(&max[1]).ok()?,
        value_to_f32(&max[2]).ok()?,
    );
    Some((min, max))
}

fn parse_bbox_center_translation(payload: &Value) -> Option<DMat4> {
    let (bbox_min, bbox_max) = parse_bbox_local(payload)?;
    let center = (bbox_min + bbox_max) * 0.5;
    Some(DMat4::from_translation(center.as_dvec3()))
}

fn parse_bbox_floor_center_translation(payload: &Value) -> Option<DMat4> {
    let (bbox_min, bbox_max) = parse_bbox_local(payload)?;
    Some(DMat4::from_translation(DVec3::new(
        ((bbox_min.x + bbox_max.x) * 0.5) as f64,
        ((bbox_min.y + bbox_max.y) * 0.5) as f64,
        bbox_min.z as f64,
    )))
}

fn parse_f32_pair(value: Option<&Value>) -> Result<[f32; 2]> {
    let arr = value
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("JSON 数组字段缺失或格式错误"))?;
    if arr.len() != 2 {
        return Err(anyhow!("JSON 数组字段长度不是 2"));
    }
    Ok([value_to_f32(&arr[0])?, value_to_f32(&arr[1])?])
}

fn parse_vec3_array(value: Option<&Value>) -> Vec<Vec3> {
    let Some(values) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|item| {
            let arr = item.as_array()?;
            if arr.len() != 3 {
                return None;
            }
            Some(Vec3::new(
                value_to_f32(&arr[0]).ok()?,
                value_to_f32(&arr[1]).ok()?,
                value_to_f32(&arr[2]).ok()?,
            ))
        })
        .collect()
}

fn value_to_f32(value: &Value) -> Result<f32> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| anyhow!("JSON 数值字段不是 f32"))
}

fn value_to_f64(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .ok_or_else(|| anyhow!("JSON 数值字段不是 f64"))
}
