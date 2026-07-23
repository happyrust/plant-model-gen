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

/// 属性对模型生成输入的影响。未知属性保守触发，只有明确列出的业务元数据可跳过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeModelImpact {
    AffectsModel,
    KnownNeutral,
    Unknown,
}

/// 属性改动的「模型影响 effect」——对齐 Core3D `DCHC/EVALAT` 的**作用域路由**。
///
/// 依据 `docs/reverse/core_dll_noun_att_model_update.md` §15：`EVALAT` 里 DCHC 不是
/// 严重度标量，而是路由选择器（0=不变、1=重定向到关联/owner、2=自身、3/4=自身+依赖
/// 闭包）。Rust 侧静态拿不到每属性 DCHC 原值，故本枚举是基于 §13.2 几何输入类别 +
/// §15 路由语义 + BAKREF/ATTABK 反向闭包证据的**保守建模**，不是 DCHC dump。
///
/// 不变量：`classify_attribute_model_effect(n).affects_model() == attribute_affects_model(n)`，
/// 因此三态 `AttributeModelImpact` 可完全由本 effect 派生，行为与旧扁平白名单一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeModelEffect {
    /// DCHC≈0：业务元数据，不影响模型（NAME/DESC/PURP/FUNCTION）。
    DataOnly,
    /// DCHC 2（transform 子集）：世界变换/位姿，理论上只需失效 `pe_transform` 子树。
    Transform,
    /// DCHC 2/3：直接改变本元素几何 mesh（尺寸/参数/图元/profile）。
    DirectGeometry,
    /// DCHC 1/3/4：目录/规格/引用——改动应重定向/扩散到 owner 与被引用实例闭包。
    DependencyCascade,
    /// 结构/成员/类型：owner、children、noun、type（改层级或改生成分派）。
    StructuralMembership,
    /// 未知：由上层 `unknown_fallback` 保守触发。
    Unknown,
}

impl AttributeModelEffect {
    /// 是否构成已知的模型输入变化（`Unknown` 不在此列——交由上层保守触发）。
    pub const fn affects_model(self) -> bool {
        matches!(
            self,
            Self::Transform
                | Self::DirectGeometry
                | Self::DependencyCascade
                | Self::StructuralMembership
        )
    }

    /// 是否可安全跳过模型欠账（纯业务元数据）。
    pub const fn is_known_neutral(self) -> bool {
        matches!(self, Self::DataOnly)
    }

    /// 该 effect 是否需要把欠账**重定向/扩散**到关联对象
    /// （owner / 被引用实例 / 成员闭包），而不仅是被改元素自身。
    ///
    /// 对应 Core3D DCHC code 1（重定向）与目录/结构闭包（§11.2/§15.4）。生成侧
    /// 消费方（如 `sesno_increment`）可据此决定是否触发 owner 上溯 / 目录反查。
    pub const fn needs_dependency_redirect(self) -> bool {
        matches!(self, Self::DependencyCascade | Self::StructuralMembership)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DataOnly => "data_only",
            Self::Transform => "transform",
            Self::DirectGeometry => "direct_geometry",
            Self::DependencyCascade => "dependency_cascade",
            Self::StructuralMembership => "structural_membership",
            Self::Unknown => "unknown",
        }
    }
}

pub fn normalize_attribute_name(raw_name: &str) -> String {
    raw_name
        .trim()
        .trim_start_matches("att.")
        .trim_start_matches("ATT.")
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
}

pub fn classify_attribute_model_impact(raw_name: &str) -> AttributeModelImpact {
    // 三态由 effect 派生，effect 是模型影响的单一真相源（见不变量说明）。
    match classify_attribute_model_effect(raw_name) {
        AttributeModelEffect::DataOnly => AttributeModelImpact::KnownNeutral,
        AttributeModelEffect::Unknown => AttributeModelImpact::Unknown,
        _ => AttributeModelImpact::AffectsModel,
    }
}

/// 结构 / 成员 / 类型属性：改动会改变层级或生成分派（对应 §6.2、§13.3）。
fn is_structural_membership_attr(name: &str) -> bool {
    matches!(name, "OWNER" | "CHILDREN" | "NOUN" | "TYPE")
}

/// 目录 / 规格 / 设计表 / 引用类属性——Core3D DCHC 1/3/4：改动需重定向或扩散到
/// 被引用元素、目录反向实例与 owner 闭包（§11.2.3/§11.2.4/§13.2-B、BAKREF/ATTABK）。
/// 全部是 `attribute_affects_model` 白名单的子集，保证不变量成立。
fn is_dependency_ref_attr(name: &str) -> bool {
    matches!(
        name,
        // 目录 / 规格 / 元件引用。
        "CATR" | "CREF" | "SPRE" | "SPCO" | "SCOM" | "SCREF" | "PRTREF" | "PSPE" |
        // 连接引用 / 管件规格引用。
        "HREF" | "TREF" | "LSTU" | "HSTU" |
        // 目录几何 / 负实体几何 / 设计表引用与模板。
        "GMREF" | "GMRE" | "GSTR" | "GTYP" | "NGMR" | "DTRE" | "DPRO" | "ISPE" | "TMPL"
    )
}

