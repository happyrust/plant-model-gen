use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::str::FromStr;

use aios_core::rs_surreal::geometry_query::PlantTransform;
use aios_core::transform::get_local_mat4;
use aios_core::{RefnoEnum, SurrealQueryExt, Transform, model_primary_db};
use anyhow::{Context, Result};
use futures::StreamExt;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use crate::data_interface::db_meta_manager::db_meta;
use crate::fast_model::gen_model::mesh_generate::fetch_inst_relate_refnos;
use crate::options::{DbOptionExt, get_db_option_ext};

const TRANSFORM_CACHE_DIRNAME: &str = "transforms";
const TRANSFORM_CACHE_VERSION: u32 = 1;
const WORLD_QUERY_CHUNK: usize = 200;
const LOCAL_BUILD_CONCURRENCY: usize = 64;

#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct TransformCacheFileV1 {
    pub version: u32,
    pub dbnum: u32,
    pub source_version: String,
    pub entries: Vec<TransformCacheEntryV1>,
}

#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct TransformCacheEntryV1 {
    pub refno: String,
    pub local: Option<TransformRecordV1>,
    pub world: Option<TransformRecordV1>,
}

#[derive(Debug, Clone, Copy, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct TransformRecordV1 {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl TransformRecordV1 {
    fn from_transform(value: Transform) -> Self {
        Self {
            translation: [
                value.translation.x,
                value.translation.y,
                value.translation.z,
            ],
            rotation: [
                value.rotation.x,
                value.rotation.y,
                value.rotation.z,
                value.rotation.w,
            ],
            scale: [value.scale.x, value.scale.y, value.scale.z],
        }
    }

    fn into_transform(self) -> Transform {
        Transform {
            translation: glam::Vec3::new(
                self.translation[0],
                self.translation[1],
                self.translation[2],
            ),
            rotation: glam::Quat::from_xyzw(
                self.rotation[0],
                self.rotation[1],
                self.rotation[2],
                self.rotation[3],
            ),
            scale: glam::Vec3::new(self.scale[0], self.scale[1], self.scale[2]),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadedTransformDbnum {
    pub source_version: String,
    pub world: HashMap<RefnoEnum, Transform>,
    pub local: HashMap<RefnoEnum, Transform>,
}

fn transform_from_dmat4(m: glam::DMat4) -> Transform {
    let (scale, rot, trans) = m.to_scale_rotation_translation();
    Transform {
        translation: glam::Vec3::new(trans.x as f32, trans.y as f32, trans.z as f32),
        rotation: glam::Quat::from_xyzw(rot.x as f32, rot.y as f32, rot.z as f32, rot.w as f32),
        scale: glam::Vec3::new(scale.x as f32, scale.y as f32, scale.z as f32),
    }
}

fn parse_refno(raw: &str) -> Option<RefnoEnum> {
    RefnoEnum::from_str(raw)
        .or_else(|_| RefnoEnum::from_str(&raw.replace('_', "/")))
        .ok()
}

fn resolve_db_option(db_option: Option<&DbOptionExt>) -> DbOptionExt {
    db_option.cloned().unwrap_or_else(get_db_option_ext)
}

pub(crate) fn transform_cache_dir(db_option: Option<&DbOptionExt>) -> PathBuf {
    resolve_db_option(db_option)
        .get_model_cache_dir()
        .join(TRANSFORM_CACHE_DIRNAME)
}

pub(crate) fn dbnum_cache_path(db_option: Option<&DbOptionExt>, dbnum: u32) -> PathBuf {
    transform_cache_dir(db_option).join(format!("transform_cache_db_{dbnum}.rkyv"))
}

pub(crate) fn source_version_for_dbnum(dbnum: u32) -> String {
    let _ = db_meta().ensure_loaded();
    if let Some(info) = db_meta().get_db_file_info(dbnum) {
        return format!(
            "latest_sesno={};file={};ref0s={}",
            info.latest_sesno,
            info.file_name,
            info.ref0s.len()
        );
    }
    format!("dbnum={dbnum};latest_sesno=unknown")
}

fn decode_cache_file(file: TransformCacheFileV1) -> LoadedTransformDbnum {
    let mut world = HashMap::with_capacity(file.entries.len());
    let mut local = HashMap::with_capacity(file.entries.len());
    for entry in file.entries {
        let Some(refno) = parse_refno(&entry.refno) else {
            log::warn!(
                "[transform_rkyv_cache] 跳过无法解析的 refno: {}",
                entry.refno
            );
            continue;
        };
        if let Some(value) = entry.world {
            world.insert(refno, value.into_transform());
        }
        if let Some(value) = entry.local {
            local.insert(refno, value.into_transform());
        }
    }
    LoadedTransformDbnum {
        source_version: file.source_version,
        world,
        local,
    }
}

pub(crate) fn load_dbnum_cache_if_fresh(
    db_option: Option<&DbOptionExt>,
    dbnum: u32,
) -> Result<Option<LoadedTransformDbnum>> {
    let path = dbnum_cache_path(db_option, dbnum);
    let data = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("读取 transform rkyv 文件失败: {}", path.display()));
        }
    };

    let file: TransformCacheFileV1 =
        rkyv::from_bytes::<TransformCacheFileV1, rkyv::rancor::Error>(&data)
            .map_err(|e| anyhow::anyhow!("transform rkyv 反序列化失败: {:?}", e))?;

    let expected_source = source_version_for_dbnum(dbnum);
    if file.version != TRANSFORM_CACHE_VERSION
        || file.dbnum != dbnum
        || file.source_version != expected_source
    {
        return Ok(None);
    }

    Ok(Some(decode_cache_file(file)))
}

