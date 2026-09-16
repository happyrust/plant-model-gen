//! BRAN（管路分支）遍历与 PCF 文件组装，外加各元件共用的字段生成原语。
//!
//! 这是整个 `pcf` 模块的**主干**：[`get_bran_name_and_children`] 从一条 BRAN
//! 出发，走完它的全部几何段与元件，拼出一份完整的 PCF 文本。
//!
//! ## 一份 PCF 的结构
//!
//! ```text
//! ISOGEN-FILES / UNITS-*        ← 文件头，声明单位制
//! PIPELINE-REFERENCE <管路名>    ← 管路总体信息（起点、温度、规格、壁厚）
//! END-CONNECTION-PIPELINE       ← 头端连接（HREF）
//!   PIPE / ELBOW / VALVE / …    ← 中间：直管段与元件交替出现
//! END-CONNECTION-PIPELINE       ← 尾端连接（TREF）
//! MATERIALS                     ← 材料表，全篇用到的材料在此汇总
//! ```
//!
//! ## 遍历的两个关键点
//!
//! 1. **直管段是推算出来的，不是查出来的**。`query_bran_info` 返回一串
//!    TubiEdge（几何段），相邻两段之间若距离超过 [`TUBI_TOL`] 就说明中间
//!    有一截实际管子，补一条 PIPE 记录。
//! 2. **ATTA 不占长度**。附件（支架等）挂在管上但不打断管子，所以统计
//!    直管长度时要连续跳过 ATTA，把它前后的管段合成一段——否则一根管子
//!    会被支架切成许多短段，下游按段下料就全错了。
//!
//! ## 材料表为什么要边走边攒
//!
//! `materials` 这个 `Vec` 一路作为出参往下传，各元件把自己用到的材料
//! 登记进来，最后由 [`gen_material_data`] 统一输出。这样材料表天然去重，
//! 也不必为了凑材料再把整条管路走第二遍。

use std::io::Write;
use std::sync::Arc;
use aios_core::{AttrMap, AttrVal};
use aios_core::pdms_types::*;
use aios_core::prim_geo::tubing::TubiEdge;


use dashmap::DashMap;
use glam::Vec3;
use parse_pdms_db::parse_explict_tools::times_keep_f32_two_decimal_place;
use sqlx::{MySql, Pool};
use crate::api::attr::{query_attr, query_explicit_attr, query_implicit_attr};
use crate::api::element::{query_children, query_children_eles, query_name};
use crate::aql_api::tubi::query_bran_info;
use crate::data_interface::db_model::TUBI_TOL;
use crate::data_interface::interface::PdmsDataInterface;
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::elbo::gen_elbo_data;
use crate::pcf::excel_api::get_pipe_thickness_table;
use crate::pcf::flan::gen_flan_data;
use crate::pcf::gask::gen_gask_data;
use crate::pcf::nozz::gen_nozz_data;
use crate::pcf::pcf_api::{create_end_position_null_data, create_pipeline_href_data, create_pipeline_spec_data, create_pipeline_tref_data, create_refno_data, create_temperature_data, create_thickness_data, gen_node_basic_data};
use crate::pcf::tee::gen_tee_data;
use crate::pcf::tubi::gen_tubi_data;
use crate::pcf::valv::gen_valv_data;


/// PCF 文件头：声明后续数值用什么单位解读。
///
/// 单位制必须与 [`gen_endpoint_data`] 等原语实际输出的量纲一致——
/// 坐标按毫米写、口径按英寸写。改这里之前先确认几何侧的单位没变，
/// 否则下游会按错误量纲读数，且不会有任何报错。
fn gen_pcf_file_head() -> String {
    "ISOGEN-FILES            ISOGEN.FLS
     UNITS-BORE              INCH
     UNITS-CO-ORDS           MM
     UNITS-BOLT-LENGTH       MM
     UNITS-BOLT-DIA          INCH
     UNITS-WEIGHT            KGS\r\n".to_string()
}

