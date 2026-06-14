use std::fmt::Display;

use aios_core::RefnoEnum;

/// Decomposed model record id prefix: `[ref0, ref1, sesno]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ModelRefnoIdParts {
    pub ref0: u32,
    pub ref1: u32,
    pub sesno: u32,
}

/// Return the array-id prefix used by model artifact tables.
///
/// `RefnoEnum::Refno` is the current/latest form and is encoded with
/// `sesno = 0`; `RefnoEnum::SesRef` keeps its historical session number.
#[inline]
pub fn refno_id_parts(refno: RefnoEnum) -> ModelRefnoIdParts {
    let raw = refno.refno();
    ModelRefnoIdParts {
        ref0: raw.get_0(),
        ref1: raw.get_1(),
        sesno: refno.sesno().unwrap_or(0),
    }
}

#[inline]
fn record_id(table: &str, key: impl Display) -> String {
    format!("{table}:[{key}]")
}

/// Build a one-to-one model artifact record id such as
/// `inst_relate:[24381,145569,0]`.
#[inline]
pub fn model_refno_id(table: &str, refno: RefnoEnum) -> String {
    let parts = refno_id_parts(refno);
    model_refno_id_with_sesno(table, refno, parts.sesno)
}

/// Build a one-to-one model artifact record id for an explicit session.
#[inline]
pub fn model_refno_id_with_sesno(table: &str, base_refno: RefnoEnum, sesno: u32) -> String {
    let parts = refno_id_parts(base_refno);
    record_id(table, format!("{},{},{}", parts.ref0, parts.ref1, sesno))
}

/// Build a per-geometry relation id under a carrier refno/session prefix.
#[inline]
pub fn geo_relate_id(carrier: RefnoEnum, geo_index: impl Display) -> String {
    model_refno_child_id("geo_relate", carrier, geo_index)
}

/// Build a one-to-many model artifact record id such as
/// `geo_relate:[24381,145569,0,2]`.
#[inline]
pub fn model_refno_child_id(table: &str, refno: RefnoEnum, child: impl Display) -> String {
    let parts = refno_id_parts(refno);
    record_id(
        table,
        format!("{},{},{},{}", parts.ref0, parts.ref1, parts.sesno, child),
    )
}

/// Build a TUBI relation id under the owning BRAN refno/session prefix.
#[inline]
pub fn tubi_relate_id(branch_refno: RefnoEnum, tubi_index: impl Display) -> String {
    model_refno_child_id("tubi_relate", branch_refno, tubi_index)
}

/// Build a negative relation id that is range-cleanable by target first.
///
/// Explicit refno/session regeneration is target-driven, so target ownership
/// must be the leading range prefix. Carrier identity is still preserved after
/// the target prefix.
#[inline]
pub fn neg_relate_id(target: RefnoEnum, carrier: RefnoEnum, geo_index: impl Display) -> String {
    let target = refno_id_parts(target);
    let carrier = refno_id_parts(carrier);
    record_id(
        "neg_relate",
        format!(
            "{},{},{},{},{},{},{}",
            target.ref0,
            target.ref1,
            target.sesno,
            carrier.ref0,
            carrier.ref1,
            carrier.sesno,
            geo_index
        ),
    )
}

/// Build an NGMR relation id that is range-cleanable by target first.
#[inline]
pub fn ngmr_relate_id(
    target: RefnoEnum,
    carrier: RefnoEnum,
    ngmr: RefnoEnum,
    geo_index: impl Display,
) -> String {
    let target = refno_id_parts(target);
    let carrier = refno_id_parts(carrier);
    let ngmr = refno_id_parts(ngmr);
    record_id(
        "ngmr_relate",
        format!(
            "{},{},{},{},{},{},{},{},{},{}",
            target.ref0,
            target.ref1,
            target.sesno,
            carrier.ref0,
            carrier.ref1,
            carrier.sesno,
            ngmr.ref0,
            ngmr.ref1,
            ngmr.sesno,
            geo_index
        ),
    )
}

/// Build a SurrealDB range covering all model records for one `ref0`.
#[inline]
pub fn model_ref0_range(table: &str, ref0: u32) -> String {
    format!("{table}:[{ref0}, NONE]..=[{ref0}, ..]")
}

/// Build a SurrealDB range covering all current-session model records for one
/// refno. Historical records use `model_refno_sesno_range`.
#[inline]
pub fn model_refno_range(table: &str, refno: RefnoEnum) -> String {
    let parts = refno_id_parts(refno);
    model_refno_sesno_range(table, refno, parts.sesno)
}

/// Build a SurrealDB range covering all model records for one refno/session.
#[inline]
pub fn model_refno_sesno_range(table: &str, refno: RefnoEnum, sesno: u32) -> String {
    let parts = refno_id_parts(refno);
    format!(
        "{table}:[{},{},{}, NONE]..=[{},{},{}, ..]",
        parts.ref0, parts.ref1, sesno, parts.ref0, parts.ref1, sesno
    )
}
