//! 验证 FITT transform 计算正确性
//!
//! PDMS 实际值（q pos wrt /*, q wori）：
//!   FITT 17496_142306: pos=[-3160, -21150, 5470], wori="Y is X and Z is -Y"
//!   FITT 17496_142305: pos=[490, -21150, 5470]
//!   STWALL 17496_106028: pos=[-3360, -21150, 3290]
//!
//! 用法:
//!   cargo run --example verify_fitt_transform

use aios_core::transform::get_local_mat4;
use aios_core::*;
use anyhow::Result;
use glam::DVec3;

struct FittCase {
    refno: &'static str,
    expected_world_pos: DVec3,
}

const STWALL_REFNO: &str = "17496_106028";

const CASES: &[FittCase] = &[
    FittCase {
        refno: "17496_142306",
        expected_world_pos: DVec3::new(-3160.0, -21150.0, 5470.0),
    },
    FittCase {
        refno: "17496_142305",
        expected_world_pos: DVec3::new(490.0, -21150.0, 5470.0),
    },
];

#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    println!("═══════════════════════════════════════════════════");
    println!("  FITT Transform 验证 (STWALL 17496_106028 子件)");
    println!("═══════════════════════════════════════════════════");

    // 1. STWALL world_mat（从 pe_transform 缓存）
    let stwall_refno = RefnoEnum::from(STWALL_REFNO);
    let stwall_world = aios_core::transform::get_world_mat4(stwall_refno, false)
        .await?
        .expect("STWALL world_mat 为 None");
    let (_, stwall_rot, stwall_trans) = stwall_world.to_scale_rotation_translation();
    println!("\n📍 STWALL world:");
    println!(
        "   translation: [{:.1}, {:.1}, {:.1}]",
        stwall_trans.x, stwall_trans.y, stwall_trans.z
    );
    println!(
        "   rotation:    [{:.4}, {:.4}, {:.4}, {:.4}]",
        stwall_rot.x, stwall_rot.y, stwall_rot.z, stwall_rot.w
    );

    let tolerance = 1.0; // 1mm
    let mut all_pass = true;

    for case in CASES {
        println!("\n───────────────────────────────────────────────");
        println!("🔍 FITT {}", case.refno);

        let refno = RefnoEnum::from(case.refno);

        // 2. 查看 FITT 属性
        let att = get_named_attmap(refno).await?;
        println!("   POSL: {:?}", att.get_str("POSL"));
        println!("   DELP: {:?}", att.get_dvec3("DELP"));
        println!("   ZDIS: {:?}", att.get_f64("ZDIS"));

        // 3. 计算 local_mat
        let local_mat = get_local_mat4(refno).await?;
        match &local_mat {
            Some(mat) => {
                let (_, rot, trans) = mat.to_scale_rotation_translation();
                println!(
                    "   local translation: [{:.1}, {:.1}, {:.1}]",
                    trans.x, trans.y, trans.z
                );
                println!(
                    "   local rotation:    [{:.4}, {:.4}, {:.4}, {:.4}]",
                    rot.x, rot.y, rot.z, rot.w
                );

                if DVec3::new(trans.x, trans.y, trans.z).length() < 0.01 {
                    println!("   ❌ local_mat 是 IDENTITY（bug 未修复）");
                    all_pass = false;
                    continue;
                }

                // 4. world = parent_world * local
                let computed_world = stwall_world * *mat;
                let (_, cw_rot, cw_trans) = computed_world.to_scale_rotation_translation();
                let computed_pos = DVec3::new(cw_trans.x, cw_trans.y, cw_trans.z);
                let diff = (computed_pos - case.expected_world_pos).length();

                println!(
                    "   computed world: [{:.1}, {:.1}, {:.1}]",
                    cw_trans.x, cw_trans.y, cw_trans.z
                );
                println!(
                    "   PDMS expected:  [{:.1}, {:.1}, {:.1}]",
                    case.expected_world_pos.x, case.expected_world_pos.y, case.expected_world_pos.z
                );
                println!(
                    "   world rotation: [{:.4}, {:.4}, {:.4}, {:.4}]",
                    cw_rot.x, cw_rot.y, cw_rot.z, cw_rot.w
                );
                println!("   差异: {:.2}mm", diff);

                if diff < tolerance {
                    println!("   ✅ 验证通过");
                } else {
                    println!("   ❌ 偏差过大 ({:.2}mm > {:.0}mm)", diff, tolerance);
                    all_pass = false;
                }
            }
            None => {
                println!("   ❌ get_local_mat4 返回 None");
                all_pass = false;
            }
        }
    }

    println!("\n═══════════════════════════════════════════════════");
    if all_pass {
        println!("✅ 全部验证通过！");
    } else {
        println!("❌ 存在验证失败项");
    }
    println!("═══════════════════════════════════════════════════");

    Ok(())
}
