//! E3D 属性变化到模型生成影响的收敛判定。
//!
//! `pdms-io` 已经给出 Added/Deleted/Modified 的属性级差异。这里只把生成器
//! 实际消费的拓扑、定位、型材/目录引用及几何参数视为模型变化；名称、描述、
//! 用户自定义属性等业务元数据不会仅因 sesno 前进而触发几何生成。

/// 最小交付单元允许作为独立版本根的 noun。
pub const DELIVERY_UNIT_ROOT_NOUNS: &[&str] = &["BRAN", "HANG", "EQUI", "WALL", "FLOOR"];

pub fn is_delivery_unit_root_noun(noun: &str) -> bool {
    DELIVERY_UNIT_ROOT_NOUNS.contains(&noun.trim().to_ascii_uppercase().as_str())
}

/// 判断单个 E3D/PDMS 属性是否会改变生成器的模型输入。
///
/// 名称统一按大写比较。该集合来自当前生成链路读取的定位、目录引用、布尔关系、
/// 管路连接和 primitive 参数；未知属性默认不触发生成，避免“任意属性修改即重建”。
pub fn attribute_affects_model(raw_name: &str) -> bool {
    let name = raw_name
        .trim()
        .trim_start_matches("att.")
        .trim_start_matches("ATT.")
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();

    matches!(
        name.as_str(),
        // 层级/类型与世界定位。
        "OWNER" | "CHILDREN" | "NOUN" | "TYPE" | "POS" | "ORI" |
        // catalogue / specification / design-template 依赖。
        "CATR" | "CREF" | "SPRE" | "SPCO" | "SCOM" | "SCREF" |
        "DESP" | "DDSE" | "DDAT" | "DKEY" | "DDPR" | "GMREF" | "GMRE" |
        "GSTR" | "GTYP" | "DPRO" | "DTRE" | "ISPE" | "TMPL" |
        "DDANGLE" | "DDHEIGHT" | "DDRADIUS" | "IPARAM" |
        // 管路、连接与方向依赖。
        "HREF" | "TREF" | "LSTU" | "STYP" | "ANGF" | "ANGL" |
        "ABOR" | "LBOR" | "PBOR" | "SBOR" | "BORE" | "CONN" |
        "HBOR" | "TBOR" | "ADIR" | "RDIR" | "LDIR" | "HDIR" | "TDIR" |
        "HPOS" | "TPOS" | "HSTU" | "BRCO" |
        // 通用 primitive 尺寸/形状参数。
        "XLEN" | "YLEN" | "ZLEN" | "LENG" | "HEIG" | "DIAM" |
        "RADI" | "IRAD" | "ORAD" | "FRAD" | "DRAD" | "CRAD" |
        "DTOP" | "DBOT" | "XBOT" | "YBOT" | "XTOP" | "YTOP" |
        "XOFF" | "YOFF" | "ZOFF" | "THIC" | "WIDE" | "DEPT" |
        "SIZE" | "SHEA" | "TAPER" | "ECC" | "DWID" | "DHEI" | "DIMD" |
        "SDIA" | "SDIS" | "SHEI" | "STHI" | "SWID" | "ARRHEI" |
        "ARRI" | "ARRWID" | "LEAHEI" | "LEAWID" | "MAXA" |
        "CENT" | "DCEN" | "UBOT" | "UCEN" | "UTOP" |
        // P-point / profile 参数。
        "PTDI" | "PTCI" | "PAXI" | "PHEI" | "PANG" | "PPOS" |
        "PORI" | "PXDI" | "PYDI" | "PZDI" | "PAAX" | "PBAX" |
        "PBBT" | "PBDI" | "PBDM" | "PBOF" | "PBTP" | "PCAX" |
        "PCBT" | "PCOF" | "PCON" | "PCTP" | "PDIA" | "PDIS" |
        "PLAX" | "PLIN" | "POFF" | "PRAD" | "PTAX" | "PTCA" |
        "PTCD" | "PTCP" | "PTCPOS" | "PTDM" | "PTMI" | "PTPOS" |
        "PWID" | "PXBS" | "PXLE" | "PXTS" | "PYBS" | "PYLE" |
        "PYTS" | "PZAXI" | "PZLE" | "PARA" | "PARAM" | "UNIPAR" |
        "PKEY" | "PPRO" | "PSTR" | "PTRE" | "PTYP" | "PVER" |
        // loop/profile/negative geometry definitions。
        "ATTA" | "NAPP" | "NGMR" | "SJUS" | "SCTN" | "STWALL" |
        "AEXTR" | "CMPF" | "EXTR" | "NREV" | "NXTR" | "PANE" |
        "REVO" | "SCREED" | "ORRF" | "POHE" | "POIN" | "POLOOP" |
        "POLPTL" | "POLYHE" | "PTOF" | "VXREF" | "CLFL" | "JUSL" |
        "NSEX" | "NSRE" | "NUMB" | "RPRO" | "SEXT" | "SLOO" | "SPRO" |
        "SPVE" | "SREV" | "SVER" | "TUFL" |
        // 可见性、负实体和布尔生成开关。
        "LEVE" | "LEVEL" | "OBST" | "NEG" | "POSI" | "BOOL"
    ) || name.starts_with("PARA")
        || name.starts_with("PARAM")
}

/// PE/ATT 历史 diff 的字段路径是否会影响模型。
pub fn field_path_affects_model(path: &str) -> bool {
    let normalized = path.trim();
    if normalized.is_empty() {
        // 整条记录 Added/Deleted。
        return true;
    }
    attribute_affects_model(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_only_changes_do_not_trigger_geometry_generation() {
        for name in ["NAME", "DESC", "PURP", "FUNCTION", "att.NAME", "att.DESC"] {
            assert!(!attribute_affects_model(name), "{name}");
        }
    }

    #[test]
    fn transform_catalogue_topology_and_dimensions_trigger_generation() {
        for name in [
            "POS", "ORI", "OWNER", "children", "att.CATR", "SPRE", "DIAM", "XLEN", "HREF", "DDPR",
        ] {
            assert!(attribute_affects_model(name), "{name}");
        }
        assert!(field_path_affects_model(""));
    }

    #[test]
    fn only_approved_nouns_are_minimum_delivery_unit_roots() {
        for noun in DELIVERY_UNIT_ROOT_NOUNS {
            assert!(is_delivery_unit_root_noun(noun));
        }
        assert!(!is_delivery_unit_root_noun("HVAC"));
        assert!(!is_delivery_unit_root_noun("PIPE"));
        assert!(!is_delivery_unit_root_noun("SITE"));
    }
}
