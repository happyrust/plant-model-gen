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
                .help("Export dbnum instances from SurrealDB as multi-table Parquet (instances/geo_instances/tubings/transforms/aabb) for DuckDB querying. If --dbnum is omitted, scans all distinct dbnums from inst_relate and exports each")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-pdms-tree-parquet")
                .long("export-pdms-tree-parquet")
                .help("Export PDMS TreeIndex + pe.name as Parquet (pdms_tree_{dbnum}.parquet) for DuckDB-WASM model tree queries")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-world-sites-parquet")
                .long("export-world-sites-parquet")
                .help("Export WORL->SITE nodes as Parquet (world_sites.parquet) for DuckDB-WASM (Full Parquet Mode)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("export-dbnum-instances")
                .long("export-dbnum-instances")
                .help("Export dbnum instances from SurrealDB (default: Parquet format; use --export-dbnum-instances-json for JSON). If --dbnum is omitted, scans all distinct dbnums from inst_relate and exports each")
                .action(clap::ArgAction::SetTrue),
        )
}
