//! PCF 元件分发入口与跨元件共用的字段原语。
//!
//! [`crate::pcf::bran`] 负责走完一条管路的骨架，本模块负责**逐个节点**把它
//! 翻成 PCF 字段：`gen_node_basic_data` 按类型分发到各元件子模块，其余
//! `create_*` / `gen_*` 函数是各元件反复用到的公共取数与格式化原语。
//!
//! ## 两类函数的分工
//!
//! - **`create_xxx_data`**：带查询的「取数 + 格式化」。要访问数据库或缓存，
//!   多为 `async`。取不到就返回空 `Vec`。
//! - **`gen_xxx_str`**：纯格式化，把已有的值拼成一行 PCF 文本。同步、无副作用。
//!
//! ## 贯穿全模块的错误约定
//!
//! **任何一步取不到数据都返回空 `Vec`，绝不返回 `Result`。**
//! 理由：PCF 是尽力而为的交付物，缺一行字段远比整篇导出失败可接受。
//! 副作用是错误会被静默吞掉——排查缺字段时要顺着调用链一跳跳看，
//! 而不是指望日志里有报错。
//!
//! ## 反复出现的 `SPRE → DETR → …` 两跳
//!
//! PDMS 的选型信息是三层结构：设计实例的 `SPRE` 指向**规格选型条目**，
//! 选型条目的 `DETR` 指向**元件库详图**，真正的 SKEY、材料描述、焊接
//! 形式都写在详图上。所以 [`create_s_key_data`]、[`get_s_key_value`]、
//! [`create_weld_spec_data`] 三个函数结构几乎一样，都是这两跳。
//! 它们没有合并，是因为各自要取的属性和返回形态不同。

use aios_core::{AttrMap, AttrVal};
use aios_core::pdms_types::*;
use aios_core::prim_geo::tubing::{TubiEdge, TubiSize};
use itertools::Itertools;
use lazy_static::lazy_static;
use sqlx::{MySql, Pool};
use dashmap::{DashMap, DashSet};
use glam::Vec3;
use log::kv::ToValue;
use crate::api::attr::{query_explicit_attr, query_attr, query_implicit_attr};
use crate::api::element::query_name;
use aios_core::get_db_option;
use aios_core::db_pool::get_project_pool;
use crate::pcf::atta::gen_atta_data;
use crate::pcf::bend::gen_bend_data;
use crate::pcf::bran::{gen_center_point_data, gen_co_ords_data, gen_cords_point_data, gen_endpoint_data, gen_refno_data, gen_type_name_data};
use crate::pcf::cap::gen_cap_data;
use crate::pcf::coup::gen_coup_data;
use crate::pcf::elbo::gen_elbo_data;
use crate::pcf::flan::gen_flan_data;
use crate::pcf::gask::gen_gask_data;
use crate::pcf::inst::gen_inst_data;
use crate::pcf::olet::gen_olet_data;
use crate::pcf::redu::gen_redu_data;
use crate::pcf::tee::gen_tee_data;
use crate::pcf::valv::gen_valv_data;

lazy_static! {
    /// attr_map 中不需要转为 bytes的属性
    ///
    /// 实际用途是**白名单**：只有列在这里的 PDMS 类型才会被翻成 PCF 记录，
    /// 其余节点（如纯结构性的分组元素）一律跳过。往 PCF 里加新元件类型时，
    /// 除了写子模块，还必须把类型名加进这个集合，否则永远不会被分发到。
    ///
    /// 用 `DashSet` 而非 `HashSet`：这张表被多条管路并发查询，免去外层加锁。
    pub static ref PCF_NODES: DashSet<String> = {
        let mut set = DashSet::new();
        set.insert("ATTA".to_string());
        set.insert("ELBO".to_string());
        set.insert("FLAN".to_string());
        set.insert("GASK".to_string());
        set.insert("INST".to_string());
        set.insert("OLET".to_string());
        set.insert("REDU".to_string());
        set.insert("TEE".to_string());
        set.insert("VALV".to_string());
        set.insert("BEND".to_string());
        set.insert("CAP".to_string());
        set.insert("COUP".to_string());
        set
    };
}