/// 把一条 BRAN 导出成完整的 PCF 字节流。
///
/// `refno` 是 BRAN 自身的引用号，`thickness_map` 是预先读好的外径壁厚表
/// （见 [`crate::pcf::excel_api::get_pipe_thickness_table`]，一次读入多路复用）。
///
/// 返回 `Err` 只发生在「连管路基本信息都取不到」这类根本性失败；
/// 单个元件查询失败一律降级成少写几行，不中断整篇。
pub async fn get_bran_name_and_children(refno: RefU64, aios_mgr: &AiosDBManager,
                                        thickness_map: &DashMap<String, DashMap<String, String>>) -> anyhow::Result<Vec<u8>> {
    let mut data = vec![];
    // 全篇共用一个材料收集器，各元件往里追加，最后统一输出 MATERIALS 段。
    let mut materials = vec![];
    data.append(&mut gen_pcf_file_head().into_bytes());
    let pool = aios_mgr.project_map.get(&aios_mgr.db_option.project_name).unwrap();
    let database = aios_mgr.get_arango_db().await?;
    // let bran_attr = query_attr(refno, &aios_mgr, None).await?;
    let bran_attr = aios_core::get_named_attmap(refno).await?;
    let bran_name = bran_attr.get_name_or_default();

    // 先把 pipe_thickness 算好，需要的直接放进去就好了
    // BRAN 的 owner 是 PIPE，管线名与温度都挂在 PIPE 上而不是 BRAN 上。
    let pipe_refno = aios_mgr.get_owner(refno);
    let pipe_name = query_name(pipe_refno, pool.value()).await?;
    let pipe_cache = aios_mgr.get_refno_basic(pipe_refno);
    if pipe_cache.is_none() { return Ok(data); }
    let pipe_cache = pipe_cache.unwrap();
    let pipe_temp = query_implicit_attr(pipe_refno,pipe_cache.value(),pool.value(),Some(vec!["TEMP"])).await.unwrap_or_default();
    // -100000.0 是「温度未填」的哨兵值，下游据此判断要不要显示这一项。
    let pipe_temp = pipe_temp.get_f64("TEMP").unwrap_or(-100000.0);
    let pipe_thickness_data = create_thickness_data(&pipe_name, thickness_map, true);
    // 查找 bran带tubi和元件 的数据
    let bran_infos = query_bran_info(refno, &database).await?;
    // 生成 bran href 的数据
    if let Some(start_position) = bran_infos.first() {
        let start_position = start_position.start_pt;
        data.append(&mut gen_bran_pipeline_reference_data(&bran_attr, start_position, pool.value(), &pipe_thickness_data,pipe_temp).await);
        // HREF = head reference，管路头端接到什么上（设备管嘴或另一条管）。
        let href = bran_attr.get_refu64("HREF");
        data.append(&mut gen_bran_connection_data(href, start_position, aios_mgr, pool.value()).await);
    }
    // 生成 bran 中间节点的数据
    for i in 0..bran_infos.len() {
        // 可能存在 tubi
        // 第 0 段没有前驱，用下标 1 去取「前一段」以免越界；
        // 其余情况下 i 自身就是前一段的位置。
        let tubi_start_index = if i == 0 { 1 } else { i };
        // 如果 tubi_edge 的 att_type 是 ATTA ， 统计tubi的时候就需要跳过下一个tubi
        if bran_infos[tubi_start_index - 1].att_type != "ATTA" {
            let mut tubi_end_index = i;
            let distance = bran_infos[i].start_pt.distance(bran_infos[tubi_end_index].end_pt);
            // 距离小于容差就认为两个元件是紧挨着的，中间没有实际管子。
            if distance >= TUBI_TOL {
                // 跳过 atta ，不记录 atta的长度
                // 连续多个 ATTA 要一路跳到底，把它们两侧的管段并成一段。
                if bran_infos[i].att_type == "ATTA" {
                    while tubi_end_index < bran_infos.len() - 1 {
                        if bran_infos[tubi_end_index].att_type == "ATTA" { tubi_end_index += 1; } else { break; }
                    }
                }
                // 直管没有自己的 refno，借前一个元件的 _from 作为来源标记。
                let from_refno = RefU64::from_str(&bran_infos[i]._from);
                let mut tubi_data = gen_tubi_data(bran_infos[i].start_pt, bran_infos[tubi_end_index].end_pt,
                                                  bran_infos[tubi_end_index].tubi_size, &bran_attr, from_refno,
                                                  &mut materials, &pipe_thickness_data, aios_mgr).await;
                data.append(&mut tubi_data);
            }
        }
        // 生成 bran 元件数据
        // 最后一段没有「下一段」可作为出口端点，跳过——它的终点已由上面的
        // 直管段或下面的 TREF 处理掉了。
        if i == bran_infos.len() - 1 { continue; }
        let refno = convert_refno_from_edge_str(&bran_infos[i]._to);
        if refno.is_err() { continue; }
        let refno = refno.unwrap();
        // 返回 true 表示碰到了 CAP（管帽），这条 BRAN 到此结束，后面不用再走。
        if gen_node_basic_data(refno, &mut data, &mut materials, &bran_attr,
                               &bran_infos[i], thickness_map, &bran_infos[i + 1],
                               aios_mgr, pool.value()).await {
            break;
        }
    }
    // 生成 bran tref 的数据
    if let Some(leave_position) = bran_infos.last() {
        let leave_position = leave_position.end_pt;
        // TREF = tail reference，管路尾端的连接目标。
        let tref = bran_attr.get_refu64("TREF");
        data.append(&mut gen_bran_connection_data(tref, leave_position, aios_mgr, pool.value()).await);
    }
    // 生成 material 数据
    data.append(&mut gen_material_data(materials, aios_mgr, pool.value()).await);
    Ok(data)
}

