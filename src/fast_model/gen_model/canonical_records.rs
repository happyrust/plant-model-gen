use std::collections::{BTreeMap, HashMap};

use aios_core::geometry::{GeoBasicType, ShapeInstancesData};
use aios_core::{gen_aabb_hash, gen_plant_transform_hash, gen_string_hash};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CanonicalRawTable {
    RawInstInfo,
    RawInstRelate,
    RawInstGeo,
    RawGeoRelate,
    RawTubiInfo,
    RawTubiRelate,
    RawNegRelate,
    RawNgmrRelate,
    RawAabb,
    RawTrans,
    RawVec3,
    RawInstRelateAabb,
    RawRefnoAssocIndex,
}

impl CanonicalRawTable {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RawInstInfo => "raw_inst_info",
            Self::RawInstRelate => "raw_inst_relate",
            Self::RawInstGeo => "raw_inst_geo",
            Self::RawGeoRelate => "raw_geo_relate",
            Self::RawTubiInfo => "raw_tubi_info",
            Self::RawTubiRelate => "raw_tubi_relate",
            Self::RawNegRelate => "raw_neg_relate",
            Self::RawNgmrRelate => "raw_ngmr_relate",
            Self::RawAabb => "raw_aabb",
            Self::RawTrans => "raw_trans",
            Self::RawVec3 => "raw_vec3",
            Self::RawInstRelateAabb => "raw_inst_relate_aabb",
            Self::RawRefnoAssocIndex => "raw_refno_assoc_index",
        }
    }

    pub fn phase1_limitation(self) -> Option<&'static str> {
        match self {
            Self::RawRefnoAssocIndex => Some(
                "Phase 1 retains refno_assoc_index as a raw table for delete/index parity, but runtime materialization is intentionally disabled.",
            ),
            _ => None,
        }
    }

    pub const fn all_phase1() -> &'static [Self] {
        &[
            Self::RawInstInfo,
            Self::RawInstRelate,
            Self::RawInstGeo,
            Self::RawGeoRelate,
            Self::RawTubiInfo,
            Self::RawTubiRelate,
            Self::RawNegRelate,
            Self::RawNgmrRelate,
            Self::RawAabb,
            Self::RawTrans,
            Self::RawVec3,
            Self::RawInstRelateAabb,
            Self::RawRefnoAssocIndex,
        ]
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CanonicalRawRowCounts {
    counts: BTreeMap<String, usize>,
    limitations: BTreeMap<String, Option<String>>,
}

impl CanonicalRawRowCounts {
    pub fn set(&mut self, table: CanonicalRawTable, count: usize) {
        self.counts.insert(table.as_str().to_owned(), count);
        self.limitations.insert(
            table.as_str().to_owned(),
            table.phase1_limitation().map(str::to_owned),
        );
    }

    pub fn get(&self, table: CanonicalRawTable) -> usize {
        self.counts.get(table.as_str()).copied().unwrap_or(0)
    }

    pub fn as_map(&self) -> &BTreeMap<String, usize> {
        &self.counts
    }

