use crate::model_relation_store::{InstGeoRecord, InstRelateRecord, ModelRelationStore};
use aios_core::pdms_types::{RefU64, RefnoEnum};
use anyhow::{Context, Result, anyhow};
use bincode::serialize;
use glam::{Affine3A, Mat3A, Vec3, Vec3A};
use rvm_rs::store::Store;
use rvm_rs::store::geometry::{Geometry, GeometryKind, GeometryType};
use rvm_rs::store::node::{NodeId, NodeKind};
use rvm_rs::{parse_att, parse_rvm};
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use twox_hash::XxHash64;

#[derive(Debug, Clone)]
pub struct RvmImportOptions {
    pub dbnum: u32,
    pub relation_store_root: PathBuf,
    pub rvm_path: PathBuf,
    pub att_paths: Vec<PathBuf>,
    pub verbose: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RvmImportStats {
    pub file_nodes: usize,
    pub model_nodes: usize,
    pub group_nodes: usize,
    pub geometry_records: usize,
    pub cleaned_records: usize,
}

pub fn import_rvm_to_sqlite(options: &RvmImportOptions) -> Result<RvmImportStats> {
    let mut store = Store::new();
    let rvm_bytes = fs::read(&options.rvm_path)
        .with_context(|| format!("读取 RVM 文件失败: {}", options.rvm_path.display()))?;
    parse_rvm(&rvm_bytes, &mut store)
        .with_context(|| format!("解析 RVM 文件失败: {}", options.rvm_path.display()))?;

    for att_path in &options.att_paths {
        let att_text = fs::read_to_string(att_path)
            .with_context(|| format!("读取 ATT 文件失败: {}", att_path.display()))?;
        parse_att(&att_text, &mut store)
            .with_context(|| format!("解析 ATT 文件失败: {}", att_path.display()))?;
    }

    let mut builder = RelationBuilder::new(options.dbnum, options.verbose);
    builder.build(&store)?;

    let relation_store = ModelRelationStore::new(&options.relation_store_root);
    let refnos: Vec<RefnoEnum> = builder.inst_relates.iter().map(|r| r.refno).collect();
    let cleaned_records = relation_store
        .cleanup_by_refnos(options.dbnum, &refnos)
        .unwrap_or(0);
    relation_store.insert_inst_geos(options.dbnum, &builder.inst_geos)?;
    relation_store.insert_inst_relates(options.dbnum, &builder.inst_relates)?;
    relation_store.insert_geo_relates(options.dbnum, &builder.geo_relates)?;

    Ok(RvmImportStats {
        file_nodes: builder.stats.file_nodes,
        model_nodes: builder.stats.model_nodes,
        group_nodes: builder.stats.group_nodes,
        geometry_records: builder.stats.geometry_records,
        cleaned_records,
    })
}

struct RelationBuilder {
    dbnum: u32,
    verbose: bool,
    inst_relates: Vec<InstRelateRecord>,
    inst_geos: Vec<InstGeoRecord>,
    geo_relates: Vec<(u64, u64)>,
    stats: RvmImportStats,
}

impl RelationBuilder {
    fn new(dbnum: u32, verbose: bool) -> Self {
        Self {
            dbnum,
            verbose,
            inst_relates: Vec::new(),
            inst_geos: Vec::new(),
            geo_relates: Vec::new(),
            stats: RvmImportStats::default(),
        }
    }

    fn build(&mut self, store: &Store) -> Result<()> {
        for &root in store.roots() {
            self.walk_node(store, root, &mut VecDeque::new(), None, Vec3::ZERO)?;
        }
        Ok(())
    }