/// 输出管路总体信息段（PIPELINE-REFERENCE 及其下属字段）。
///
/// 这一段描述的是整条管路的公共属性：起点坐标、设计温度、管道等级、
/// 头尾连接引用、主管壁厚。各元件记录都挂在它下面。
pub async fn gen_bran_pipeline_reference_data(attr: &AttrMap, start_position: Vec3, pool: &Pool<MySql>,
                                              pipe_thickness_data: &Vec<u8>,pipe_temp:f64) -> Vec<u8> {
    let mut data = vec![];
    let name = attr.get_name_or_default();
    data.append(&mut gen_pipeline_reference_data_str_head(name.as_str()));
    data.append(&mut gen_start_co_ords_data(start_position));
    data.append(&mut create_temperature_data(pipe_temp));
    data.append(&mut create_pipeline_spec_data(attr, pool).await);
    data.append(&mut create_refno_data(attr));
    data.append(&mut create_pipeline_href_data(attr));
    data.append(&mut create_pipeline_tref_data(attr));
    // 壁厚数据在调用方已算好，这里克隆一份拼进去（头尾两处都要用）。
    data.append(&mut pipe_thickness_data.clone());
    data
}

/// 输出管路一端的连接信息（头端用 HREF、尾端用 TREF）。
///
/// 三种情况：
/// - **接管嘴**：连接目标是 NOZZ，走 [`gen_nozz_data`] 输出完整管嘴记录；
/// - **接别的管路**：输出 END-CONNECTION-PIPELINE + 坐标 + 对端管路名；
/// - **没接东西**（`None` 或 refno 为 0）：输出 END-POSITION-NULL，
///   表示这一端是自由端。
pub async fn gen_bran_connection_data(refno: Option<RefU64>, leave_position: Vec3, aios_mgr: &AiosDBManager, pool: &Pool<MySql>) -> Vec<u8> {
    let mut data = vec![];
    if let Some(refno) = refno {
        // RefU64(0) 是源库里「未连接」的表示法，等同于 None。
        if refno == RefU64(0) { return create_end_position_null_data(leave_position); }
        let refno_table_name = aios_mgr.get_refno_basic(refno);
        if refno_table_name.is_none() { return data; }
        let refno_table_name = refno_table_name.unwrap();
        // 如果连接的是 nozz ， 则是另一种取数据方式
        if refno_table_name.table.to_uppercase() == "NOZZ" {
            let nozz_attr = query_attr(refno, aios_mgr, Some(vec!["CREF"])).await;
            if let Ok(nozz_attr) = nozz_attr {
                data.append(&mut gen_nozz_data(aios_mgr, &nozz_attr, pool).await);
            }
        } else {
            data.append(&mut gen_end_connection_pipeline_head_data());
            data.append(&mut gen_co_ords_data(leave_position));
            // 取不到名字时写空串：这一行的存在本身就表达了「有连接」。
            let name = query_name(refno, pool).await.unwrap_or("".to_string());
            data.append(&mut gen_pipeline_reference_data_str(&name));
        }
    } else {
        data.append(&mut create_end_position_null_data(leave_position));
    }
    data
}

