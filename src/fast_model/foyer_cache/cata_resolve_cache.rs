//! CATA `resolve_desi_comp` 产物缓存（按 cata_hash）
//!
//! 目标：将 CATE 元件库的“可复用几何准备结果”落到 foyer cache，避免后续运行重复调用
//! `resolve_desi_comp -> try_convert -> unit 参数/scale 归一` 这一整段链路。
//!
//! 注意：
//! - 缓存粒度：`cata_hash`（同组 design_refno 共享）。
//! - 迁移策略（方案1）：只读 rkyv payload；旧 JSON / schema 不匹配一律视为 miss，由上游重建并回灌。
//! - payload 使用 `rkyv_payload`，带 type_tag/schema/header hash 校验。

use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};

use aios_core::geometry::GeoBasicType;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::parsed_data::CateAxisParam;
use aios_core::RefnoEnum;
use aios_core::Transform;
use glam::Vec3;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
use twox_hash::XxHash64;

use super::rkyv_payload;

// ---------------------------------------------------------------------------
// rkyv payload（V1 schema）
// ---------------------------------------------------------------------------

const CATA_RESOLVE_TYPE_TAG: u16 = 3001;
const CATA_RESOLVE_SCHEMA_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct TransformV1 {
    t: [f32; 3],
    r: [f32; 4], // x,y,z,w
    s: [f32; 3],
}

impl From<Transform> for TransformV1 {
    fn from(v: Transform) -> Self {
        Self {
            t: [v.translation.x, v.translation.y, v.translation.z],
            r: [v.rotation.x, v.rotation.y, v.rotation.z, v.rotation.w],
            s: [v.scale.x, v.scale.y, v.scale.z],
        }
    }
}

impl From<TransformV1> for Transform {
    fn from(v: TransformV1) -> Self {
        Transform {
            translation: Vec3::new(v.t[0], v.t[1], v.t[2]),
            rotation: glam::Quat::from_xyzw(v.r[0], v.r[1], v.r[2], v.r[3]),
            scale: Vec3::new(v.s[0], v.s[1], v.s[2]),
        }
    }
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct PreparedInstGeoV1 {
    geo_hash: u64,
    geom_refno: RefnoEnum,
    pts: Vec<i32>,
    geo_transform: TransformV1,
    geo_param: PdmsGeoParam,
    /// shape 原始可见性（来自 TUFL/tube_flag），用于 `AIOS_RESPECT_TUFL` 过滤
    shape_visible: bool,
    is_tubi: bool,
    geo_type: GeoBasicType,
    /// 是否使用 unit 参数（true=geo_param 已写入 unit param；transform.scale 保留）
    unit_flag: bool,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct CataResolvedCompV1 {
    created_at: i64,
    /// `ptset_map` 的序列化镜像（避免 BTreeMap Archive 兼容性问题）
    ptset_items: Vec<(i32, CateAxisParam)>,
    geos: Vec<PreparedInstGeoV1>,
    has_solid: bool,
}

// ---------------------------------------------------------------------------
// 对外数据结构
// ---------------------------------------------------------------------------

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct CataResolveCacheKey {
    pub cata_hash: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CataResolveCacheValue {
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PreparedInstGeo {
    pub geo_hash: u64,
    pub geom_refno: RefnoEnum,
    pub pts: Vec<i32>,
    pub geo_transform: Transform,
    pub geo_param: PdmsGeoParam,
    pub shape_visible: bool,
    pub is_tubi: bool,
    pub geo_type: GeoBasicType,
    pub unit_flag: bool,
}

#[derive(Clone, Debug)]
pub struct CataResolvedComp {
    pub created_at: i64,
    pub ptset_items: Vec<(i32, CateAxisParam)>,
    pub geos: Vec<PreparedInstGeo>,
    pub has_solid: bool,
}

impl CataResolvedComp {
    #[inline]
    pub fn ptset_map(&self) -> std::collections::BTreeMap<i32, CateAxisParam> {
        self.ptset_items.iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Cache Manager
// ---------------------------------------------------------------------------

pub struct CataResolveCacheManager {
    cache: HybridCache<CataResolveCacheKey, CataResolveCacheValue, BuildHasherDefault<XxHash64>>,
    cache_dir: PathBuf,
}

impl CataResolveCacheManager {
    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let device_config = DirectFsDeviceOptionsBuilder::new(cache_dir)
            // resolve_desi_comp 产物一般比 geom_input 小，但条目数量可能更多，给一个中等容量即可
            .with_capacity(256 * 1024 * 1024)
            .build();

        let cache = HybridCacheBuilder::new()
            .memory(64 * 1024 * 1024)
            .with_hash_builder(BuildHasherDefault::<XxHash64>::default())
            .storage()
            .with_device_config(device_config)
            .build()
            .await?;

        Ok(Self {
            cache,
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn insert(&self, cata_hash: String, value: &CataResolvedComp) {
        let key = CataResolveCacheKey { cata_hash };
        let v1 = CataResolvedCompV1 {
            created_at: value.created_at,
            ptset_items: value.ptset_items.clone(),
            geos: value
                .geos
                .iter()
                .cloned()
                .map(|g| PreparedInstGeoV1 {
                    geo_hash: g.geo_hash,
                    geom_refno: g.geom_refno,
                    pts: g.pts,
                    geo_transform: g.geo_transform.into(),
                    geo_param: g.geo_param,
                    shape_visible: g.shape_visible,
                    is_tubi: g.is_tubi,
                    geo_type: g.geo_type,
                    unit_flag: g.unit_flag,
                })
                .collect(),
            has_solid: value.has_solid,
        };

        let payload = match rkyv_payload::encode(CATA_RESOLVE_TYPE_TAG, CATA_RESOLVE_SCHEMA_V1, &v1)
        {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[cata_resolve_cache] rkyv 序列化失败: cata_hash={}, err={}",
                    key.cata_hash, e
                );
                return;
            }
        };

        self.cache.insert(key, CataResolveCacheValue { payload });
    }

    pub async fn get(&self, cata_hash: &str) -> Option<CataResolvedComp> {
        let key = CataResolveCacheKey {
            cata_hash: cata_hash.to_string(),
        };
        match self.cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                let v1 = match rkyv_payload::decode::<CataResolvedCompV1>(
                    CATA_RESOLVE_TYPE_TAG,
                    CATA_RESOLVE_SCHEMA_V1,
                    payload,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        // 迁移策略（方案1）：旧 JSON payload / schema 不匹配 一律视为 miss。
                        eprintln!(
                            "[cata_resolve_cache] payload decode miss: cata_hash={}, err={}",
                            cata_hash, e
                        );
                        return None;
                    }
                };

                Some(CataResolvedComp {
                    created_at: v1.created_at,
                    ptset_items: v1.ptset_items,
                    geos: v1
                        .geos
                        .into_iter()
                        .map(|g| PreparedInstGeo {
                            geo_hash: g.geo_hash,
                            geom_refno: g.geom_refno,
                            pts: g.pts,
                            geo_transform: g.geo_transform.into(),
                            geo_param: g.geo_param,
                            shape_visible: g.shape_visible,
                            is_tubi: g.is_tubi,
                            geo_type: g.geo_type,
                            unit_flag: g.unit_flag,
                        })
                        .collect(),
                    has_solid: v1.has_solid,
                })
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!("[cata_resolve_cache] 读取失败: cata_hash={}, err={}", cata_hash, e);
                None
            }
        }
    }
}