    fn walk_node(
        &mut self,
        store: &Store,
        node_id: NodeId,
        path: &mut VecDeque<String>,
        parent_refno: Option<RefnoEnum>,
        parent_translation: Vec3,
    ) -> Result<()> {
        let node = store
            .get_node(node_id)
            .ok_or_else(|| anyhow!("无效的节点 ID: {}", node_id.0))?;

        match &node.kind {
            NodeKind::File(file) => {
                self.stats.file_nodes += 1;
                let name =
                    sanitize_name(store.get_string(file.info), format!("file_{}", node_id.0));
                path.push_back(name);
                self.walk_children(store, node, path, parent_refno, parent_translation)?;
                path.pop_back();
            }
            NodeKind::Model(model) => {
                self.stats.model_nodes += 1;
                let name =
                    sanitize_name(store.get_string(model.name), format!("model_{}", node_id.0));
                path.push_back(name);
                self.walk_children(store, node, path, parent_refno, parent_translation)?;
                path.pop_back();
            }
            NodeKind::Group(group) => {
                self.stats.group_nodes += 1;
                let name =
                    sanitize_name(store.get_string(group.name), format!("group_{}", node_id.0));
                path.push_back(name.clone());
                let current_path = join_path(path);
                let refno = stable_refno(self.dbnum, &current_path, "group");
                let inst_id = refno.0;
                let world_translation = parent_translation + group.translation;
                let world_affine = affine_from_translation(world_translation);

                if self.verbose {
                    println!(
                        "[rvm-import] group path={} refno={} geos={} attrs={}",
                        current_path,
                        refno.0,
                        count_group_geometries(group.first_geometry, store),
                        group.attributes.len()
                    );
                }

                self.inst_relates.push(InstRelateRecord {
                    refno: RefnoEnum::from(refno),
                    inst_id,
                    parent_refno,
                    world_matrix: Some(encode_affine_blob(&world_affine)?),
                });

                let mut geometry_link = group.first_geometry;
                let mut geometry_index = 0usize;
                while let Some(geometry_id) = geometry_link {
                    let geometry = store
                        .get_geometry(geometry_id)
                        .ok_or_else(|| anyhow!("无效的几何 ID: {}", geometry_id.0))?;
                    geometry_index += 1;
                    self.push_geometry(
                        inst_id,
                        &current_path,
                        geometry_index,
                        geometry,
                        world_translation,
                    )?;
                    geometry_link = geometry.next;
                }

                self.walk_children(
                    store,
                    node,
                    path,
                    Some(RefnoEnum::from(refno)),
                    world_translation,
                )?;
                path.pop_back();
            }
        }

        Ok(())
    }

    fn walk_children(
        &mut self,
        store: &Store,
        node: &rvm_rs::store::node::Node,
        path: &mut VecDeque<String>,
        parent_refno: Option<RefnoEnum>,
        parent_translation: Vec3,
    ) -> Result<()> {
        let mut child = node.first_child;
        while let Some(child_id) = child {
            let child_node = store
                .get_node(child_id)
                .ok_or_else(|| anyhow!("无效的子节点 ID: {}", child_id.0))?;
            self.walk_node(store, child_id, path, parent_refno, parent_translation)?;
            child = child_node.next;
        }
        Ok(())
    }

    fn push_geometry(
        &mut self,
        inst_id: u64,
        group_path: &str,
        geometry_index: usize,
        geometry: &Geometry,
        world_translation: Vec3,
    ) -> Result<()> {
        self.stats.geometry_records += 1;
        let geo_hash = stable_geo_hash(self.dbnum, group_path, geometry_index, geometry);
        let final_bbox = translate_bbox(geometry.bbox_world, world_translation);
        self.inst_geos.push(InstGeoRecord {
            hash: geo_hash,
            geometry: encode_geometry_blob(
                geometry,
                group_path,
                geometry_index,
                world_translation,
                final_bbox,
            )?,
            aabb_min_x: final_bbox.is_valid().then_some(final_bbox.min.x as f64),
            aabb_min_y: final_bbox.is_valid().then_some(final_bbox.min.y as f64),
            aabb_min_z: final_bbox.is_valid().then_some(final_bbox.min.z as f64),
            aabb_max_x: final_bbox.is_valid().then_some(final_bbox.max.x as f64),
            aabb_max_y: final_bbox.is_valid().then_some(final_bbox.max.y as f64),
            aabb_max_z: final_bbox.is_valid().then_some(final_bbox.max.z as f64),
            meshed: false,
        });
        self.geo_relates.push((inst_id, geo_hash));
        Ok(())
    }
}

fn sanitize_name(raw: &str, fallback: String) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed.replace('\n', " ")
    }
}

