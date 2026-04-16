use std::collections::HashSet;

use anyhow::{Context, Result, bail};

use crate::data_interface::db_meta;
use crate::data_interface::db_meta_manager::generate_desi_indextree;
use crate::options::DbOptionExt;
use crate::pe_transform_refresh::refresh_pe_transform_for_dbnums_compat;

pub fn resolve_target_dbnums(
    cli_dbnums: Option<Vec<u32>>,
    discovered_dbnums: Vec<u32>,
) -> Result<Vec<u32>> {
    let mut discovered_dbnums = discovered_dbnums;
    discovered_dbnums.sort_unstable();
    discovered_dbnums.dedup();

    if discovered_dbnums.is_empty() {
        bail!("db_meta 中没有任何 DESI dbnum（请先完成第 1 步 scene_tree / indextree 生成）");
    }

    let Some(mut dbnums) = cli_dbnums else {
        return Ok(discovered_dbnums);
    };

    dbnums.sort_unstable();
    dbnums.dedup();

    if dbnums.is_empty() {
        bail!("目标 dbnums 为空，请检查 --dbnums 参数");
    }

    let discovered_set: HashSet<u32> = discovered_dbnums.iter().copied().collect();
    let missing_dbnums: Vec<u32> = dbnums
        .iter()
        .copied()
        .filter(|dbnum| !discovered_set.contains(dbnum))
        .collect();
    if !missing_dbnums.is_empty() {
        bail!(
            "指定的 dbnums 未出现在本次 DESI 扫描结果中: {:?}；可用 dbnums={:?}",
            missing_dbnums,
            discovered_dbnums
        );
    }

    Ok(dbnums)
}

pub async fn run_init_project_mode(
    db_option_ext: DbOptionExt,
    cli_dbnums: Option<Vec<u32>>,
) -> Result<()> {
    let configured_dbnums = db_option_ext
        .inner
        .manual_db_nums
        .clone()
        .filter(|values| !values.is_empty());
    let ignore_manual_dbnums = configured_dbnums.is_none();

    println!(
        "🚀 开始初始化项目: project={} ns={}",
        db_option_ext.inner.project_name, db_option_ext.inner.surreal_ns
    );

    println!(
        "🌲 第 1 步：生成 scene_tree（DESI indextree：output/<项目>/scene_tree/*.tree + db_meta_info.json）"
    );
    generate_desi_indextree(ignore_manual_dbnums)
        .context("scene_tree / DESI indextree 生成失败")?;

    println!("🔌 第 2 步：连接 SurrealDB 并从磁盘加载 db_meta（校验 scene_tree 元数据）");
    aios_core::init_surreal()
        .await
        .context("初始化 Surreal 连接失败")?;
    db_meta()
        .try_load_default()
        .context("加载 db_meta_info.json 失败（确认第 1 步已写出 scene_tree）")?;

    let discovered_dbnums = db_meta().get_all_dbnums();
    let dbnums = resolve_target_dbnums(cli_dbnums.or(configured_dbnums), discovered_dbnums)?;
    println!(
        "🎯 第 3 步：目标 dbnums（将刷新其 pe_transform）: {:?}",
        dbnums
    );

    println!("🔄 第 4 步：写入/刷新 pe_transform（依赖库中已有 pe 与 scene 数据）");
    let refresh_count = refresh_pe_transform_for_dbnums_compat(&dbnums)
        .await
        .context("刷新 pe_transform 失败")?;

    println!(
        "✅ 项目初始化完成: project={} dbnums={:?} pe_transform_nodes={}",
        db_option_ext.inner.project_name, dbnums, refresh_count
    );

    Ok(())
}