/// 输出 MATERIALS 段：把全篇攒下的材料逐条展开成「料号 + 描述」。
///
/// 描述要顺着 `SPRE → DETR → RTEX` 两跳去元件库详图里取。任何一跳失败
/// 就 `continue` 跳过该条——材料表少一行描述可以接受，中断整篇不行。
async fn gen_material_data(materials: Vec<(RefU64, String)>, aios_mgr: &AiosDBManager, pool: &Pool<MySql>) -> Vec<u8> {
    let mut data = Vec::new();
    data.push(gen_material_head_data());
    for (spre_refno, spre_name) in materials {
        data.push(gen_item_code_data_name(&spre_name));
        let spre_cache = aios_mgr.get_refno_basic(spre_refno);
        if spre_cache.is_err() { continue; }
        let spre_cache = spre_cache.unwrap();
        // 第 1 跳：规格选型 → 详图引用（DETR）
        let spre_attr = query_implicit_attr(spre_refno, spre_cache.value(), pool, Some(vec!["DETR"])).await;
        if spre_attr.is_err() { continue; }
        let spre_attr = spre_attr.unwrap();
        let detr_refno = spre_attr.get_refu64("DETR");
        if detr_refno.is_err() { continue; }
        let detr_refno = detr_refno.unwrap();
        // dbg!(&detr_refno);
        let detr_cache = aios_mgr.get_refno_basic(detr_refno);
        if detr_cache.is_err() { continue; }
        let detr_cache = detr_cache.unwrap();
        // 第 2 跳：详图的显式属性 RTEX 就是人可读的材料描述
        let detr_attr = query_explicit_attr(detr_refno, pool).await;
        if detr_attr.is_err() { continue; }
        let detr_attr = detr_attr.unwrap();
        let r_text = detr_attr.get_str("RTEX");
        if let Some(r_text) = r_text {
            data.push(gen_material_item_code_description(r_text));
        }
    }
    // 各条是分开 push 的 Vec<Vec<u8>>，这里摊平成一条连续字节流。
    data.into_iter().flatten().collect()
}

/// PDMS 类型名 → PCF 关键字的映射表。
///
/// 两边词汇不一致：PDMS 用 4 字母缩写，PCF 用英文全称。没列出来的类型
/// （ELBO 之外的 BEND、TEE、CAP 等）两边写法一致，原样透传即可。
fn match_type_name(input: &str) -> &str {
    match input {
        "ATTA" => { "SUPPORT" }
        "GASK" => { "GASKET" }
        "FLAN" => { "FLANGE" }
        "ELBO" => { "ELBOW" }
        "VALV" => { "VALVE" }
        // 异径管默认按同心处理；偏心异径管目前未单独区分。
        "REDU" => { "REDUCER-CONCENTRIC" }
        "INST" => { "INSTRUMENT" }
        _ => { input }
    }
}

/// 生成 pipeline_reference 数据
fn gen_bran_reference_data(bran_name: &str) -> Vec<u8> {
    format!("PIPELINE-REFERENCE      {}\r\n", bran_name).into_bytes()
}

/// 生成 pcf type 名
pub fn gen_type_name_data(type_name: &str) -> Vec<u8> {
    let type_name = match_type_name(type_name);
    format!("{}\r\n", type_name).into_bytes()
}

/// 生成 end_point 得 pcf 数据
///
/// 端点行末尾多带一个口径（bore），这是 PCF 判断变径的依据。
pub fn gen_endpoint_data(point: Vec3, bore: f32) -> Vec<u8> {
    format!("        END-POINT    {}  {}  {} {}\r\n", point.x, point.y, point.z, bore).into_bytes()
}

/// 生成 center_point 得 pcf 数据
pub fn gen_center_point_data(center_point: Vec3) -> Vec<u8> {
    format!("        CENTRE-POINT    {}  {}  {}\r\n", center_point.x, center_point.y, center_point.z).into_bytes()
}

/// 输出 CO-ORDS 行。与 CENTRE-POINT 的区别：这是安装标注坐标，
/// 用于 ATTA 这类附件；元件几何中心用 CENTRE-POINT。
pub fn gen_cords_point_data(cords_point: Vec3) -> Vec<u8> {
    format!("        CO-ORDS    {}  {}  {}\r\n", cords_point.x, cords_point.y, cords_point.z).into_bytes()
}