fn join_path(path: &VecDeque<String>) -> String {
    path.iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

fn count_group_geometries(
    mut first: Option<rvm_rs::store::geometry::GeometryId>,
    store: &Store,
) -> usize {
    let mut count = 0usize;
    while let Some(id) = first {
        count += 1;
        first = store.get_geometry(id).and_then(|geo| geo.next);
    }
    count
}

fn stable_refno(dbnum: u32, path: &str, kind: &str) -> RefU64 {
    let mut hasher = XxHash64::with_seed(0x52_56_4d);
    hasher.write_u32(dbnum);
    hasher.write(kind.as_bytes());
    hasher.write(path.as_bytes());
    let low = (hasher.finish() as u32).max(1);
    RefU64::from(((dbnum as u64) << 32) | low as u64)
}

fn stable_geo_hash(
    dbnum: u32,
    group_path: &str,
    geometry_index: usize,
    geometry: &Geometry,
) -> u64 {
    let mut hasher = XxHash64::with_seed(0x67_65_6f);
    hasher.write_u32(dbnum);
    hasher.write(group_path.as_bytes());
    hasher.write_usize(geometry_index);
    hasher.write(geometry_signature(geometry).as_bytes());
    let hash = hasher.finish() & (i64::MAX as u64);
    hash.max(10)
}

fn geometry_signature(geometry: &Geometry) -> String {
    format!(
        "kind={:?}|type={}|color={}|rgb={}|bbox={:?}:{:?}",
        geometry.kind,
        geometry_type_name(geometry.geo_type),
        geometry.color,
        geometry.color_rgb,
        geometry.bbox_local.min,
        geometry.bbox_local.max
    )
}

fn geometry_kind_name(kind: &GeometryKind) -> &'static str {
    match kind {
        GeometryKind::Pyramid(_) => "Pyramid",
        GeometryKind::Box(_) => "Box",
        GeometryKind::RectangularTorus(_) => "RectangularTorus",
        GeometryKind::CircularTorus(_) => "CircularTorus",
        GeometryKind::EllipticalDish(_) => "EllipticalDish",
        GeometryKind::SphericalDish(_) => "SphericalDish",
        GeometryKind::Snout(_) => "Snout",
        GeometryKind::Cylinder(_) => "Cylinder",
        GeometryKind::Sphere(_) => "Sphere",
        GeometryKind::Line(_) => "Line",
        GeometryKind::FacetGroup(_) => "FacetGroup",
    }
}

fn geometry_detail_payload(kind: &GeometryKind) -> serde_json::Value {
    match kind {
        GeometryKind::Pyramid(data) => json!({
            "pyramid": {
                "bottom": data.bottom,
                "top": data.top,
                "offset": data.offset,
                "height": data.height,
            }
        }),
        GeometryKind::Box(data) => json!({
            "box": {
                "lengths": data.lengths,
            }
        }),
        GeometryKind::RectangularTorus(data) => json!({
            "rectangular_torus": {
                "inner_radius": data.inner_radius,
                "outer_radius": data.outer_radius,
                "height": data.height,
                "angle": data.angle,
            }
        }),
        GeometryKind::CircularTorus(data) => json!({
            "circular_torus": {
                "offset": data.offset,
                "radius": data.radius,
                "angle": data.angle,
            }
        }),
        GeometryKind::EllipticalDish(data) => json!({
            "elliptical_dish": {
                "base_radius": data.base_radius,
                "height": data.height,
            }
        }),
        GeometryKind::SphericalDish(data) => json!({
            "spherical_dish": {
                "base_radius": data.base_radius,
                "height": data.height,
            }
        }),
        GeometryKind::Snout(data) => json!({
            "snout": {
                "radius_bottom": data.radius_bottom,
                "radius_top": data.radius_top,
                "height": data.height,
                "offset_x": data.offset_x,
                "offset_y": data.offset_y,
                "bottom_shear_x": data.bottom_shear_x,
                "bottom_shear_y": data.bottom_shear_y,
                "top_shear_x": data.top_shear_x,
                "top_shear_y": data.top_shear_y,
            }
        }),
        GeometryKind::Cylinder(data) => json!({
            "cylinder": {
                "radius": data.radius,
                "height": data.height,
            }
        }),
        GeometryKind::Sphere(data) => json!({
            "sphere": {
                "radius": data.radius,
            }
        }),
        GeometryKind::Line(data) => json!({
            "line": {
                "start_radius": data.start_radius,
                "end_radius": data.end_radius,
            }
        }),
        GeometryKind::FacetGroup(data) => json!({
            "facet_group": {
                "polygons": data.polygons.iter().map(|polygon| {
                    json!({
                        "contours": polygon.contours.iter().map(|contour| {
                            json!({
                                "vertices": contour.vertices.iter().map(|v| [v.x, v.y, v.z]).collect::<Vec<_>>(),
                                "normals": contour.normals.iter().map(|n| [n.x, n.y, n.z]).collect::<Vec<_>>(),
                            })
                        }).collect::<Vec<_>>()
                    })
                }).collect::<Vec<_>>()
            }
        }),
    }
}