/// 位姿 / 变换类属性（DCHC 2·transform 子集，§13.2-A）：理论上只需失效世界变换，
/// 不必整体 mesh 重建。全部是 `attribute_affects_model` 白名单的子集。
fn is_transform_attr(name: &str) -> bool {
    matches!(
        name,
        "POS" | "POSL" | "POSS" | "POSE" | "NPOS" | "CPOS" | "PPOS" |
        "ORI" | "PORI" | "YDIR" | "ZDIR" |
        "ADIR" | "RDIR" | "LDIR" | "HDIR" | "TDIR" |
        "PAXI" | "PZAXI" | "PLAX" | "BANG" | "HPOS" | "TPOS"
    )
}

/// 把一个属性映射到 [`AttributeModelEffect`]（对齐 Core3D DCHC 路由，见枚举文档）。
///
/// 判定顺序：结构/成员 → 目录/引用闭包 → 位姿/变换 → 业务元数据(data-only) →
/// 命中扁平白名单的其它项归为直接几何 → 否则未知。此顺序保证与
/// `attribute_affects_model` 的布尔判定一致（`affects_model()` 不变量）。
pub fn classify_attribute_model_effect(raw_name: &str) -> AttributeModelEffect {
    let name = normalize_attribute_name(raw_name);
    if is_structural_membership_attr(&name) {
        return AttributeModelEffect::StructuralMembership;
    }
    if is_dependency_ref_attr(&name) {
        return AttributeModelEffect::DependencyCascade;
    }
    if is_transform_attr(&name) {
        return AttributeModelEffect::Transform;
    }
    if matches!(name.as_str(), "NAME" | "DESC" | "PURP" | "FUNCTION") {
        return AttributeModelEffect::DataOnly;
    }
    if attribute_affects_model(&name) {
        // 白名单里其余的尺寸 / 参数 / 图元 / profile / 负实体开关等 → 直接几何。
        return AttributeModelEffect::DirectGeometry;
    }
    AttributeModelEffect::Unknown
}