/// 生成每个节点都存在的 pcf 数据 ，返回值为是否是cap 是cap就代表bran结束，后面的节点就不用执行了
///
/// 两段式：先输出各元件共有的「类型名 + 前后端点」，再按类型分发给
/// 对应子模块补该元件特有的字段。
///
/// **ATTA 和 TEE 被排除在共有段之外**：附件不占管长、也就没有前后端点；
/// 三通的 set-on 变体连类型名都要改写。这两类由各自子模块自行输出。
///
/// 返回 `true` 的唯一情形是碰到 CAP（管帽）——管帽封端，调用方据此
/// 停止遍历这条 BRAN。返回 `false` 既可能是「正常继续」，也可能是
/// 「这个节点不产出记录」，调用方不需要区分。
pub async fn gen_node_basic_data(refno: RefU64, mut data: &mut Vec<u8>, mut materials: &mut Vec<(RefU64, String)>,
                                 bran_attr: &AttrMap, start_edge: &TubiEdge, thickness_map: &DashMap<String, DashMap<String, String>>,
                                 end_edge: &TubiEdge, aios_mgr: &AiosDBManager, pool: &Pool<MySql>) -> bool {
    // 传 None 表示取全部属性：这里还不知道是什么类型，也就无法预判要哪几个，
    // 只能整份拉回来再按类型分发。
    let attr = query_attr(refno, &aios_mgr, None).await;
    // 查不到属性按「跳过该节点」处理，而不是中断整条管路。
    if attr.is_err() { return false; }
    let attr = attr.unwrap();
    let type_name = attr.get_type_str();
    // 不在白名单里的类型直接放行，不产出任何记录。
    if !PCF_NODES.contains(type_name) { return false; }
    if !["ATTA", "TEE"].contains(&type_name) { // TEE 需要做特殊处理
        data.append(&mut gen_type_name_data(type_name));

        // 入口端点取**前一段的终点**——前一段管子结束的地方就是本元件的入口。
        if let TubiSize::BoreSize(bore) = start_edge.tubi_size {
            let start_point = start_edge.end_pt;
            data.append(&mut gen_endpoint_data(start_point, bore));
        }

        // 出口端点取**后一段的起点**，同理。
        if let TubiSize::BoreSize(bore) = end_edge.tubi_size {
            let end_point = end_edge.start_pt;
            data.append(&mut gen_endpoint_data(end_point, bore));
        }

    }
    // 分发到各元件子模块。注意 TEE 的参数最多——它要算支管点和支路壁厚。
    match type_name {
        "ATTA" => { data.append(&mut gen_atta_data(aios_mgr, &attr, pool, materials).await); }
        "BEND" => { data.append(&mut gen_bend_data(aios_mgr, &attr, pool, materials).await); }
        "CAP" => {
            data.append(&mut gen_cap_data(aios_mgr, &attr, pool).await);
            // cap 是管套 代表一个bran结束
            return true;
        }
        "ELBO" => { data.append(&mut gen_elbo_data(aios_mgr, &attr, pool, materials).await); }
        "GASK" => { data.append(&mut gen_gask_data(aios_mgr, &attr, pool, materials).await); }
        "FLAN" => { data.append(&mut gen_flan_data(aios_mgr, &attr, pool, materials).await); }
        "VALV" => { data.append(&mut gen_valv_data(aios_mgr, &attr, pool, materials).await); }
        "TEE" => { data.append(&mut gen_tee_data(aios_mgr, &attr, bran_attr, pool, materials, start_edge, end_edge, thickness_map).await); }
        "REDU" => { data.append(&mut gen_redu_data(aios_mgr, &attr, pool, materials).await); }
        "INST" => { data.append(&mut gen_inst_data(aios_mgr, &attr, pool, materials).await); }
        "OLET" => { data.append(&mut gen_olet_data(aios_mgr, &attr, pool, materials).await); }
        "COUP" => { data.append(&mut gen_coup_data(aios_mgr, &attr, pool, materials).await); }
        // 白名单已经挡过一遍，这里理论上到不了；留空分支是为了 match 穷尽。
        _ => {}
    }
    false
}

/// 生成 center_point 数据
///
/// 取的是**世界变换**后的平移量，不是元件局部坐标——PCF 里所有坐标
/// 都必须在同一个世界系下才拼得起来。
pub async fn create_center_point_data(refno: RefU64, aios_mgr: &AiosDBManager) -> Vec<u8> {
    let center_point = aios_mgr.get_world_transform(refno).await;
    if let Ok(Some(center_point)) = center_point {
        return gen_center_point_data(center_point.translation);
    }
    vec![]
}

