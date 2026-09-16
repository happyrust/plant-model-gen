use aios_core::*;
use anyhow::Result;
use glam::DVec3;

/// FIXING 方位检查测试
///
/// 验证 FIXING 17496/152153 的世界坐标是否与 PDMS 一致：
///   q pos wrt /* → Position X 5760.911mm Y 15408.258mm Z 5700mm
///
/// 用法:
///   cargo run --example verify_fixing_position
#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;
    println!("═══════════════════════════════════════════════════");
    println!("  FIXING 方位检查: 17496/152153");
    println!("═══════════════════════════════════════════════════");

    let target_refno = RefnoEnum::from("17496_152153");
    let expected_pos = DVec3::new(5760.911, 15408.258, 5700.0);
    let tolerance_mm = 1.0;

    // ─── 1. 基础信息 ───
    let att = get_named_attmap(target_refno).await?;
    let owner_refno = att.get_owner();
    let owner_att = get_named_attmap(owner_refno).await?;
    let cur_type = att.get_type_str().to_string();
    let owner_type = owner_att.get_type_str().to_string();

    println!("\n📋 元素信息:");
    println!("   refno     = {}", target_refno);
    println!("   type      = {}", cur_type);
    println!("   owner     = {} ({})", owner_refno, owner_type);
    println!("   POS       = {:?}", att.get_position());
    println!("   POSL      = {:?}", att.get_str("POSL"));
    println!("   PKDI      = {:?}", att.get_f64("PKDI"));
    println!("   ZDIS      = {:?}", att.get_f64("ZDIS"));
    println!("   DELP      = {:?}", att.get_dvec3("DELP"));
    println!("   BANG      = {:?}", att.get_f32("BANG"));
    println!("   ORI       = {:?}", att.get_rotation());

    // ─── 2. 父节点信息 ───
    println!("\n📋 父节点信息:");
    println!("   JUSL      = {:?}", owner_att.get_str("JUSL"));
    println!("   LMIRR     = {:?}", owner_att.get_bool("LMIRR"));
    println!("   POS       = {:?}", owner_att.get_position());
    println!("   DPOSS     = {:?}", owner_att.get_dposs());
    println!("   DPOSE     = {:?}", owner_att.get_dpose());

    // 如果父节点是 STWALL/GENSEC, 查看其子节点
    if owner_type == "STWALL" || owner_type == "GENSEC" || owner_type == "WALL" {
        let children = get_children_refnos(owner_refno).await?;
        println!("   children  = {} 个", children.len());
        for c in &children {
            let c_type = get_type_name(*c).await.unwrap_or_default();
            println!("     - {} ({})", c, c_type);
        }
    }

    // ─── 3. 祖先链 ───
    let ancestors = rs_surreal::query_ancestor_refnos(target_refno).await?;
    println!("\n📋 祖先链:");
    for a in &ancestors {
        if *a == target_refno {
            continue;
        }
        let a_type = get_type_name(*a).await.unwrap_or_default();
        println!("   {} ({})", a, a_type);
    }

    // ─── 4. 计算 local transform ───
    println!("\n📋 变换计算:");
    let local_mat = transform::get_local_mat4(target_refno).await?;
    if let Some(m) = local_mat {
        let t = m.col(3);
        println!(
            "   local_mat translation = ({:.3}, {:.3}, {:.3})",
            t.x, t.y, t.z
        );
    } else {
        println!("   ⚠️  local_mat = None (策略未返回变换)");
    }

    // ─── 5. 计算 world transform ───
    let world_mat = transform::get_world_mat4(target_refno, false).await?;
    if let Some(m) = world_mat {
        let t = m.col(3);
        let calculated_pos = DVec3::new(t.x, t.y, t.z);
        let diff = calculated_pos - expected_pos;
        let dist = diff.length();

        println!("   world pos = ({:.3}, {:.3}, {:.3})", t.x, t.y, t.z);

        println!("\n═══════════════════════════════════════════════════");
        println!(
            "  期望位置: ({:.3}, {:.3}, {:.3})",
            expected_pos.x, expected_pos.y, expected_pos.z
        );
        println!(
            "  计算位置: ({:.3}, {:.3}, {:.3})",
            calculated_pos.x, calculated_pos.y, calculated_pos.z
        );
        println!(
            "  偏差:     ({:.3}, {:.3}, {:.3})  |{:.3}| mm",
            diff.x, diff.y, diff.z, dist
        );

        if dist < tolerance_mm {
            println!("  ✅ 通过 (偏差 < {:.1}mm)", tolerance_mm);
        } else {
            println!("  ❌ 失败 (偏差 {:.3}mm > {:.1}mm)", dist, tolerance_mm);
        }
        println!("═══════════════════════════════════════════════════");
    } else {
        println!("   ❌ world_mat = None");
        println!("\n═══════════════════════════════════════════════════");
        println!("  ❌ 无法计算世界变换");
        println!("═══════════════════════════════════════════════════");
    }

    // ─── 6. 父节点 world transform（用于对比分析） ───
    let parent_world = transform::get_world_mat4(owner_refno, false).await?;
    if let Some(m) = parent_world {
        let t = m.col(3);
        println!("\n📋 父节点世界位置: ({:.3}, {:.3}, {:.3})", t.x, t.y, t.z);
    }

    Ok(())
}