/// 判断单个 E3D/PDMS 属性是否会改变生成器的模型输入。
///
/// 名称统一按大写比较。该集合来自当前生成链路读取的定位、目录引用、布尔关系、
/// 管路连接和 primitive 参数。此 bool 只表示“命中已知影响集合”；未命中项会由
/// `classify_attribute_model_impact` 标为 Unknown，并在增量采集层保守触发。
///
/// 清单经 core.dll/Core3D 逆向 + 运行库 `att_meta`(702) 三方交叉校验补齐，取“宁多勿漏”
/// （漏判=模型陈旧的正确性 bug；误判=多算一次成本可控）。目标语义近似 Core3D 的
/// `DCHC/EVALAT` 模型影响层；`wnoevt` 仅是 core 事件门。详见 ADR-0009 及
/// `docs/reverse/core_dll_noun_att_model_update.md` §13/§14。
pub fn attribute_affects_model(raw_name: &str) -> bool {
    let name = normalize_attribute_name(raw_name);

    matches!(
        name.as_str(),
        // 层级/类型与世界定位。
        "OWNER" | "CHILDREN" | "NOUN" | "TYPE" | "POS" | "ORI" |
        // catalogue / specification / design-template 依赖。
        "CATR" | "CREF" | "SPRE" | "SPCO" | "SCOM" | "SCREF" |
        // PRTREF：SPCO.PRTREF 在 VDESPF 有派生属性回写、TABITE 以其为目录跳板
        // （Core3D §11.2.3/§11.2.4/§14.3，noun-scoped；现 API 无 noun，宁多勿漏先全局收录）。
        "PRTREF" |
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
        "LEVE" | "LEVEL" | "OBST" | "NEG" | "POSI" | "BOOL" |
        // ── §13/§14 逆向补缺（core.dll/Core3D + att_meta 交叉校验，2026-07-22，见 ADR-0009）──
        // 顶点/坐标：SPVE/SVER/PVER 等顶点改坐标时 modified_attrs 为 PX/PY/PZ，父不一定重列子表。
        "PX" | "PY" | "PZ" | "DX" | "DY" |
        // 定位变体、朝向 Y/Z 轴分量与弯角。
        "POSL" | "POSS" | "POSE" | "NPOS" | "CPOS" | "YDIR" | "ZDIR" | "BANG" |
        // 管路布线/几何：坡降/离开点/曲率/外径/路由/排水端点。
        "ZDIS" | "LEAV" | "CURD" | "CURTYP" | "OPDI" | "ROUT" | "DRNS" | "DRNE" | "DETR" |
        // 规格/类型/布线定位（CTYP/JFRE 系 Core3D VDESPT (noun,attr) 特例）。
        "PSPE" | "CTYP" | "JFRE" | "JLIN" |
        // 设计增量位置 / 保温半径 / P-line 方向键。
        "DELP" | "RINS" | "PKDI"
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
    !matches!(
        classify_attribute_model_impact(normalized),
        AttributeModelImpact::KnownNeutral
    )
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
            "PRTREF", "att.PRTREF",
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

    #[test]
    fn effect_maps_attributes_to_dchc_routing_buckets() {
        use AttributeModelEffect::*;
        let cases = [
            // data-only（DCHC≈0）。
            ("NAME", DataOnly),
            ("att.DESC", DataOnly),
            ("PURP", DataOnly),
            // 结构/成员/类型。
            ("OWNER", StructuralMembership),
            ("children", StructuralMembership),
            ("NOUN", StructuralMembership),
            ("TYPE", StructuralMembership),
            // 目录/规格/引用闭包（DCHC 1/3/4）。
            ("CATR", DependencyCascade),
            ("SPRE", DependencyCascade),
            ("att.PRTREF", DependencyCascade),
            ("SPCO", DependencyCascade),
            ("HREF", DependencyCascade),
            ("NGMR", DependencyCascade),
            ("TMPL", DependencyCascade),
            // 位姿/变换（DCHC 2·transform 子集）。
            ("POS", Transform),
            ("ORI", Transform),
            ("YDIR", Transform),
            ("ADIR", Transform),
            // 直接几何（尺寸/参数/图元）。
            ("HEIG", DirectGeometry),
            ("DIAM", DirectGeometry),
            ("PARA", DirectGeometry),
            ("PDIA", DirectGeometry),
            // 未知 → 保守。
            ("ZZUNKNOWN", Unknown),
        ];
        for (name, want) in cases {
            assert_eq!(classify_attribute_model_effect(name), want, "{name}");
        }
    }

    #[test]
    fn effect_affects_model_matches_flat_allowlist_invariant() {
        // 关键不变量：effect.affects_model() 必须与扁平 attribute_affects_model 完全一致，
        // 保证三态判定（以及增量管线的 trigger/neutral）行为零回归。
        let names = [
            "NAME", "DESC", "PURP", "FUNCTION", "OWNER", "CHILDREN", "NOUN", "TYPE", "POS", "ORI",
            "CATR", "SPRE", "SPCO", "SCOM", "PRTREF", "HREF", "TREF", "NGMR", "TMPL", "DPRO",
            "DTRE", "HEIG", "DIAM", "RADI", "XLEN", "PARA", "PARAM", "PDIA", "PZAXI", "YDIR",
            "ZDIR", "BANG", "LEAV", "CTYP", "JFRE", "NEG", "BOOL", "OBST", "LEVE", "PX", "PY",
            "PZ", "att.CATR", "att.NAME", "ZZUNKNOWN", "SOMETHINGELSE", "",
        ];
        for name in names {
            let effect = classify_attribute_model_effect(name);
            assert_eq!(
                effect.affects_model(),
                attribute_affects_model(name),
                "effect/allowlist disagree for {name:?} (effect={})",
                effect.as_str()
            );
            // 三态一致性：DataOnly⟺KnownNeutral、Unknown⟺Unknown、其余⟺AffectsModel。
            let impact = classify_attribute_model_impact(name);
            let expected = match effect {
                AttributeModelEffect::DataOnly => AttributeModelImpact::KnownNeutral,
                AttributeModelEffect::Unknown => AttributeModelImpact::Unknown,
                _ => AttributeModelImpact::AffectsModel,
            };
            assert_eq!(impact, expected, "impact/effect disagree for {name:?}");
        }
    }

    #[test]
    fn dependency_and_structural_effects_request_redirect() {
        assert!(classify_attribute_model_effect("CATR").needs_dependency_redirect());
        assert!(classify_attribute_model_effect("OWNER").needs_dependency_redirect());
        // 直接几何 / 变换 / 数据-only 不需要反向重定向。
        assert!(!classify_attribute_model_effect("HEIG").needs_dependency_redirect());
        assert!(!classify_attribute_model_effect("POS").needs_dependency_redirect());
        assert!(!classify_attribute_model_effect("NAME").needs_dependency_redirect());
    }
}
