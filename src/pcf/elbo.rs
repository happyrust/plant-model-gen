//! ELBO（弯头）元件的 PCF 记录生成，外加弯曲半径的两种取数方式。
//!
//! 弯头是成品配件，PCF 里要给出中心点、规格代号、转角、材料、焊接规格，
//! 以及**弯曲半径**（BEND-RADIUS）——半径决定了轴测图上的走线和实际占位。
//!
//! ## 半径为什么有两条取数路径
//!
//! - [`create_radius_data`]：直接读设计实例上的 `RADI` 属性。快，但只有
//!   设计侧显式填了才有值。
//! - [`get_catr_para_data_from_spre`]：顺着 `SPRE`（规格选型）→ `CATR`
//!   （元件库目录条目）→ `PARA`（参数数组）三跳去元件库里翻。慢，但这是
//!   成品弯头半径的**权威来源**。
//!
//! 弯头走后者（成品件以元件库为准），弯管（BEND）走前者（现场煨弯，
//! 以设计值为准）。这个分工别搞反。
//!
//! 注意本文件同时输出了 `gen_refno_data(refno)` 和上层已有的 refno 字段，
//! 是弯头记录格式的既有要求，不要当成重复而删掉。

use aios_core::{AttrMap, AttrVal};
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::api::attr::{query_explicit_attr, query_implicit_attr};
use crate::data_interface::interface::PdmsDataInterface;
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::{gen_center_point_data, gen_item_code_data_attr_val, gen_refno_data};
use crate::pcf::pcf_api::{create_angl_data, create_center_point_data, create_s_key_data, create_weld_spec_data};

/// 生成 elbo 特有的 pcf 数据
pub async fn gen_elbo_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    data.append(&mut create_center_point_data(refno, aios_mgr).await);
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    data.append(&mut create_angl_data(attr));
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
    data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
    data.append(&mut gen_refno_data(refno));
    // 半径放最后：这一跳要查元件库，是本函数最慢的一步，
    // 放在末尾可让前面的字段先成型，出问题时也容易定位到是哪一段卡住。
    data.append(&mut get_catr_para_data_from_spre(spre, &aios_mgr, pool).await);
    data
}

/// 从 SPRE 顺着 `SPRE → CATR → PARA` 反查成品弯头的弯曲半径。
///
/// 三跳里任何一跳查不到都返回空 `Vec`（不产出 BEND-RADIUS 行），
/// 而不是报错——元件库数据不全属于常见情况，不该因此中断整篇 PCF。
///
/// `PARA` 是一个 double 数组，半径固定取**下标 1**那一项；
/// 这是元件库对弯头参数的排布约定，改动前先确认元件库定义。
pub async fn get_catr_para_data_from_spre(spre: Option<&AttrVal>, aios_mgr: &AiosDBManager, pool: &Pool<MySql>) -> Vec<u8> {
    // 只处理引用型的 SPRE；其他类型说明选型数据不规范，直接放弃。
    if let Some(AttrVal::RefU64Type(spre_refno)) = spre {
        let spre_refno = *spre_refno;
        // 先拿缓存里的基础信息，隐式属性查询需要它做上下文。
        let cache_basic = aios_mgr.get_refno_basic(spre_refno);
        if let Some(cache_basic) = cache_basic {
            // 第 1 跳：规格选型 → 目录条目（CATR）
            let spre_attr = query_implicit_attr(spre_refno, cache_basic.value(), pool, Some(vec!["CATR"])).await;
            if let Ok(spre_attr) = spre_attr {
                let catr = spre_attr.get_refu64("CATR");
                if let Some(catr) = catr {
                    // 第 2 跳：目录条目的显式属性里带着参数数组
                    let catr_attr = query_explicit_attr(catr, pool).await;
                    if let Ok(catr_attr) = catr_attr {
                        let para = catr_attr.get_val("PARA");
                        // 第 3 跳：从参数数组里取半径
                        if let Some(AttrVal::DoubleArrayType(paras)) = para {
                            if let Some(para) = paras.get(1) {
                                return gen_radius_data(*para);
                            }
                        }
                    }
                }
            }
        }
    }
    vec![]
}


/// 生成 radius 数据 ( 直接从desi的attr里面取的radius )
pub fn create_radius_data(attr: &AttrMap) -> Vec<u8> {
    if let Some(radius) = attr.get_f64("RADI") {
        return gen_radius_data(radius);
    }
    vec![]
}

/// 输出 BEND-RADIUS 行。缩进 8 空格 + `\r\n` 是 PCF 硬性格式，别改。
fn gen_radius_data(radius: f64) -> Vec<u8> { format!("        BEND-RADIUS  {}\r\n", radius).into_bytes() }