/// 生成 cords_point 数据
///
/// 取数方式与 [`create_center_point_data`] 完全相同，只是输出的字段名
/// 不一样（CO-ORDS 而非 CENTRE-POINT）。ATTA 这类附件用它。
pub async fn create_cords_point_data(refno: RefU64, aios_mgr: &AiosDBManager) -> Vec<u8> {
    let center_point = aios_mgr.get_world_transform(refno).await;
    if let Ok(Some(center_point)) = center_point {
        return gen_cords_point_data(center_point.translation);
    }
    vec![]
}

/// 生成 SKEY 数据
///
/// SKEY 是元件的规格代号（如 `ELBW`、`TESO`），下游据此选轴测图上的符号。
/// 它写在元件库详图上，所以要走 `SPRE → DETR → SKEY` 两跳。
///
/// 与 [`get_s_key_value`] 的差别：本函数每跳都按 refno 重新解析所属项目的
/// 连接池（规格库常常不在当前项目库里），因此更慢但更可靠；后者复用
/// 传入的 `pool`，只适用于同库场景。
pub async fn create_s_key_data(attr: &AttrMap, aios_mgr: &AiosDBManager) -> Vec<u8> {
    let spre_refno = attr.get_refu64("SPRE");
    if spre_refno.is_none() { return vec![]; }
    let spre_refno = spre_refno.unwrap();
    let spre_cache = aios_mgr.get_refno_basic(spre_refno);
    if spre_cache.is_none() { return vec![]; }
    let spre_cache = spre_cache.unwrap();
    // 规格库可能属于另一个项目，必须按 refno 找对应连接池。
    let spre_pool = aios_mgr.get_project_pool_by_refno(spre_refno).await;
    if spre_pool.is_none() { return vec![]; }
    let (project, spre_pool) = spre_pool.unwrap();
    // 第 1 跳：选型条目 → 详图引用
    let spre_attr = query_implicit_attr(spre_refno, spre_cache.value(), &spre_pool, Some(vec!["DETR"])).await;
    if spre_attr.is_err() { return vec![]; }
    let spre_attr = spre_attr.unwrap();
    let detr_refno = spre_attr.get_refu64("DETR");
    if let Some(detr_refno) = detr_refno {
        // 详图又可能在第三个库里，再解析一次连接池。
        let detr_pool = aios_mgr.get_project_pool_by_refno(detr_refno).await;
        if detr_pool.is_none() { return vec![]; }
        let (project, detr_pool) = detr_pool.unwrap();
        if let Some(cache) = aios_mgr.get_refno_basic(detr_refno) {
            // 第 2 跳：详图 → SKEY
            let detr_att = query_implicit_attr(detr_refno, cache.value(), &detr_pool, Some(vec!["SKEY"])).await;
            if let Ok(detr_att) = detr_att {
                let s_key = detr_att.get_str("SKEY");
                if let Some(s_key) = s_key {
                    return gen_s_key_data_str(s_key);
                }
            }
        }
    }
    vec![]
}

/// 取 SKEY 的**字符串值**而不是格式化好的字段行。
///
/// 三通需要先拿到 SKEY 做分支判断（是否 `TESO`），再决定怎么输出，
/// 所以不能直接用 [`create_s_key_data`]。
///
/// 注意本函数全程复用传入的 `pool`，不像 `create_s_key_data` 那样逐跳
/// 重解析——调用方要确保规格库与详图和当前元素同库。
pub async fn get_s_key_value(attr: &AttrMap, aios_mgr: &AiosDBManager, pool: &Pool<MySql>) -> Option<String> {
    let spre_refno = attr.get_refu64("SPRE");
    if spre_refno.is_none() { return None; }
    let spre_refno = spre_refno.unwrap();
    let spre_cache = aios_mgr.get_refno_basic(spre_refno);
    if spre_cache.is_none() { return None; }
    let spre_cache = spre_cache.unwrap();
    let spre_attr = query_implicit_attr(spre_refno, spre_cache.value(), pool, Some(vec!["DETR"])).await;
    if spre_attr.is_err() { return None; }
    let spre_attr = spre_attr.unwrap();
    let detr_refno = spre_attr.get_refu64("DETR");
    if let Some(detr_refno) = detr_refno {
        if let Some(cache) = aios_mgr.get_refno_basic(detr_refno) {
            let detr_att = query_implicit_attr(detr_refno, cache.value(), pool, Some(vec!["SKEY"])).await;
            if let Ok(detr_att) = detr_att {
                let s_key = detr_att.get_str("SKEY");
                if let Some(s_key) = s_key {
                    return Some(s_key.to_string());
                }
            }
        }
    }
    None
}

