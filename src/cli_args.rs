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
}

pub fn add_init_project_subcommand(command: Command) -> Command {
    command.subcommand(
        Command::new("init-project")
            .about("执行项目初始化：先全量扫描生成 DESI indextree，再生成 pe_transform")
            .arg(
                Arg::new("dbnums")
                    .long("dbnums")
                    .help("限定需要生成 pe_transform 的 dbnum 列表（逗号分隔）；不传则对扫描到的全部 DESI dbnum 生成")
                    .value_name("DBNUMS")
                    .value_delimiter(',')
                    .value_parser(clap::value_parser!(u32))
                    .num_args(1..),
            ),
    )
}