fn write_cache_file(db_option: Option<&DbOptionExt>, file: &TransformCacheFileV1) -> Result<()> {
    let dir = transform_cache_dir(db_option);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建 transform cache 目录失败: {}", dir.display()))?;
    let path = dbnum_cache_path(db_option, file.dbnum);
    let tmp_path = path.with_extension("rkyv.tmp");
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(file)
        .map_err(|e| anyhow::anyhow!("transform rkyv 序列化失败: {:?}", e))?;
    std::fs::write(&tmp_path, &bytes)
        .with_context(|| format!("写入 transform 临时缓存失败: {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("落盘 transform 缓存失败: {}", path.display()))?;
    Ok(())
}

pub(crate) async fn query_world_transforms_from_pe_transform(
    refnos: &[RefnoEnum],
) -> Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    crate::fast_model::utils::ensure_surreal_init().await?;

    #[derive(Debug, Deserialize, SurrealValue)]
    struct PeWorldTransRow {
        #[serde(default)]
        refno: Option<RefnoEnum>,
        #[serde(default)]
        world_trans: Option<PlantTransform>,
    }

    let mut out = HashMap::with_capacity(refnos.len());
    for chunk in refnos.chunks(WORLD_QUERY_CHUNK) {
        let pt_ids = chunk
            .iter()
            .map(|r| r.to_table_key("pe_transform"))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            r#"
            SELECT
                record::id(id) as refno,
                world_trans.d as world_trans
            FROM [{pt_ids}];
            "#
        );

        let rows: Vec<PeWorldTransRow> = model_primary_db().query_take(&sql, 0).await?;
        for row in rows {
            let Some(refno) = row.refno else { continue };
            let Some(world) = row.world_trans else {
                continue;
            };
            out.insert(refno, world.0);
        }
    }

    Ok(out)
}

async fn build_local_transform_map(refnos: &[RefnoEnum]) -> HashMap<RefnoEnum, Transform> {
    if refnos.is_empty() {
        return HashMap::new();
    }

    let mut stream = futures::stream::iter(refnos.iter().copied().map(|refno| async move {
        let local = get_local_mat4(refno)
            .await
            .ok()
            .flatten()
            .map(transform_from_dmat4);
        (refno, local)
    }))
    .buffer_unordered(LOCAL_BUILD_CONCURRENCY);

    let mut out = HashMap::with_capacity(refnos.len());
    while let Some((refno, local)) = stream.next().await {
        if let Some(value) = local {
            out.insert(refno, value);
        }
    }
    out
}

