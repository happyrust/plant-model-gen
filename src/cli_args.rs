use clap::{Arg, Command};

pub fn add_export_instance_args(command: Command) -> Command {
    command
        .arg(
            Arg::new("export-dbnum-instances-json")
                .long("export-dbnum-instances-json")
                .help("Export dbnum instances as JSON (default: SurrealDB + compact format)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("from-cache")
                .long("from-cache")
                .help("Use model cache instead of SurrealDB for JSON export")
                .action(clap::ArgAction::SetTrue)
                .requires("export-dbnum-instances-json"),
        )
        .arg(
            Arg::new("detailed")
                .long("detailed")
                .help("Export detailed JSON format with all fields (default: compact)")
                .action(clap::ArgAction::SetTrue)
                .requires("export-dbnum-instances-json"),
        )
        .arg(
            Arg::new("export-parquet")
                .long("export-parquet")
                .help("Export dbnum instances from SurrealDB as multi-table Parquet (instances/geo_instances/tubings/transforms/aabb). If --dbnum is omitted, scans all distinct dbnums from inst_relate and exports each")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-pdms-tree-parquet")
                .long("export-pdms-tree-parquet")
                .help("Export PDMS TreeIndex + pe.name as Parquet (pdms_tree_{dbnum}.parquet) for model tree queries")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-world-sites-parquet")
                .long("export-world-sites-parquet")
                .help("Export WORL->SITE nodes as Parquet (world_sites.parquet)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-dbnum-instances")
                .long("export-dbnum-instances")
                .help("Export dbnum instances from SurrealDB (default: Parquet format; use --export-dbnum-instances-json for JSON). If --dbnum is omitted, scans all distinct dbnums from inst_relate and exports each")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-dbnum-instances-web")
                .long("export-dbnum-instances-web")
                .help("Export delivery-code V2 JSON (inline matrix + uniforms). With --root-model / --debug-model REFNO: export only that subtree → instances_web_root_<refno>.json (BRAN/EQUI smoke tests)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-v3")
                .long("export-v3")
                .help("Export v3 JSON (hash-dedup transforms, no color/name/lod fields). Requires --dbnum")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("v3-target-unit")
                .long("v3-target-unit")
                .help("Target length unit for v3 export (mm/m/ft, default: mm)")
                .value_name("UNIT")
                .requires("export-v3"),
        )
        .arg(
            Arg::new("v3-rotate")
                .long("v3-rotate")
                .help("Apply Z-up → Y-up rotation in v3 export")
                .action(clap::ArgAction::SetTrue)
                .requires("export-v3"),
        )
        .arg(
            Arg::new("merge-v3")
                .long("merge-v3")
                .help("Merge all per-dbnum instances_v3_*.json into a single instances_v3.json (no DB needed)")
                .action(clap::ArgAction::SetTrue),
        )
}

pub fn add_init_project_subcommand(command: Command) -> Command {
    command.subcommand(
        Command::new("init-project")
            .about(
                "项目冷启动推荐入口：① scene_tree（*.tree + db_meta_info.json）② 连库加载 db_meta ③ pe_transform",
            )
            .long_about(
                "顺序固定为：\n\
                 1) 全量扫描 PDMS，生成 DESI 的 scene_tree（output/<项目>/scene_tree/*.tree）并更新 db_meta_info.json；\n\
                 2) 连接 SurrealDB，加载 db_meta；\n\
                 3) 对指定（或全部）DESI dbnum 刷新 pe_transform。\n\
                 完成后再执行常规模型生成（如带 --dbnum 的全量/增量 gen_model）。\n\
                 示例：aios-database -c db_options/DbOption-zsy init-project --dbnums 5525",
            )
            .arg(
                Arg::new("dbnums")
                    .long("dbnums")
                    .help("仅对这些 dbnum 刷新 pe_transform（逗号分隔）；省略则对 db_meta 中全部 DESI dbnum 刷新")
                    .value_name("DBNUMS")
                    .value_delimiter(',')
                    .value_parser(clap::value_parser!(u32))
                    .num_args(1..),
            ),
    )
}