/// 由 SPRE 属性值生成 ITEM-CODE 行，**同时把该材料登记进 `materials`**。
///
/// 这是材料表的唯一入口：各元件都调它，因此只要元件写了料号，
/// 材料表末尾就一定有对应条目。登记前先 `contains` 查重，避免同一
/// 规格在一条管路上出现多次时重复列入。
pub async fn gen_item_code_data_attr_val(spre_refno: Option<&AttrVal>, aios_mgr: &AiosDBManager, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    if let Some(spre) = spre_refno {
        let spre_refno = spre.refno_value();
        if let Some(spre_refno) = spre_refno {
            // 规格库可能不在当前项目库里，要按 refno 找对应的连接池。
            let spre_pool = aios_mgr.get_project_pool_by_refno(spre_refno).await;
            if spre_pool.is_none() { return vec![]; }
            let (_, spre_pool) = spre_pool.unwrap();
            let spre_name = query_name(spre_refno, &spre_pool).await;
            return if let Ok(spre_name) = spre_name {
                if !materials.contains(&(spre_refno, spre_name.clone())) {
                    materials.push((spre_refno, spre_name.clone()));
                }
                format!("        ITEM-CODE    {}\r\n", spre_name).into_bytes()
            } else {
                vec![]
            };
        }
    }
    vec![]
}

/// 由 refno 直接生成 ITEM-CODE 行（不登记材料表）。
/// 用在已知不需要进材料表、只要标一个料号的场合。
pub async fn gen_item_code_data_refno(spre_refno: RefU64, pool: &Pool<MySql>) -> Vec<u8> {
    let spre_name = query_name(spre_refno, pool).await;
    return if let Ok(spre_name) = spre_name {
        format!("        ITEM-CODE    {}\r\n", spre_name).into_bytes()
    } else {
        vec![]
    };
}

/// MATERIALS 段里的料号行。
/// 注意这一条**顶格**输出，与元件内部缩进 8 格的 ITEM-CODE 不是一回事。
pub fn gen_item_code_data_name(spre_name: &str) -> Vec<u8> {
    format!("ITEM-CODE    {}\r\n", spre_name).into_bytes()
}

/// 生成 ID-REF-NO 的数据
pub fn gen_refno_data(refno: RefU64) -> Vec<u8> {
    format!("        ID-REF-NO    {}\r\n", refno.to_string()).into_bytes()
}

/// 直管段的 ID-REF-NO：在来源元件 refno 后缀 `-1`。
///
/// 加后缀是因为直管借用了前一个元件的 refno，不加就会和那个元件的记录
/// 撞号，下游按 refno 建索引时会覆盖掉一条。
pub fn gen_refno_data_pipe(refno: RefU64) -> Vec<u8> {
    format!("        ID-REF-NO    {}-1\r\n", refno.to_string()).into_bytes()
}

/// 从图数据库的 edge 的 _from / _to 数据转换成 RefU64
fn convert_refno_from_edge_str(refno_str: &str) -> Option<RefU64> {
    // ArangoDB 的文档 id 形如 `集合名/键`，refno 在斜杠右边。
    let refno_str = refno_str.split("/").collect::<Vec<_>>();
    if refno_str.len() <= 1 { return None; }
    let refno_str = refno_str[1];
    RefU64::from_str(refno_str)
}


/// END-CONNECTION-PIPELINE 段头：表示这一端接的是另一条管路。
fn gen_end_connection_pipeline_head_data() -> Vec<u8> {
    format!("END-CONNECTION-PIPELINE\r\n").into_bytes()
}

/// 管路起点坐标行，只在 PIPELINE-REFERENCE 段里出现一次。
fn gen_start_co_ords_data(position: Vec3) -> Vec<u8> {
    format!("        START-CO-ORDS  {}  {}  {}\r\n", position.x, position.y, position.z).into_bytes()
}

/// 通用坐标行，用于连接段与 END-POSITION-NULL。
pub fn gen_co_ords_data(position: Vec3) -> Vec<u8> {
    format!("        CO-ORDS  {}  {}  {}\r\n", position.x, position.y, position.z).into_bytes()
}

/// 缩进版管路引用行：出现在连接段内部，指向对端管路。
fn gen_pipeline_reference_data_str(name: &str) -> Vec<u8> {
    format!("        PIPELINE-REFERENCE  {}\r\n", name).into_bytes()
}

/// 顶格版管路引用行：作为整条管路记录的段头。
fn gen_pipeline_reference_data_str_head(name: &str) -> Vec<u8> {
    format!("PIPELINE-REFERENCE  {}\r\n", name).into_bytes()
}

/// MATERIALS 段头，全篇只出现一次，位置固定在文件末尾。
fn gen_material_head_data() -> Vec<u8> {
    format!("MATERIALS\r\n").into_bytes()
}

/// 材料描述行，紧跟在对应的 ITEM-CODE 之后。
fn gen_material_item_code_description(desc: &str) -> Vec<u8> { format!("        DESCRIPTION    {}\r\n", desc).into_bytes() }