async fn collect_refnos_for_dbnum(dbnum: u32) -> Result<Vec<RefnoEnum>> {
    db_meta().ensure_loaded()?;
    let mut refnos = fetch_inst_relate_refnos().await?;
    refnos.retain(|refno| db_meta().get_dbnum_by_refno(*refno) == Some(dbnum));
    refnos.sort_by_key(|refno| refno.to_string());
    refnos.dedup();
    Ok(refnos)
}

async fn build_dbnum_cache_file(
    db_option: Option<&DbOptionExt>,
    dbnum: u32,
) -> Result<TransformCacheFileV1> {
    let refnos = collect_refnos_for_dbnum(dbnum).await?;
    let mut world = query_world_transforms_from_pe_transform(&refnos).await?;

    let world_missing: Vec<RefnoEnum> = refnos
        .iter()
        .copied()
        .filter(|refno| !world.contains_key(refno))
        .collect();
    for refno in world_missing {
        if let Ok(Some(value)) = aios_core::get_world_transform(refno).await {
            world.insert(refno, value);
        }
    }

    let mut local = build_local_transform_map(&refnos).await;

    let mut entry_refnos: HashSet<RefnoEnum> = refnos.iter().copied().collect();
    entry_refnos.extend(world.keys().copied());
    entry_refnos.extend(local.keys().copied());
    let mut ordered_refnos: Vec<RefnoEnum> = entry_refnos.into_iter().collect();
    ordered_refnos.sort_by_key(|refno| refno.to_string());

    let mut entries = Vec::with_capacity(ordered_refnos.len());
    for refno in ordered_refnos {
        entries.push(TransformCacheEntryV1 {
            refno: refno.to_string(),
            local: local.remove(&refno).map(TransformRecordV1::from_transform),
            world: world.remove(&refno).map(TransformRecordV1::from_transform),
        });
    }

    let file = TransformCacheFileV1 {
        version: TRANSFORM_CACHE_VERSION,
        dbnum,
        source_version: source_version_for_dbnum(dbnum),
        entries,
    };
    write_cache_file(db_option, &file)?;
    Ok(file)
}

pub(crate) async fn load_or_build_dbnum_cache(
    db_option: Option<&DbOptionExt>,
    dbnum: u32,
) -> Result<LoadedTransformDbnum> {
    if let Some(loaded) = load_dbnum_cache_if_fresh(db_option, dbnum)? {
        return Ok(loaded);
    }

    let file = build_dbnum_cache_file(db_option, dbnum)
        .await
        .with_context(|| format!("构建 transform rkyv 失败: dbnum={dbnum}"))?;
    Ok(decode_cache_file(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_record_roundtrip() {
        let src = Transform {
            translation: glam::Vec3::new(1.0, 2.0, 3.0),
            rotation: glam::Quat::from_xyzw(0.0, 0.5, 0.0, 0.8660254),
            scale: glam::Vec3::new(4.0, 5.0, 6.0),
        };

        let record = TransformRecordV1::from_transform(src);
        let dst = record.into_transform();
        assert_eq!(dst.translation, src.translation);
        assert_eq!(dst.scale, src.scale);
        assert_eq!(dst.rotation, src.rotation);
    }

    #[test]
    fn decode_cache_file_loads_valid_entry() {
        let file = TransformCacheFileV1 {
            version: TRANSFORM_CACHE_VERSION,
            dbnum: 1,
            source_version: "v1".into(),
            entries: vec![TransformCacheEntryV1 {
                refno: "24381/36716".into(),
                local: Some(TransformRecordV1 {
                    translation: [1.0, 2.0, 3.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                }),
                world: Some(TransformRecordV1 {
                    translation: [4.0, 5.0, 6.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                }),
            }],
        };

        let decoded = decode_cache_file(file);
        assert_eq!(decoded.local.len(), 1);
        assert_eq!(decoded.world.len(), 1);
    }
}
