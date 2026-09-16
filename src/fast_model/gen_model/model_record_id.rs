use aios_core::{RefnoEnum, tool::hash_tool::hash_str};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub struct ModelRefnoIdParts {
    pub ref0: u32,
    pub ref1: u32,
}

pub fn refno_id_parts(refno: RefnoEnum) -> ModelRefnoIdParts {
    let base = refno.refno();
    ModelRefnoIdParts {
        ref0: base.get_0(),
        ref1: base.get_1(),
    }
}

pub fn model_refno_id(table: &str, refno: RefnoEnum) -> String {
    let parts = refno_id_parts(refno);
    model_refno_id_from_parts(table, parts)
}

pub fn model_refno_id_from_parts(table: &str, parts: ModelRefnoIdParts) -> String {
    format!("{table}:[{},{}]", parts.ref0, parts.ref1)
}

pub fn geo_relate_id(carrier: RefnoEnum, geo_index: usize) -> String {
    let parts = refno_id_parts(carrier);
    format!("geo_relate:[{},{},{}]", parts.ref0, parts.ref1, geo_index)
}

pub fn geo_relate_id_for_inst(carrier: RefnoEnum, geo_index: usize, inst_info_id: &str) -> String {
    let parts = refno_id_parts(carrier);
    // SurrealDB 数组 record id 的整数元素按 i64 解析，u64 哈希可能超过 i64::MAX
    // 导致 "number cannot fit within a 64bit signed integer"，故掩码到 63 位非负范围。
    let inst_id_hash = hash_str(inst_info_id) & (i64::MAX as u64);
    format!(
        "geo_relate:[{},{},{},{}]",
        parts.ref0, parts.ref1, geo_index, inst_id_hash
    )
}

pub fn neg_relate_id(
    target: RefnoEnum,
    carrier: RefnoEnum,
    geo_index: usize,
    neg_index: usize,
) -> String {
    target_owned_relation_id("neg_relate", target, carrier, geo_index, neg_index)
}

pub fn ngmr_relate_id(
    target: RefnoEnum,
    carrier: RefnoEnum,
    geo_index: usize,
    ngmr_index: usize,
) -> String {
    target_owned_relation_id("ngmr_relate", target, carrier, geo_index, ngmr_index)
}

pub fn tubi_relate_id(branch_refno: RefnoEnum, tubi_index: usize) -> String {
    let parts = refno_id_parts(branch_refno);
    format!("tubi_relate:[{},{},{}]", parts.ref0, parts.ref1, tubi_index)
}

fn target_owned_relation_id(
    table: &str,
    target: RefnoEnum,
    carrier: RefnoEnum,
    geo_index: usize,
    relation_index: usize,
) -> String {
    let target = refno_id_parts(target);
    let carrier = refno_id_parts(carrier);
    format!(
        "{table}:[{},{},{},{},{},{}]",
        target.ref0, target.ref1, carrier.ref0, carrier.ref1, geo_index, relation_index
    )
}

pub fn model_ref0_range(table: &str, ref0: u32) -> String {
    format!("{table}:[{ref0}, NONE]..=[{ref0}, ..]")
}

pub fn model_refno_range(table: &str, refno: RefnoEnum) -> String {
    let parts = refno_id_parts(refno);
    format!(
        "{table}:[{}, {}, NONE]..=[{}, {}, ..]",
        parts.ref0, parts.ref1, parts.ref0, parts.ref1
    )
}

#[derive(Debug, Serialize)]
pub struct ModelRecordIdEvidence {
    pub input_refno: String,
    pub parts: ModelRefnoIdParts,
    pub inst_relate: String,
    pub inst_relate_aabb: String,
    pub inst_relate_bool: String,
    pub inst_relate_cata_bool: String,
    pub refno_relations: String,
    pub geo_relate_0: String,
    pub neg_relate_target_owned_0_0: String,
    pub ngmr_relate_target_owned_0_0: String,
    pub tubi_relate_0: String,
    pub ranges: ModelRecordIdRangeEvidence,
    pub cleanup: ModelRecordIdCleanupEvidence,
}

#[derive(Debug, Serialize)]
pub struct ModelRecordIdRangeEvidence {
    pub inst_relate_ref0: String,
    pub inst_relate_refno: String,
    pub geo_relate_refno: String,
    pub neg_relate_target_refno: String,
    pub ngmr_relate_target_refno: String,
    pub tubi_relate_refno: String,
}

#[derive(Debug, Serialize)]
pub struct ModelRecordIdCleanupEvidence {
    pub exact_delete_ids: Vec<String>,
    pub range_delete_ranges: Vec<String>,
}

pub fn build_model_record_id_evidence(refno: RefnoEnum) -> ModelRecordIdEvidence {
    let parts = refno_id_parts(refno);
    ModelRecordIdEvidence {
        input_refno: refno.to_string(),
        parts,
        inst_relate: model_refno_id("inst_relate", refno),
        inst_relate_aabb: model_refno_id("inst_relate_aabb", refno),
        inst_relate_bool: model_refno_id("inst_relate_bool", refno),
        inst_relate_cata_bool: model_refno_id("inst_relate_cata_bool", refno),
        refno_relations: model_refno_id("refno_relations", refno),
        geo_relate_0: geo_relate_id(refno, 0),
        neg_relate_target_owned_0_0: neg_relate_id(refno, refno, 0, 0),
        ngmr_relate_target_owned_0_0: ngmr_relate_id(refno, refno, 0, 0),
        tubi_relate_0: tubi_relate_id(refno, 0),
        ranges: ModelRecordIdRangeEvidence {
            inst_relate_ref0: model_ref0_range("inst_relate", parts.ref0),
            inst_relate_refno: model_refno_range("inst_relate", refno),
            geo_relate_refno: model_refno_range("geo_relate", refno),
            neg_relate_target_refno: model_refno_range("neg_relate", refno),
            ngmr_relate_target_refno: model_refno_range("ngmr_relate", refno),
            tubi_relate_refno: model_refno_range("tubi_relate", refno),
        },
        cleanup: ModelRecordIdCleanupEvidence {
            exact_delete_ids: [
                "inst_relate",
                "inst_relate_aabb",
                "inst_relate_bool",
                "inst_relate_cata_bool",
                "refno_relations",
            ]
            .iter()
            .map(|table| model_refno_id(table, refno))
            .collect(),
            range_delete_ranges: ["geo_relate", "neg_relate", "ngmr_relate", "tubi_relate"]
                .iter()
                .map(|table| model_refno_range(table, refno))
                .collect(),
        },
    }
}