    pub fn limitation(&self, table: CanonicalRawTable) -> Option<&str> {
        self.limitations
            .get(table.as_str())
            .and_then(|value| value.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalRawPayload {
    pub surreal_json: Option<String>,
    pub limitation: Option<String>,
}

impl CanonicalRawPayload {
    fn surreal_json(value: String) -> Self {
        Self {
            surreal_json: Some(value),
            limitation: None,
        }
    }

    fn placeholder(reason: &'static str) -> Self {
        Self {
            surreal_json: None,
            limitation: Some(reason.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawInstInfoRecord {
    pub refno: String,
    pub inst_id: String,
    pub inst_relate_id: String,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawInstRelateRecord {
    pub refno: String,
    pub pe_id: String,
    pub inst_id: String,
    pub relation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawInstGeoRecord {
    pub owner_refno: String,
    pub inst_id: String,
    pub geom_refno: String,
    pub geo_hash: u64,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawGeoRelateRecord {
    pub relation_id: u64,
    pub inst_id: String,
    pub geo_hash: u64,
    pub geom_refno: String,
    pub geo_type: String,
    pub trans_id: u64,
    pub visible: bool,
    pub pts_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawTubiInfoRecord {
    pub tubi_id: String,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawTubiRelateRecord {
    pub refno: String,
    pub branch_id: String,
    pub aabb_id: Option<u64>,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawDependencyRelateRecord {
    pub carrier_refno: String,
    pub target_refno: String,
    pub geom_refno: Option<String>,
    pub relation_kind: String,
    pub geo_relate_ids: Vec<u64>,
    pub pending: bool,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAabbRecord {
    pub aabb_id: u64,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawTransRecord {
    pub trans_id: u64,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawVec3Record {
    pub vec3_id: u64,
    pub payload: CanonicalRawPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawInstRelateAabbRecord {
    pub refno: String,
    pub relation_id: String,
    pub aabb_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRefnoAssocIndexRecord {
    pub refno: String,
    pub inst_relate_ids: Vec<String>,
    pub inst_info_ids: Vec<String>,
    pub geo_relate_ids: Vec<String>,
    pub geo_hashes: Vec<String>,
    pub neg_relate_ids: Vec<String>,
    pub ngmr_relate_ids: Vec<String>,
    pub inst_relate_aabb_ids: Vec<String>,
    pub tubi_branch_keys: Vec<String>,
    pub pending_phase2_inst_relate_bool_ids: Vec<String>,
    pub pending_phase2_inst_relate_cata_bool_ids: Vec<String>,
    pub unsupported_inst_relate_booled_aabb_ids: Vec<String>,
    pub limitation: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CanonicalRawBatch {
    pub inst_info: Vec<RawInstInfoRecord>,
    pub inst_relate: Vec<RawInstRelateRecord>,
    pub inst_geo: Vec<RawInstGeoRecord>,
    pub geo_relate: Vec<RawGeoRelateRecord>,
    pub tubi_info: Vec<RawTubiInfoRecord>,
    pub tubi_relate: Vec<RawTubiRelateRecord>,
    pub neg_relate: Vec<RawDependencyRelateRecord>,
    pub ngmr_relate: Vec<RawDependencyRelateRecord>,
    pub aabb: Vec<RawAabbRecord>,
    pub trans: Vec<RawTransRecord>,
    pub vec3: Vec<RawVec3Record>,
    pub inst_relate_aabb: Vec<RawInstRelateAabbRecord>,
    pub refno_assoc_index: Vec<RawRefnoAssocIndexRecord>,
    pub row_counts: CanonicalRawRowCounts,
}

impl CanonicalRawBatch {
    pub fn refresh_row_counts(&mut self) {
        let mut counts = CanonicalRawRowCounts::default();
        counts.set(CanonicalRawTable::RawInstInfo, self.inst_info.len());
        counts.set(CanonicalRawTable::RawInstRelate, self.inst_relate.len());
        counts.set(CanonicalRawTable::RawInstGeo, self.inst_geo.len());
        counts.set(CanonicalRawTable::RawGeoRelate, self.geo_relate.len());
        counts.set(CanonicalRawTable::RawTubiInfo, self.tubi_info.len());
        counts.set(CanonicalRawTable::RawTubiRelate, self.tubi_relate.len());
        counts.set(CanonicalRawTable::RawNegRelate, self.neg_relate.len());
        counts.set(CanonicalRawTable::RawNgmrRelate, self.ngmr_relate.len());
        counts.set(CanonicalRawTable::RawAabb, self.aabb.len());
        counts.set(CanonicalRawTable::RawTrans, self.trans.len());
        counts.set(CanonicalRawTable::RawVec3, self.vec3.len());
        counts.set(
            CanonicalRawTable::RawInstRelateAabb,
            self.inst_relate_aabb.len(),
        );
        counts.set(
            CanonicalRawTable::RawRefnoAssocIndex,
            self.refno_assoc_index.len(),
        );
        self.row_counts = counts;
    }
}

#[derive(Debug, Clone, Default)]
pub struct CanonicalRawPlanner;

impl CanonicalRawPlanner {
    pub fn plan_shape_instances(&self, shape_insts: &ShapeInstancesData) -> CanonicalRawBatch {
        let mut batch = CanonicalRawBatch::default();
        let mut aabb_by_hash: HashMap<u64, String> = HashMap::new();
        let mut trans_by_hash: HashMap<u64, String> = HashMap::new();
        let mut vec3_by_hash: HashMap<u64, String> = HashMap::new();
        let mut neg_geo_by_carrier: HashMap<String, Vec<(u64, String)>> = HashMap::new();
        let mut ngmr_geo_by_key: HashMap<(String, String), Vec<(u64, String)>> = HashMap::new();

        for (refno, info) in &shape_insts.inst_info_map {
            let refno_s = refno.to_string();
            let inst_id = info.id_str();
            batch.inst_info.push(RawInstInfoRecord {
                refno: refno_s.clone(),
                inst_id: inst_id.clone(),
                inst_relate_id: refno.to_inst_relate_key(),
                payload: CanonicalRawPayload::surreal_json(info.gen_sur_json_full()),
            });
            batch.inst_relate.push(RawInstRelateRecord {
                refno: refno_s,
                pe_id: refno.to_pe_key(),
                inst_id,
                relation_id: refno.to_inst_relate_key(),
            });

            if let Some(aabb) = info.aabb {
                let hash = gen_aabb_hash(&aabb);
                aabb_by_hash
                    .entry(hash)
                    .or_insert_with(|| serde_json::to_string(&aabb).unwrap_or_default());
                batch.inst_relate_aabb.push(RawInstRelateAabbRecord {
                    refno: refno.to_string(),
                    relation_id: refno.to_table_key("inst_relate_aabb"),
                    aabb_id: hash,
                });
            }
        }

        for inst_geo_data in shape_insts.inst_geos_map.values() {
            let inst_id = inst_geo_data.id();
            let owner_refno = inst_geo_data.refno.to_string();
            for inst in &inst_geo_data.insts {
                if inst.geo_transform.translation.is_nan()
                    || inst.geo_transform.rotation.is_nan()
                    || inst.geo_transform.scale.is_nan()
                {
                    continue;
                }

                let trans_id = gen_plant_transform_hash(&inst.geo_transform);
                trans_by_hash.entry(trans_id).or_insert_with(|| {
                    serde_json::to_string(&inst.geo_transform).unwrap_or_default()
                });

                let mut pts_ids = Vec::new();
                for key_pt in inst.geo_param.key_points() {
                    let pts_hash = key_pt.gen_hash();
                    pts_ids.push(pts_hash);
                    vec3_by_hash
                        .entry(pts_hash)
                        .or_insert_with(|| serde_json::to_string(&key_pt).unwrap_or_default());
                }

                let cat_negs_str = if !inst.cata_neg_refnos.is_empty() {
                    format!(
                        ", cata_neg: [{}]",
                        inst.cata_neg_refnos
                            .iter()
                            .map(|x| x.to_pe_key())
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                } else {
                    String::new()
                };
                let pts_key_list = pts_ids
                    .iter()
                    .map(|hash| format!("vec3:⟨{}⟩", hash))
                    .collect::<Vec<_>>()
                    .join(",");
                let relate_json = format!(
                    r#"in: inst_info:⟨{0}⟩, out: inst_geo:⟨{1}⟩, trans: trans:⟨{2}⟩, geom_refno: pe:{3}, pts: [{4}], geo_type: '{5}', visible: {6} {7}"#,
                    inst_id,
                    inst.geo_hash,
                    trans_id,
                    inst.refno,
                    pts_key_list,
                    inst.geo_type,
                    inst.visible,
                    cat_negs_str
                );
                let relation_id = gen_string_hash(&relate_json);

                batch.inst_geo.push(RawInstGeoRecord {
                    owner_refno: owner_refno.clone(),
                    inst_id: inst_id.clone(),
                    geom_refno: inst.refno.to_string(),
                    geo_hash: inst.geo_hash,
                    payload: CanonicalRawPayload::surreal_json(inst.gen_unit_geo_sur_json()),
                });
                batch.geo_relate.push(RawGeoRelateRecord {
                    relation_id,
                    inst_id: inst_id.clone(),
                    geo_hash: inst.geo_hash,
                    geom_refno: inst.refno.to_string(),
                    geo_type: inst.geo_type.to_string(),
                    trans_id,
                    visible: inst.visible,
                    pts_ids,
                });

                match inst.geo_type {
                    GeoBasicType::Neg => {
                        neg_geo_by_carrier
                            .entry(inst_geo_data.refno.to_string())
                            .or_default()
                            .push((relation_id, inst.refno.to_pe_key()));
                    }
                    GeoBasicType::CataCrossNeg => {
                        ngmr_geo_by_key
                            .entry((inst_geo_data.refno.to_string(), inst.refno.to_string()))
                            .or_default()
                            .push((relation_id, inst.refno.to_pe_key()));
                    }
                    _ => {}
                }
            }
        }

        for (target, carriers) in &shape_insts.neg_relate_map {
            for carrier in carriers {
                let carrier_key = carrier.to_string();
                let matching_geo_relates = neg_geo_by_carrier
                    .get(&carrier_key)
                    .cloned()
                    .unwrap_or_default();
                let geo_relate_ids = matching_geo_relates
                    .iter()
                    .map(|(relation_id, _)| *relation_id)
                    .collect::<Vec<_>>();
                batch.neg_relate.push(RawDependencyRelateRecord {
                    carrier_refno: carrier_key,
                    target_refno: target.to_string(),
                    geom_refno: None,
                    relation_kind: "neg_relate".to_owned(),
                    pending: geo_relate_ids.is_empty(),
                    geo_relate_ids,
                    payload: CanonicalRawPayload::placeholder(
                        "Surreal neg_relate rows require geo_relate ids that may be loaded from DB across batches.",
                    ),
                });
            }
        }

        for (target, ngmrs) in &shape_insts.ngmr_neg_relate_map {
            for (carrier, geom_refno) in ngmrs {
                let key = (carrier.to_string(), geom_refno.to_string());
                let matching_geo_relates = ngmr_geo_by_key.get(&key).cloned().unwrap_or_default();
                let geo_relate_ids = matching_geo_relates
                    .iter()
                    .map(|(relation_id, _)| *relation_id)
                    .collect::<Vec<_>>();
                batch.ngmr_relate.push(RawDependencyRelateRecord {
                    carrier_refno: carrier.to_string(),
                    target_refno: target.to_string(),
                    geom_refno: Some(geom_refno.to_string()),
                    relation_kind: "ngmr_relate".to_owned(),
                    pending: geo_relate_ids.is_empty(),
                    geo_relate_ids,
                    payload: CanonicalRawPayload::placeholder(
                        "Surreal ngmr_relate rows require matching CataCrossNeg geo_relate ids in the same or prior batch.",
                    ),
                });
            }
        }

        for (refno, tubi) in &shape_insts.inst_tubi_map {
            let aabb_id = tubi.aabb.map(|aabb| {
                let hash = gen_aabb_hash(&aabb);
                aabb_by_hash
                    .entry(hash)
                    .or_insert_with(|| serde_json::to_string(&aabb).unwrap_or_default());
                hash
            });
            batch.tubi_relate.push(RawTubiRelateRecord {
                refno: refno.to_string(),
                branch_id: tubi.refno.to_pe_key(),
                aabb_id,
                payload: CanonicalRawPayload::placeholder(
                    "ShapeInstancesData carries tubing instance rows, but not the global tubi_info collector payload.",
                ),
            });
        }

        batch.aabb = aabb_by_hash
            .into_iter()
            .map(|(aabb_id, payload)| RawAabbRecord {
                aabb_id,
                payload: CanonicalRawPayload::surreal_json(payload),
            })
            .collect();
        batch.trans = trans_by_hash
            .into_iter()
            .map(|(trans_id, payload)| RawTransRecord {
                trans_id,
                payload: CanonicalRawPayload::surreal_json(payload),
            })
            .collect();
        batch.vec3 = vec3_by_hash
            .into_iter()
            .map(|(vec3_id, payload)| RawVec3Record {
                vec3_id,
                payload: CanonicalRawPayload::surreal_json(payload),
            })
            .collect();

        for refno in shape_insts.inst_info_map.keys() {
            let refno_s = refno.to_string();
            let inst_ids = shape_insts
                .inst_geos_map
                .values()
                .filter(|geos| geos.refno == *refno)
                .map(|geos| geos.id())
                .collect::<Vec<_>>();
            let inst_info_ids = shape_insts
                .inst_info_map
                .get(refno)
                .map(|info| vec![format!("inst_info:⟨{}⟩", info.id_str())])
                .unwrap_or_default();
            let geo_relate_ids = batch
                .geo_relate
                .iter()
                .filter(|row| inst_ids.iter().any(|inst_id| inst_id == &row.inst_id))
                .map(|row| format!("geo_relate:⟨{}⟩", row.relation_id))
                .collect::<Vec<_>>();
            let geo_hashes = shape_insts
                .inst_geos_map
                .values()
                .filter(|geos| geos.refno == *refno)
                .flat_map(|geos| geos.insts.iter().map(|inst| inst.geo_hash.to_string()))
                .collect::<Vec<_>>();
            let neg_relate_ids = neg_geo_by_carrier
                .get(&refno_s)
                .into_iter()
                .flatten()
                .flat_map(|(relation_id, _)| {
                    shape_insts
                        .neg_relate_map
                        .iter()
                        .filter(|(_, carriers)| carriers.iter().any(|carrier| carrier == refno))
                        .map(move |(target, _)| {
                            format!("neg_relate:['{}',{}]", relation_id, target.to_pe_key())
                        })
                })
                .collect::<Vec<_>>();
            let ngmr_relate_ids = ngmr_geo_by_key
                .iter()
                .filter(|((carrier, _), _)| carrier == &refno_s)
                .flat_map(|((_, geom_refno), geo_relates)| {
                    geo_relates.iter().flat_map(move |(relation_id, _)| {
                        shape_insts
                            .ngmr_neg_relate_map
                            .iter()
                            .filter_map(move |(target, ngmrs)| {
                                ngmrs
                                    .iter()
                                    .any(|(carrier, geom)| {
                                        carrier == refno && geom.to_string() == *geom_refno
                                    })
                                    .then(|| {
                                        format!(
                                            "ngmr_relate:['{}',{}]",
                                            relation_id,
                                            target.to_pe_key()
                                        )
                                    })
                            })
                    })
                })
                .collect::<Vec<_>>();
            let inst_relate_aabb_ids = batch
                .inst_relate_aabb
                .iter()
                .filter(|row| row.refno == refno_s)
                .map(|row| row.relation_id.clone())
                .collect::<Vec<_>>();
            let tubi_branch_keys = shape_insts
                .inst_tubi_map
                .get(refno)
                .map(|tubi| vec![tubi.refno.to_pe_key()])
                .unwrap_or_default();
            batch.refno_assoc_index.push(RawRefnoAssocIndexRecord {
                refno: refno_s,
                inst_relate_ids: vec![refno.to_inst_relate_key()],
                inst_info_ids,
                geo_relate_ids,
                geo_hashes,
                neg_relate_ids,
                ngmr_relate_ids,
                inst_relate_aabb_ids,
                tubi_branch_keys,
                pending_phase2_inst_relate_bool_ids: vec![format!("inst_relate_bool:⟨{}⟩", refno)],
                pending_phase2_inst_relate_cata_bool_ids: Vec::new(),
                unsupported_inst_relate_booled_aabb_ids: Vec::new(),
                limitation: CanonicalRawTable::RawRefnoAssocIndex
                    .phase1_limitation()
                    .map(str::to_owned),
            });
        }
        batch.refresh_row_counts();
        batch
    }
}