/// 生成 ANGL 数据
///
/// 只接受 `DoubleType`：角度必须是数值。属性缺失或类型不符时不输出这一行，
/// 直通阀、直管件本来就没有角度。
pub fn create_angl_data(attr: &AttrMap) -> Vec<u8> {
    let angle = attr.get_val("ANGL");
    if let Some(AttrVal::DoubleType(angl)) = angle {
        return gen_angl_data_str(*angl);
    }
    vec![]
}

/// 输出 ID-REF-NO 行，把 PCF 记录连回源库元素。几乎每个元件都要带上它。
///
/// 这是 PCF 与 PDMS/E3D 之间**唯一的对账凭据**：下游发现某个元件有问题时，
/// 靠这一行才能回到源库定位到具体是哪个设计元素。缺了它这条记录就成了孤儿，
/// 所以各元件在拿不到 refno 时宁可整条不输出。
pub fn create_refno_data(attr: &AttrMap) -> Vec<u8> {
    if let Some(refno) = attr.get_refno() {
        return gen_refno_data(refno);
    }
    vec![]
}

/// 输出管线设计温度行。值由调用方从 PIPE 上取好传入。
pub fn create_temperature_data(temp: f64) -> Vec<u8> {
    gen_temperature_data_str(temp)
}

/// 输出 SUPPORT-TYPE 行，源属性是 `STEX`。只有 ATTA（支架）用得上。
pub fn create_s_text_data(attr: &AttrMap) -> Vec<u8> {
    if let Some(s_text) = attr.get_str("STEX") {
        return gen_s_text_str(s_text);
    }
    vec![]
}

/// 输出管道等级行（PIPING-SPEC），来源是 `PSPE` 指向元素的**名字**。
pub async fn create_pipeline_spec_data(attr: &AttrMap, pool: &Pool<MySql>) -> Vec<u8> {
    if let Some(pspe_refno) = attr.get_refu64("PSPE") {
        let pspe_name = query_name(pspe_refno, pool).await;
        if let Ok(name) = pspe_name {
            return gen_pipeline_spec_str(&name);
        }
    }
    vec![]
}


/// 输出三通支路的料号行（ITEM-CODE-BRANCH1）。
///
/// 取数逻辑与 [`create_pipeline_spec_data`] 一模一样（都是读 `PSPE` 的名字），
/// 只有输出的字段名不同——鞍式接管按主管等级提料，所以复用同一个来源。
pub async fn create_tee_item_code_bran_data(attr: &AttrMap, pool: &Pool<MySql>) -> Vec<u8> {
    if let Some(pspe_refno) = attr.get_refu64("PSPE") {
        let pspe_name = query_name(pspe_refno, pool).await;
        if let Ok(name) = pspe_name {
            return gen_tee_item_code_bran_data_str(&name);
        }
    }
    vec![]
}

/// 输出管路头端连接引用（ID-HREF）。写的是 refno 而非名字。
pub fn create_pipeline_href_data(attr: &AttrMap) -> Vec<u8> {
    if let Some(href_refno) = attr.get_refu64("HREF") {
        return gen_pipeline_href_str(href_refno);
    }
    vec![]
}

/// 输出管路尾端连接引用（ID-TREF）。与 HREF 对称。
pub fn create_pipeline_tref_data(attr: &AttrMap) -> Vec<u8> {
    if let Some(tref_refno) = attr.get_refu64("TREF") {
        return gen_pipeline_tref_str(tref_refno);
    }
    vec![]
}

