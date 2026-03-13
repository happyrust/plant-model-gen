//! 验证 FITT transform 计算正确性
//!
//! PDMS 实际值（q pos wrt /*, q wori）：
//!   FITT 17496_142306: pos=[-3160, -21150, 5470], wori="Y is X and Z is -Y"
//!   FITT 17496_142305: pos=[490, -21150, 5470]
//!   STWALL 17496_106028: pos=[-3360, -21150, 3290]
//!
//! 运行:
//! ```
//! cargo test --features gen_model --lib test::test_model_regression::test_fitt_transform -- --nocapture --ignored
//! ```

use aios_core::RefnoEnum;
use aios_core::transform::get_local_mat4;
use glam::DVec3;

/// PDMS 期望值结构
struct ExpectedTransform {
    refno: &'static str,
    world_pos: DVec3,
}

const STWALL_REFNO: &str = "17496_106028";
const STWALL_WORLD_POS: DVec3 = DVec3::new(-3360.0, -21150.0, 3290.0);

const FITT_CASES: &[ExpectedTransform] = &[
    ExpectedTransform {
        refno: "17496_142306",
        world_pos: DVec3::new(-3160.0, -21150.0, 5470.0),
    },
    ExpectedTransform {
        refno: "17496_142305",
        world_pos: DVec3::new(490.0, -21150.0, 5470.0),
    },
];

#[tokio::test]
#[ignore]
async fn test_fitt_local_mat4_not_identity() {
    if let Err(e) = aios_core::init_surreal().await {
        eprintln!("⚠️  跳过：SurrealDB 连接失败: {:?}", e);
        return;
    }

    for case in FITT_CASES {
        let refno: RefnoEnum = case.refno.parse().unwrap();
        let local_mat = get_local_mat4(refno).await.expect("get_local_mat4 失败");

        match &local_mat {
            Some(mat) => {
                let pos = mat.w_axis.truncate();
                println!(
                    "FITT {} local translation: [{:.1}, {:.1}, {:.1}]",
                    case.refno, pos.x, pos.y, pos.z
                );
                assert!(
                    pos.length() > 0.01,
                    "FITT {} local_mat 不应为 IDENTITY，translation={:?}",
                    case.refno,
                    pos
                );
                println!("  ✅ local_mat 非 IDENTITY");
            }
            None => {
                panic!("FITT {} get_local_mat4 返回 None", case.refno);
            }
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_fitt_world_position_matches_pdms() {
    if let Err(e) = aios_core::init_surreal().await {
        eprintln!("⚠️  跳过：SurrealDB 连接失败: {:?}", e);
        return;
    }

    // 获取 STWALL 的 world_mat
    let stwall_refno: RefnoEnum = STWALL_REFNO.parse().unwrap();
    let stwall_world = aios_core::transform::get_world_mat4(stwall_refno, false)
        .await
        .expect("get_world_mat4 STWALL 失败")
        .expect("STWALL world_mat 为 None");

    let (_, _, stwall_trans) = stwall_world.to_scale_rotation_translation();
    println!(
        "STWALL world pos: [{:.1}, {:.1}, {:.1}]",
        stwall_trans.x, stwall_trans.y, stwall_trans.z
    );
    println!(
        "STWALL expected:  [{:.1}, {:.1}, {:.1}]",
        STWALL_WORLD_POS.x, STWALL_WORLD_POS.y, STWALL_WORLD_POS.z
    );

    const TOLERANCE: f64 = 1.0; // 1mm

    for case in FITT_CASES {
        let refno: RefnoEnum = case.refno.parse().unwrap();
        let local_mat = get_local_mat4(refno)
            .await
            .expect("get_local_mat4 失败")
            .expect("local_mat 为 None");

        // world = parent_world * local
        let computed_world = stwall_world * local_mat;
        let (_, rot, trans) = computed_world.to_scale_rotation_translation();

        let computed_pos = DVec3::new(trans.x, trans.y, trans.z);
        let diff = (computed_pos - case.world_pos).length();

        println!("\nFITT {}", case.refno);
        println!(
            "  computed: [{:.1}, {:.1}, {:.1}]",
            trans.x, trans.y, trans.z
        );
        println!(
            "  PDMS:     [{:.1}, {:.1}, {:.1}]",
            case.world_pos.x, case.world_pos.y, case.world_pos.z
        );
        println!(
            "  rotation: [{:.4}, {:.4}, {:.4}, {:.4}]",
            rot.x, rot.y, rot.z, rot.w
        );
        println!("  diff:     {:.2}mm", diff);

        assert!(
            diff < TOLERANCE,
            "FITT {} world position 偏差过大: {:.2}mm (tolerance={:.0}mm)\n  computed: {:?}\n  expected: {:?}",
            case.refno,
            diff,
            TOLERANCE,
            computed_pos,
            case.world_pos
        );
        println!("  ✅ 验证通过");
    }
}