fn geometry_type_name(geo_type: GeometryType) -> &'static str {
    match geo_type {
        GeometryType::Primitive => "Primitive",
        GeometryType::Obstruction => "Obstruction",
        GeometryType::Insulation => "Insulation",
    }
}

fn affine_from_translation(translation: Vec3) -> Affine3A {
    let mut affine = Affine3A::IDENTITY;
    affine.translation = Vec3A::new(translation.x, translation.y, translation.z);
    affine
}

fn encode_affine_blob(affine: &Affine3A) -> Result<Vec<u8>> {
    let matrix = matrix3_to_array(&affine.matrix3);
    let translation = [
        affine.translation.x,
        affine.translation.y,
        affine.translation.z,
    ];
    serialize(&(matrix, translation)).context("序列化 world_matrix 失败")
}

fn matrix3_to_array(matrix: &Mat3A) -> [f32; 9] {
    [
        matrix.x_axis.x,
        matrix.x_axis.y,
        matrix.x_axis.z,
        matrix.y_axis.x,
        matrix.y_axis.y,
        matrix.y_axis.z,
        matrix.z_axis.x,
        matrix.z_axis.y,
        matrix.z_axis.z,
    ]
}

fn translate_bbox(bbox: rvm_rs::math::BBox3, translation: Vec3) -> rvm_rs::math::BBox3 {
    if !bbox.is_valid() {
        return bbox;
    }
    rvm_rs::math::BBox3::from_min_max(bbox.min + translation, bbox.max + translation)
}

fn encode_geometry_blob(
    geometry: &Geometry,
    group_path: &str,
    geometry_index: usize,
    world_translation: Vec3,
    final_bbox: rvm_rs::math::BBox3,
) -> Result<Vec<u8>> {
    let transform = json!({
        "matrix3": matrix3_to_array(&geometry.transform.matrix3),
        "translation": [
            geometry.transform.translation.x + world_translation.x,
            geometry.transform.translation.y + world_translation.y,
            geometry.transform.translation.z + world_translation.z,
        ],
    });
    let bbox_local = json!({
        "min": [geometry.bbox_local.min.x, geometry.bbox_local.min.y, geometry.bbox_local.min.z],
        "max": [geometry.bbox_local.max.x, geometry.bbox_local.max.y, geometry.bbox_local.max.z],
    });
    let bbox_world = json!({
        "min": [final_bbox.min.x, final_bbox.min.y, final_bbox.min.z],
        "max": [final_bbox.max.x, final_bbox.max.y, final_bbox.max.z],
    });
    let payload = json!({
        "source": "rvm-rs",
        "group_path": group_path,
        "geometry_index": geometry_index,
        "kind": geometry_kind_name(&geometry.kind),
        "kind_debug": format!("{:?}", geometry.kind),
        "detail": geometry_detail_payload(&geometry.kind),
        "geo_type": geometry_type_name(geometry.geo_type),
        "color": geometry.color,
        "color_rgb": geometry.color_rgb,
        "transparency": geometry.transparency,
        "sample_start_angle": geometry.sample_start_angle,
        "transform": transform,
        "bbox_local": bbox_local,
        "bbox_world": bbox_world,
    });
    serde_json::to_vec(&payload).context("序列化 inst_geo.geometry 失败")
}