/// 输出 CONNECTION-REFERENCE 行：本元件连到哪个元素（按名字）。
/// 管嘴用它标注所属设备。
pub async fn create_cref_name_data(attr: &AttrMap, pool: &Pool<MySql>) -> Vec<u8> {
    let cref = attr.get_refu64("CREF");
    if cref.is_none() { return Vec::new(); }
    let cref = cref.unwrap();
    let cref_name = query_name(cref, pool).await;
    if cref_name.is_err() { return Vec::new(); }
    let cref_name = cref_name.unwrap();
    gen_cref_name_str(&cref_name)
}

/// 输出「这一端没有连接」的记录：END-POSITION-NULL + 该端坐标。
/// 管路的自由端用它收尾，下游才知道管子到此为止而不是数据缺失。
pub fn create_end_position_null_data(position: Vec3) -> Vec<u8> {
    let mut data = Vec::new();
    data.append(&mut gen_end_connection_null_head_data());
    data.append(&mut gen_co_ords_data(position));
    data
}

/// 生成焊接规格行（WELD-SPEC）。
///
/// 源库里没有现成的焊接规格字段，只能从元件库详图的 `RTEX`（人可读描述）
/// 里**抠文本**：
/// - 含 `BW` → 对焊（butt weld）；
/// - 含 `RF` → 突面法兰，进一步找出形如 `RF/xxx` 的那一段原样输出。
///
/// 两者都不匹配时输出空值行——这一行本身要保留，下游按字段位置解析。
///
/// 这是**启发式解析**，依赖元件库描述的书写习惯。换了元件库来源要重新验证。
pub async fn create_weld_spec_data(attr: &AttrMap, aios_mgr: &AiosDBManager) -> Vec<u8> {
    let mut data = Vec::new();
    let spre = attr.get_refu64("SPRE");
    if spre.is_none() { return data; }
    let spre = spre.unwrap();

    let spre_cache = aios_mgr.get_refno_basic(spre);
    if spre_cache.is_none() { return data; }
    let spre_cache = spre_cache.unwrap();
    let spre_pool = aios_mgr.get_project_pool_by_refno(spre).await;
    if spre_pool.is_none() { return data; }
    let (_, spre_pool) = spre_pool.unwrap();
    // 第 1 跳：选型条目 → 详图引用
    let spre_attr = query_implicit_attr(spre, spre_cache.value(), &spre_pool, Some(vec!["DETR"])).await;
    if spre_attr.is_err() { return data; }
    let spre_attr = spre_attr.unwrap();

    let detr = spre_attr.get_refu64("DETR");
    if detr.is_none() { return data; }
    let detr = detr.unwrap();
    let detr_pool = aios_mgr.get_project_pool_by_refno(detr).await;
    if detr_pool.is_none() { return data; }
    let (_, detr_pool) = detr_pool.unwrap();
    // 第 2 跳：详图的显式属性里才有 RTEX
    let detr_attr = query_explicit_attr(detr, &detr_pool).await;
    if detr_attr.is_err() { return data; }
    let detr_attr = detr_attr.unwrap();

    let mut weld_spec = "";
    let r_text = detr_attr.get_str("RTEX").unwrap_or("");
    // BW 优先：只要描述里出现就判定为对焊，不再看后面。
    if r_text.contains("BW") {
        weld_spec = "BW";
    } else if r_text.contains("RF") {
        // RF 情况要连规格一起带出，所以按空格切开找 `RF/` 开头的那一段。
        let r_text_splits = r_text.split(" ").collect::<Vec<_>>();
        for r_text_split in r_text_splits {
            if r_text_split.contains("RF/") {
                weld_spec = r_text_split;
                break;
            }
        }
    }
    data.append(&mut gen_weld_spec_str(weld_spec));
    data
}

