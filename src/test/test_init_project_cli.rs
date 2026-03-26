use clap::Command;
use std::path::PathBuf;

#[test]
fn test_init_project_subcommand_has_dbnums_arg() {
    let command = crate::cli_args::add_init_project_subcommand(Command::new("aios-database"));
    let sub = command
        .get_subcommands()
        .find(|sub| sub.get_name() == "init-project")
        .expect("缺少 init-project 子命令");

    let arg_ids: Vec<String> = sub
        .get_arguments()
        .map(|arg| arg.get_id().to_string())
        .collect();

    assert!(arg_ids.iter().any(|id| id == "dbnums"));
}

#[test]
fn test_resolve_target_dbnums_prefers_cli_values() {
    let dbnums = crate::init_project::resolve_target_dbnums(
        Some(vec![5016, 21909, 5016]),
        vec![5001, 5016, 21909],
    )
    .expect("应优先使用 CLI 传入的 dbnums");
    assert_eq!(dbnums, vec![5016, 21909]);
}

#[test]
fn test_resolve_target_dbnums_uses_all_discovered_dbnums_when_cli_missing() {
    let dbnums = crate::init_project::resolve_target_dbnums(None, vec![21909, 5016, 21909, 5001])
        .expect("未传 --dbnums 时应使用扫描得到的全部 dbnums");
    assert_eq!(dbnums, vec![5001, 5016, 21909]);
}

#[test]
fn test_resolve_target_dbnums_requires_discovered_dbnums() {
    let err = crate::init_project::resolve_target_dbnums(None, vec![]).unwrap_err();
    assert!(err.to_string().contains("未发现任何 DESI dbnum"));
}

#[test]
fn test_resolve_target_dbnums_rejects_unknown_cli_dbnums() {
    let err =
        crate::init_project::resolve_target_dbnums(Some(vec![9999]), vec![5001, 5016]).unwrap_err();
    assert!(err.to_string().contains("未出现在本次 DESI 扫描结果中"));
}

#[test]
fn test_indextree_project_dir_candidates_include_project_name_fallback() {
    let candidates = crate::data_interface::db_meta_manager::indextree_project_dir_candidates(
        PathBuf::from("/Volumes/DPC/work/e3d_models").as_path(),
        "YCYK-E3D",
        &["SLYK".to_string(), "YCYK-E3D".to_string()],
    );

    assert_eq!(
        candidates.first(),
        Some(&PathBuf::from("/Volumes/DPC/work/e3d_models/YCYK-E3D"))
    );
    assert!(candidates.contains(&PathBuf::from("/Volumes/DPC/work/e3d_models/SLYK")));
}