/// b_pipe 分为 bran_thickness 和 pipe_thickness 两种 取数据方式一样，最后输出的文字不同
///
/// 从管线名里切出查表 key：名字形如 `2ACAS-A6-806-6-LJ6`，按 `-` 分段后
/// **第 4 段是公称直径 DN**、**第 5 段的首字母是等级**（H/I/L）。
/// 段数不足 5 或表为空时返回空值行——字段保留，值留空。
///
/// 表里存的是 `外径x壁厚` 形式（如 `219.1x6.35`），按 `x` 拆成两个数。
pub fn create_thickness_data(name: &str, thickness_map: &DashMap<String, DashMap<String, String>>, b_pipe: bool) -> Vec<u8> {
    let mut data = Vec::new();
    let mut od = "".to_string();
    let mut thick = "".to_string();
    let name_splits = name.split("-").collect::<Vec<_>>();
    if name_splits.len() >= 5 && !thickness_map.is_empty() {
        let dn = name_splits[3];
        let key = name_splits[4];
        // 只取首字母：第 5 段形如 `LJ6`，等级是开头的 `L`。
        let key = &key[0..1];
        if let Some(dn) = thickness_map.get(dn) {
            let dn = dn.value();
            if let Some(value) = dn.get(key) {
                let value = value.split("x").collect::<Vec<_>>();
                if value.len() > 1 {
                    od = value[0].to_string();
                    thick = value[1].to_string();
                }
            }
        }
    }
    // 同一组数值，主管与支路用不同字段名输出。
    if b_pipe {
        data.append(&mut gen_pipe_od_str(&od));
        data.append(&mut gen_pipe_thick_str(&thick));
    } else {
        data.append(&mut gen_bran_od_str(&od));
        data.append(&mut gen_bran_thick_str(&thick));
    }
    data
}

// ---- 以下均为纯格式化原语 ------------------------------------------------------
// 共同约定：缩进 8 个空格、以 `\r\n`（CRLF）结尾。PCF 读取端按列位解析，
// 空格数和换行符都是格式的一部分，**不要改成 \n、也不要调整缩进**。

/// 生成 SKEY pcf 的数据
pub fn gen_s_key_data_str(s_key: &str) -> Vec<u8> {
    format!("        SKEY  {}\r\n", s_key).into_bytes()
}

/// 角度行。
fn gen_angl_data_str(angl: f64) -> Vec<u8> {
    format!("        ANGLE        {}\r\n", angl).into_bytes()
}

/// 管线设计温度行。
fn gen_temperature_data_str(temp: f64) -> Vec<u8> {
    format!("        PIPELINE-TEMP       {}\r\n", temp).into_bytes()
}

/// 管道等级行。
fn gen_pipeline_spec_str(pspe_name: &str) -> Vec<u8> {
    format!("        PIPING-SPEC    {}\r\n", pspe_name).into_bytes()
}

/// 头端连接引用行。
fn gen_pipeline_href_str(href: RefU64) -> Vec<u8> {
    format!("        ID-HREF    {}\r\n", href.to_string()).into_bytes()
}

/// 尾端连接引用行。
fn gen_pipeline_tref_str(tref: RefU64) -> Vec<u8> {
    format!("        ID-TREF    {}\r\n", tref.to_string()).into_bytes()
}

/// 三通支路料号行。
fn gen_tee_item_code_bran_data_str(pspe_name: &str) -> Vec<u8> {
    format!("        ITEM-CODE-BRANCH1    {}\r\n", pspe_name).into_bytes()
}

/// 支架类型行（源属性 STEX）。
fn gen_s_text_str(s_text: &str) -> Vec<u8> {
    format!("        SUPPORT-TYPE    {}\r\n", s_text).into_bytes()
}

/// 连接引用名行。
fn gen_cref_name_str(cref_name: &str) -> Vec<u8> {
    format!("        CONNECTION-REFERENCE    {}\r\n", cref_name).into_bytes()
}

/// 自由端段头。注意这一条**顶格**输出，不带缩进。
fn gen_end_connection_null_head_data() -> Vec<u8> {
    format!("END-POSITION-NULL\r\n").into_bytes()
}

/// 焊接规格行。
fn gen_weld_spec_str(weld_spec: &str) -> Vec<u8> {
    format!("        WELD-SPEC  {}\r\n", weld_spec).into_bytes()
}

/// 主管外径行。
fn gen_pipe_od_str(od: &str) -> Vec<u8> {
    format!("        PIPE-OD  {}\r\n", od).into_bytes()
}

/// 主管壁厚行。
fn gen_pipe_thick_str(thick: &str) -> Vec<u8> {
    format!("        PIPE-THICK  {}\r\n", thick).into_bytes()
}

/// 支路外径行。
fn gen_bran_od_str(od: &str) -> Vec<u8> {
    format!("        BRANCH1-OD  {}\r\n", od).into_bytes()
}

/// 支路壁厚行。
fn gen_bran_thick_str(thick: &str) -> Vec<u8> {
    format!("        BRANCH1-THICK  {}\r\n", thick).into_bytes()
}
