use crate::options::DbOptionExt;
use crate::version_management::baseline_state::validate_baseline_state_request;
use crate::version_management::bounded_runner::{
    BoundedCommandRunRequest, parse_argv_json, parse_env_assignments, read_bounded_run_status,
    request_bounded_run_cancel, run_bounded_command,
};
use crate::version_management::ducklake_store::ModelVersionDuckLakeStore;
use crate::version_management::history_baseline::{
    HistoryBaselineInspectRequest, inspect_history_baseline,
};
use crate::version_management::history_replay_plan::prepare_history_replay;
use crate::version_management::history_replay_validation::{
    ensure_history_replay_publishable, validate_history_replay_package,
};
use crate::version_management::missing_mesh_repair::repair_missing_meshes;
use crate::version_management::model_release::{
    annotate_model_release, diff_model_release_units, diff_model_releases,
    get_model_component_unit_impacts, get_model_release_events, get_model_release_mesh_assets,
    index_model_release_components, index_model_release_mesh_assets, index_model_release_units,
    list_model_releases, migrate_model_version_catalog, publish_history_model_release,
    reconcile_model_release, register_model_release, validate_model_release_pair_readiness,
};
use crate::version_management::physical_baseline_snapshot::prepare_physical_baseline_snapshot;
use crate::version_management::scene_tree_artifact::restore_scene_tree_artifact;
use crate::version_management::source_observation::{
    SourceObservationBuildRequest, build_source_observation_manifest,
    write_source_observation_manifest,
};
use crate::version_management::types::{
    ModelBaselineStateValidationRequest, ModelHistoryReleasePublishRequest,
    ModelHistoryReplayPrepareRequest, ModelHistoryReplayValidationRequest,
    ModelMissingMeshRepairRequest, ModelPhysicalBaselineSnapshotRequest,
    ModelReleaseRegisterRequest, ModelSceneTreeArtifactRestoreRequest,
    ModelSourceObservationResponse, ModelVersionDuckLakeConfig, legacy_batch_id_for_sesno,
    parse_legacy_batch_id,
};
use crate::version_management::types::{ModelReleaseQuality, ModelReleaseStatus};
use anyhow::Context;
use clap::{Arg, ArgMatches, Command};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn model_version_command() -> Command {
    Command::new("model-version")
        .about("Register and query immutable model releases backed by DuckLake")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("register")
                .about("Register a generated Parquet package as a staged immutable model release")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .value_name("ID")
                        .help("DEPRECATED alias for package folder / legacy catalog row. Prefer omit and pass --sesno (auto db{dbnum}-s{sesno})"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N")
                        .help("Export sesno for unit_versions_v2 sync (required when --release-id omitted)"),
                )
                .arg(
                    Arg::new("release-label")
                        .long("release-label")
                        .value_name("LABEL")
                        .help("Human-readable label for the release"),
                )
                .arg(
                    Arg::new("release-quality")
                        .long("release-quality")
                        .value_name("QUALITY")
                        .help("Explicit release quality: complete_visual, quarantined_visual, degraded_visual, patch_only, non_visual"),
                )
                .arg(
                    Arg::new("release-quality-reason")
                        .long("release-quality-reason")
                        .value_name("TEXT")
                        .help("Human-readable reason for the chosen release quality"),
                )
                .arg(
                    Arg::new("validation-flag")
                        .long("validation-flag")
                        .value_name("FLAG")
                        .action(clap::ArgAction::Append)
                        .help("Validation flag to persist on the release; repeatable"),
                )
                .arg(
                    Arg::new("spec-info-fallback-count")
                        .long("spec-info-fallback-count")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("N")
                        .help("Number of components whose spec_info fell back to an unknown/default value"),
                )
                .arg(
                    Arg::new("parent-release-id")
                        .long("parent-release-id")
                        .value_name("ID")
                        .help("Optional parent release id for the release graph"),
                )
                .arg(
                    Arg::new("branch-id")
                        .long("branch-id")
                        .default_value("main")
                        .value_name("BRANCH")
                        .help("Logical release branch id"),
                )
                .arg(
                    Arg::new("derivation-type")
                        .long("derivation-type")
                        .default_value("incremental-sesno")
                        .value_name("TYPE")
                        .help("How the package was produced, e.g. incremental-sesno or manual-import"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum represented by this package"),
                )
                .arg(
                    Arg::new("parquet-dir")
                        .long("parquet-dir")
                        .value_name("DIR")
                        .help("Source package directory; defaults to output/<project>/parquet/<dbnum>"),
                )
                .arg(
                    Arg::new("release-root")
                        .long("release-root")
                        .value_name("DIR")
                        .help("Immutable release root; defaults to output/<project>/model_versions/releases"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("metadata-json")
                        .long("metadata-json")
                        .value_name("JSON")
                        .help("Optional extra metadata JSON object stored with the release"),
                )
                .arg(
                    Arg::new("index-units")
                        .long("index-units")
                        .help("Rebuild delivery-unit membership and unit aggregate index after registration")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("publish-history")
                .about("Publish an isolated historical Parquet package as an immutable model release")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .value_name("ID")
                        .help("DEPRECATED legacy catalog alias. Prefer omit; defaults to db{dbnum}-s{to-sesno}"),
                )
                .arg(
                    Arg::new("release-label")
                        .long("release-label")
                        .value_name("LABEL")
                        .help("Human-readable label for the release"),
                )
                .arg(
                    Arg::new("release-quality")
                        .long("release-quality")
                        .value_name("QUALITY")
                        .help("Explicit release quality: complete_visual, quarantined_visual, degraded_visual, patch_only, non_visual"),
                )
                .arg(
                    Arg::new("release-quality-reason")
                        .long("release-quality-reason")
                        .value_name("TEXT")
                        .help("Human-readable reason for the chosen release quality"),
                )
                .arg(
                    Arg::new("validation-flag")
                        .long("validation-flag")
                        .value_name("FLAG")
                        .action(clap::ArgAction::Append)
                        .help("Validation flag to persist on the release; repeatable"),
                )
                .arg(
                    Arg::new("spec-info-fallback-count")
                        .long("spec-info-fallback-count")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("N")
                        .help("Number of components whose spec_info fell back to an unknown/default value"),
                )
                .arg(
                    Arg::new("parent-release-id")
                        .long("parent-release-id")
                        .value_name("ID")
                        .help("Optional parent release id for the release graph"),
                )
                .arg(
                    Arg::new("branch-id")
                        .long("branch-id")
                        .default_value("main")
                        .value_name("BRANCH")
                        .help("Logical release branch id"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum represented by this historical package"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .required(true)
                        .value_name("FILE")
                        .help("E3D/PDMS source DB file used to derive this historical release"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical increment lower bound"),
                )
                .arg(
                    Arg::new("to-sesno")
                        .long("to-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical increment upper bound"),
                )
                .arg(
                    Arg::new("parquet-dir")
                        .long("parquet-dir")
                        .required(true)
                        .value_name("DIR")
                        .help("Isolated/staged Parquet package directory; current output/<project>/parquet/<dbnum> is rejected"),
                )
                .arg(
                    Arg::new("current-parquet-dir")
                        .long("current-parquet-dir")
                        .value_name("DIR")
                        .help("Current-state Parquet directory to reject; defaults to output/<project>/parquet/<dbnum>"),
                )
                .arg(
                    Arg::new("scene-tree-dir")
                        .long("scene-tree-dir")
                        .value_name("DIR")
                        .help("Optional scene_tree directory to validate for this replay workspace; defaults to inference from --parquet-dir when possible"),
                )
                .arg(
                    Arg::new("require-scene-tree")
                        .long("require-scene-tree")
                        .help("Fail publish-history unless scene_tree/<dbnum>.tree and db_meta_info.json exist")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("release-root")
                        .long("release-root")
                        .value_name("DIR")
                        .help("Immutable release root; defaults to output/<project>/model_versions/releases"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("metadata-json")
                        .long("metadata-json")
                        .value_name("JSON")
                        .help("Optional extra metadata JSON object nested under history_publish.user_metadata"),
                )
                .arg(
                    Arg::new("mesh-root")
                        .long("mesh-root")
                        .value_name("DIR")
                        .help("Mesh root for --materialize-assets; defaults to DbOption meshes_path"),
                )
                .arg(
                    Arg::new("mesh-base-url")
                        .long("mesh-base-url")
                        .value_name("URL")
                        .help("Base URL used when writing mesh asset URLs"),
                )
                .arg(
                    Arg::new("materialize-assets")
                        .long("materialize-assets")
                        .help("Copy mesh GLB files into the immutable release package and index assets")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("index-units")
                        .long("index-units")
                        .help("Rebuild delivery-unit membership and unit aggregate index after publishing")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("annotate")
                .about("Annotate an existing model release without mutating its immutable package")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Release id to annotate"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("release-quality")
                        .long("release-quality")
                        .value_name("QUALITY")
                        .help("Optional updated release quality: complete_visual, quarantined_visual, degraded_visual, patch_only, non_visual"),
                )
                .arg(
                    Arg::new("release-quality-reason")
                        .long("release-quality-reason")
                        .value_name("TEXT")
                        .help("Human-readable release quality note"),
                )
                .arg(
                    Arg::new("validation-flag")
                        .long("validation-flag")
                        .value_name("FLAG")
                        .action(clap::ArgAction::Append)
                        .help("Validation flag to append to the release; repeatable"),
                )
                .arg(
                    Arg::new("spec-info-fallback-count")
                        .long("spec-info-fallback-count")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("N")
                        .help("Number of components whose spec_info fell back to an unknown/default value"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("audit-spec-info")
                .about("Read-only audit of spec_value=0 evidence in a release package")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Release id to audit"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("migrate")
                .about("Apply DuckLake model-version catalog migrations and print a schema report")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("observe-source")
                .about("Build a read-only source DB observation manifest for a stable E3D/PDMS file")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum to observe"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .value_name("FILE")
                        .help("Explicit source DB file; if omitted, sqlite db_index is used when available"),
                )
                .arg(
                    Arg::new("dependency-file")
                        .long("dependency-file")
                        .value_name("FILE")
                        .action(clap::ArgAction::Append)
                        .help("Dependency DB file to include in the observation evidence; repeatable"),
                )
                .arg(
                    Arg::new("observation-id")
                        .long("observation-id")
                        .value_name("ID")
                        .help("Path-safe observation id; defaults to source-db<dbnum>-<timestamp>"),
                )
                .arg(
                    Arg::new("manifest-out")
                        .long("manifest-out")
                        .value_name("FILE")
                        .help("Output manifest path; defaults under output/<project>/model_versions/source_observations"),
                )
                .arg(
                    Arg::new("requested-sesno")
                        .long("requested-sesno")
                        .value_name("SESNO")
                        .help("Optional user-requested sesno/range label to record in the observation manifest"),
                )
                .arg(
                    Arg::new("resolved-sesno")
                        .long("resolved-sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Optional resolved/latest sesno override; otherwise read from the DB file"),
                )
                .arg(
                    Arg::new("quiescence-window-ms")
                        .long("quiescence-window-ms")
                        .default_value("1000")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("MS")
                        .help("Quiet-window delay between two source file hash/size checks"),
                )
                .arg(
                    Arg::new("rescan-index")
                        .long("rescan-index")
                        .help("When resolving by dbnum, refresh db_index.sqlite before lookup")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("require-stable")
                        .long("require-stable")
                        .help("Exit with an error if the source file changes during the observation window")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("Overwrite an existing observation manifest path")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("release-events")
                .about("List lifecycle/status events for an export-batch release (not unit version identity)")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .value_name("ID")
                        .help("Export-batch id; optional if --dbnum --sesno provided (db{N}-s{M})"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N")
                        .help("With --sesno, resolve batch id as db{N}-s{M}"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N")
                        .help("With --dbnum, resolve batch id as db{N}-s{M}"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("reconcile-release")
                .about("Reconcile export-batch release lifecycle (specs/023: not unit version identity; prefer unit-v2-set-status for units)")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .value_name("ID")
                        .help("Export-batch id; optional if --dbnum --sesno provided"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("publish-if-complete")
                        .long("publish-if-complete")
                        .help("If reconcile evidence is complete, mark a non-published release as published")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("fail-if-unusable")
                        .long("fail-if-unusable")
                        .help("If reconcile evidence has blocking problems, mark a non-failed release as failed")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("prepare-history-replay")
                .about("Write isolated baseline/replay DbOptions and print the historical replay command plan")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Stable user-facing release id, e.g. ams-1112-sesno-897"),
                )
                .arg(
                    Arg::new("release-label")
                        .long("release-label")
                        .value_name("LABEL")
                        .help("Human-readable label for the release"),
                )
                .arg(
                    Arg::new("baseline-release-id")
                        .long("baseline-release-id")
                        .value_name("ID")
                        .help("Release id for the generated from_sesno baseline; defaults to <release-id>-baseline-<from-sesno>"),
                )
                .arg(
                    Arg::new("parent-release-id")
                        .long("parent-release-id")
                        .value_name("ID")
                        .help("Optional parent release id for the baseline release graph node"),
                )
                .arg(
                    Arg::new("branch-id")
                        .long("branch-id")
                        .default_value("main")
                        .value_name("BRANCH")
                        .help("Logical release branch id"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum represented by this historical package"),
                )
                .arg(
                    Arg::new("baseline-dbnum")
                        .long("baseline-dbnum")
                        .value_name("DBNUM")
                        .value_delimiter(',')
                        .value_parser(clap::value_parser!(u32))
                        .num_args(1..)
                        .help("DB numbers to hydrate for the isolated baseline, including catalogue/dependency DBs; target dbnum is added automatically"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .required(true)
                        .value_name("FILE")
                        .help("E3D/PDMS source DB file used for replay generation"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical increment lower bound"),
                )
                .arg(
                    Arg::new("to-sesno")
                        .long("to-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical increment upper bound"),
                )
                .arg(
                    Arg::new("base-config")
                        .long("base-config")
                        .value_name("CONFIG")
                        .help("Base DbOption path without .toml; defaults to DB_OPTION_FILE"),
                )
                .arg(
                    Arg::new("replay-config-out")
                        .long("replay-config-out")
                        .value_name("CONFIG")
                        .help("Replay DbOption output path without .toml; defaults under output/<project>/model_versions/replay_configs"),
                )
                .arg(
                    Arg::new("baseline-config-out")
                        .long("baseline-config-out")
                        .value_name("CONFIG")
                        .help("Baseline parse DbOption output path without .toml; defaults to <replay-config-out>-baseline"),
                )
                .arg(
                    Arg::new("replay-output-root")
                        .long("replay-output-root")
                        .value_name("DIR")
                        .help("Isolated output_root for replay generation"),
                )
                .arg(
                    Arg::new("replay-surreal-ns")
                        .long("replay-surreal-ns")
                        .value_name("NS")
                        .help("Isolated SurrealDB namespace; defaults to <current>_history_<release-id>"),
                )
                .arg(
                    Arg::new("current-parquet-dir")
                        .long("current-parquet-dir")
                        .value_name("DIR")
                        .help("Current-state Parquet directory to reject; defaults to output/<project>/parquet/<dbnum>"),
                )
                .arg(
                    Arg::new("baseline-source-confirmed-at-from-sesno")
                        .long("baseline-source-confirmed-at-from-sesno")
                        .help("Confirm source-db-file is already an isolated physical baseline for from_sesno; required because prepare-history-replay does not hydrate target sesno from pdms-io history")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("Overwrite an existing replay config after safety checks")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("prepare-physical-baseline-snapshot")
                .about("Create an isolated project snapshot that replaces one DB file with a physical historical baseline")
                .arg(
                    Arg::new("snapshot-id")
                        .long("snapshot-id")
                        .required(true)
                        .value_name("ID")
                        .help("Path-safe snapshot id, e.g. ams-1112-physical-791"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum to replace inside the snapshot"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .required(true)
                        .value_name("FILE")
                        .help("Physical historical DB file whose header dbnum must match --dbnum"),
                )
                .arg(
                    Arg::new("baseline-dbnum")
                        .long("baseline-dbnum")
                        .value_name("DBNUM")
                        .value_delimiter(',')
                        .value_parser(clap::value_parser!(u32))
                        .num_args(1..)
                        .help("DB numbers to parse for the baseline snapshot; target dbnum is added automatically"),
                )
                .arg(
                    Arg::new("base-config")
                        .long("base-config")
                        .value_name("CONFIG")
                        .help("Base DbOption path without .toml; defaults to DB_OPTION_FILE"),
                )
                .arg(
                    Arg::new("config-out")
                        .long("config-out")
                        .value_name("CONFIG")
                        .help("Output DbOption path without .toml; defaults under output/<project>/model_versions/physical_baselines/<snapshot-id>"),
                )
                .arg(
                    Arg::new("snapshot-root")
                        .long("snapshot-root")
                        .value_name("DIR")
                        .help("Snapshot working root; defaults under output/<project>/model_versions/physical_baselines/<snapshot-id>"),
                )
                .arg(
                    Arg::new("output-root")
                        .long("output-root")
                        .value_name("DIR")
                        .help("Isolated output_root for the snapshot config; defaults to <snapshot-root>/output"),
                )
                .arg(
                    Arg::new("surreal-ns")
                        .long("surreal-ns")
                        .value_name("NS")
                        .help("Isolated SurrealDB namespace for baseline parse; defaults to <current>_baseline_<sanitized-snapshot-id>"),
                )
                .arg(
                    Arg::new("copy-files")
                        .long("copy-files")
                        .help("Copy files instead of hard-linking with copy fallback")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("Overwrite files inside the snapshot directory and config path")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("validate-baseline-state")
                .about("Validate a physical baseline state manifest before publishing a historical model release")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum expected in the baseline state manifest"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Optional historical lower-bound sesno that must match the manifest latest sesno"),
                )
                .arg(
                    Arg::new("baseline-state-manifest")
                        .long("baseline-state-manifest")
                        .required(true)
                        .value_name("FILE")
                        .help("Path to baseline_state_manifest.json produced by prepare-physical-baseline-snapshot"),
                )
                .arg(
                    Arg::new("baseline-state-manifest-hash")
                        .long("baseline-state-manifest-hash")
                        .value_name("SHA256")
                        .help("Expected manifest sha256; if omitted, the command computes and returns it"),
                )
                .arg(
                    Arg::new("scene-tree-dir")
                        .long("scene-tree-dir")
                        .value_name("DIR")
                        .help("Optional scene_tree directory to validate; defaults to <baseline output_root>/<project>/scene_tree"),
                )
                .arg(
                    Arg::new("require-scene-tree")
                        .long("require-scene-tree")
                        .help("Fail unless scene_tree/<dbnum>.tree and db_meta_info.json are present for the baseline workspace")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("run-command")
                .about("Run an argv-array command under a durable bounded supervisor")
                .arg(
                    Arg::new("run-id")
                        .long("run-id")
                        .required(true)
                        .value_name("ID")
                        .help("Path-safe run id used under the runner state directory"),
                )
                .arg(
                    Arg::new("kind")
                        .long("kind")
                        .default_value("generic")
                        .value_name("KIND")
                        .help("Run kind, e.g. parse, generate_full_model, publish, smoke"),
                )
                .arg(
                    Arg::new("state-dir")
                        .long("state-dir")
                        .value_name("DIR")
                        .help("Runner state root; defaults to output/<project>/model_versions/runs"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name used for the default runner state root"),
                )
                .arg(
                    Arg::new("executable")
                        .long("executable")
                        .value_name("EXE")
                        .help("Executable to spawn; defaults to the current aios-database executable"),
                )
                .arg(
                    Arg::new("argv-json")
                        .long("argv-json")
                        .value_name("JSON")
                        .help("Command argv as a JSON string array; a leading executable name is accepted and stripped when it matches --executable"),
                )
                .arg(
                    Arg::new("argv-file")
                        .long("argv-file")
                        .value_name("FILE")
                        .help("File containing a JSON string array argv; prepared command-plan arrays may include the leading executable"),
                )
                .arg(
                    Arg::new("cwd")
                        .long("cwd")
                        .value_name("DIR")
                        .help("Working directory for the child; defaults to current working directory"),
                )
                .arg(
                    Arg::new("stdout-path")
                        .long("stdout-path")
                        .value_name("FILE")
                        .help("Child stdout log path; defaults to <state-dir>/<run-id>/stdout.log"),
                )
                .arg(
                    Arg::new("stderr-path")
                        .long("stderr-path")
                        .value_name("FILE")
                        .help("Child stderr log path; defaults to <state-dir>/<run-id>/stderr.log"),
                )
                .arg(
                    Arg::new("metrics-path")
                        .long("metrics-path")
                        .value_name("FILE")
                        .help("Optional task metrics JSON file to snapshot while the command runs"),
                )
                .arg(
                    Arg::new("timeout-secs")
                        .long("timeout-secs")
                        .default_value("14400")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("SECONDS")
                        .help("Hard timeout for the supervised command"),
                )
                .arg(
                    Arg::new("stale-heartbeat-secs")
                        .long("stale-heartbeat-secs")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("SECONDS")
                        .help("Kill the command if the metrics file mtime is stale for this long"),
                )
                .arg(
                    Arg::new("poll-interval-ms")
                        .long("poll-interval-ms")
                        .default_value("1000")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("MS")
                        .help("Status update/poll interval"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .value_name("FILE")
                        .help("Optional source DB file to hash before and after the run"),
                )
                .arg(
                    Arg::new("source-db-sha256")
                        .long("source-db-sha256")
                        .value_name("SHA256")
                        .help("Expected pre-run source DB hash"),
                )
                .arg(
                    Arg::new("env")
                        .long("env")
                        .value_name("KEY=VALUE")
                        .action(clap::ArgAction::Append)
                        .help("Environment variable to set on the child; repeatable"),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("Overwrite any existing run directory for this run id")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("run-status")
                .about("Read a bounded runner status JSON file")
                .arg(
                    Arg::new("run-id")
                        .long("run-id")
                        .required(true)
                        .value_name("ID")
                        .help("Run id to read"),
                )
                .arg(
                    Arg::new("state-dir")
                        .long("state-dir")
                        .value_name("DIR")
                        .help("Runner state root; defaults to output/<project>/model_versions/runs"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name used for the default runner state root"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("cancel-run")
                .about("Request cancellation for a bounded runner command")
                .arg(
                    Arg::new("run-id")
                        .long("run-id")
                        .required(true)
                        .value_name("ID")
                        .help("Run id to cancel"),
                )
                .arg(
                    Arg::new("state-dir")
                        .long("state-dir")
                        .value_name("DIR")
                        .help("Runner state root; defaults to output/<project>/model_versions/runs"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name used for the default runner state root"),
                )
                .arg(
                    Arg::new("reason")
                        .long("reason")
                        .value_name("TEXT")
                        .help("Human-readable cancellation reason"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("inspect-history-baseline")
                .about("Inspect visible E3D elements at a target session without mutating state")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .required(true)
                        .value_name("FILE")
                        .help("E3D/PDMS source DB file to inspect"),
                )
                .arg(
                    Arg::new("target-sesno")
                        .long("target-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical session to inspect as a baseline state"),
                )
                .arg(
                    Arg::new("parse-sample-limit")
                        .long("parse-sample-limit")
                        .default_value("100")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N")
                        .help("Number of visible elements to parse as a sample; use 0 to skip parsing"),
                )
                .arg(
                    Arg::new("allow-nearest-sesno")
                        .long("allow-nearest-sesno")
                        .help("If target session is absent, inspect the nearest lower session instead of failing")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("detail")
                        .long("detail")
                        .help("Enable detailed pdms-io diagnostics")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("validate-history-replay")
                .about("Validate a staged historical replay Parquet package before publishing")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum represented by this historical replay package"),
                )
                .arg(
                    Arg::new("source-db-file")
                        .long("source-db-file")
                        .required(true)
                        .value_name("FILE")
                        .help("E3D/PDMS source DB file used for replay generation"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical increment lower bound"),
                )
                .arg(
                    Arg::new("to-sesno")
                        .long("to-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("SESNO")
                        .help("Historical increment upper bound"),
                )
                .arg(
                    Arg::new("parquet-dir")
                        .long("parquet-dir")
                        .required(true)
                        .value_name("DIR")
                        .help("Isolated/staged replay Parquet package directory"),
                )
                .arg(
                    Arg::new("current-parquet-dir")
                        .long("current-parquet-dir")
                        .value_name("DIR")
                        .help("Current-state Parquet directory to reject; defaults to output/<project>/parquet/<dbnum>"),
                )
                .arg(
                    Arg::new("scene-tree-dir")
                        .long("scene-tree-dir")
                        .value_name("DIR")
                        .help("Optional scene_tree directory to report or require"),
                )
                .arg(
                    Arg::new("require-scene-tree")
                        .long("require-scene-tree")
                        .help("Fail unless scene_tree/<dbnum>.tree and db_meta_info.json are present")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("allow-patch-only")
                        .long("allow-patch-only")
                        .help("Return exit code 0 for patch-only diagnostics while still reporting not publishable")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("repair-missing-meshes")
                .about("Regenerate GLB meshes referenced by a missing_mesh_report JSON file")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum represented by the missing mesh report"),
                )
                .arg(
                    Arg::new("report-file")
                        .long("report-file")
                        .required(true)
                        .value_name("FILE")
                        .help("Path to missing_mesh_report_<dbnum>.json emitted by Parquet export"),
                )
                .arg(
                    Arg::new("mesh-root")
                        .long("mesh-root")
                        .value_name("DIR")
                        .help("Mesh root; defaults to DbOption meshes_path"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N")
                        .help("Repair only the first N unique hashes from the report"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Inspect eligible missing meshes without generating files")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("retry-bad")
                        .long("retry-bad")
                        .help("Retry inst_geo rows that are already marked bad")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("restore-scene-tree-artifact")
                .about("Restore one dbnum scene_tree artifact from a validated baseline/replay workspace")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("PDMS/E3D dbnum whose <dbnum>.tree should be restored"),
                )
                .arg(
                    Arg::new("source-scene-tree-dir")
                        .long("source-scene-tree-dir")
                        .required(true)
                        .value_name("DIR")
                        .help("Source scene_tree directory containing <dbnum>.tree and db_meta_info.json"),
                )
                .arg(
                    Arg::new("target-scene-tree-dir")
                        .long("target-scene-tree-dir")
                        .value_name("DIR")
                        .help("Target scene_tree directory; defaults to output/<project>/scene_tree"),
                )
                .arg(
                    Arg::new("overwrite-tree")
                        .long("overwrite-tree")
                        .help("Replace an existing target <dbnum>.tree if its hash differs")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Validate and report the restore plan without writing files")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("list")
                .about("List registered model releases")
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name filter; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("all-projects")
                        .long("all-projects")
                        .help("Do not filter by project")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("index")
                .about("Rebuild the component snapshot index for a model release")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Release id to index"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("index-units")
                .about("DEPRECATED (specs/023): rebuild unit versions keyed by release_id; prefer unit-v2-* / write_unit_version_with_members_v2")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("DEPRECATED release id; if db{N}-s{M}, sesno identity is preferred after sync"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-v2-smoke")
                .about("specs/023 B5: smoke-test unit_versions_v2 upsert (max member sesno, idempotent, hash conflict)")
                .arg(
                    Arg::new("work-dir")
                        .long("work-dir")
                        .value_name("DIR")
                        .help("Working directory for a temporary DuckLake catalog; defaults under std::env::temp_dir()"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-v2-get")
                .alias("unit-get")
                .about("specs/023: get one unit version by dbnum+refno+sesno")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .required(true)
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64")
                        .help("unit_refno_u64"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-v2-list")
                .alias("unit-list")
                .about("specs/023: list unit versions for one refno ordered by sesno desc")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .required(true)
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-v2-diff")
                .about("specs/023: diff unit versions between two sesnos (optional single --refno); also via unit-diff --dbnum --from-sesno --to-sesno")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("to-sesno")
                        .long("to-sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64")
                        .help("Optional: only diff this unit_refno_u64"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .default_value("200")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-v2-set-status")
                .about("specs/023 E2: set unit_versions_v2.status and append unit_version_status_events_v2")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .required(true)
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("status")
                        .long("status")
                        .required(true)
                        .value_name("STATUS")
                        .help("e.g. indexed, published, quarantined"),
                )
                .arg(
                    Arg::new("reason")
                        .long("reason")
                        .value_name("TEXT"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-v2-events")
                .about("specs/023 E2: list unit version status events by dbnum+refno+sesno")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .required(true)
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64"),
                )
                .arg(
                    Arg::new("sesno")
                        .long("sesno")
                        .required(true)
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("index-assets")
                .about("Rebuild the release mesh asset index from geo_instances.parquet")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Release id to index"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("mesh-root")
                        .long("mesh-root")
                        .value_name("DIR")
                        .help("Mesh root; defaults to DbOption meshes_path"),
                )
                .arg(
                    Arg::new("mesh-base-url")
                        .long("mesh-base-url")
                        .value_name("URL")
                        .help("Base URL used when writing mesh asset URLs; defaults to /files/meshes, or the release URL when --materialize is set"),
                )
                .arg(
                    Arg::new("materialize")
                        .long("materialize")
                        .help("Copy mesh GLB files into the immutable release package before indexing")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("mesh-assets")
                .about("List indexed release mesh assets")
                .arg(
                    Arg::new("release-id")
                        .long("release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Release id to read"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("missing-only")
                        .long("missing-only")
                        .help("Only list missing non-builtin mesh assets")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .default_value("200")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N")
                        .help("Maximum number of assets to emit"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("validate-compare-readiness")
                .about("Validate whether two model releases are production-ready for visual comparison")
                .arg(
                    Arg::new("from-release-id")
                        .long("from-release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Baseline release id"),
                )
                .arg(
                    Arg::new("to-release-id")
                        .long("to-release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Target release id"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("diff")
                .about("Diff component snapshots between two model releases")
                .arg(
                    Arg::new("from-release-id")
                        .long("from-release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Baseline release id"),
                )
                .arg(
                    Arg::new("to-release-id")
                        .long("to-release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Target release id"),
                )
                .arg(
                    Arg::new("change-type")
                        .long("change-type")
                        .value_parser(["added", "deleted", "changed"])
                        .value_name("TYPE")
                        .help("Optional change type filter"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .default_value("200")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N")
                        .help("Maximum number of diff rows to emit"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("unit-diff")
                .about("specs/023: diff delivery units by sesno (preferred) or legacy release_id")
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N")
                        .help("Required with --from-sesno/--to-sesno (specs/023 preferred path)"),
                )
                .arg(
                    Arg::new("from-sesno")
                        .long("from-sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N")
                        .help("Baseline sesno (preferred; pairs with --to-sesno --dbnum)"),
                )
                .arg(
                    Arg::new("to-sesno")
                        .long("to-sesno")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("N")
                        .help("Target sesno (preferred; pairs with --from-sesno --dbnum)"),
                )
                .arg(
                    Arg::new("refno")
                        .long("refno")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64")
                        .help("Optional: only diff this unit_refno_u64 (sesno mode)"),
                )
                .arg(
                    Arg::new("from-release-id")
                        .long("from-release-id")
                        .value_name("ID")
                        .help("DEPRECATED: baseline release id; prefer --from-sesno. db{N}-s{M} maps to v2"),
                )
                .arg(
                    Arg::new("to-release-id")
                        .long("to-release-id")
                        .value_name("ID")
                        .help("DEPRECATED: target release id; prefer --to-sesno. db{N}-s{M} maps to v2"),
                )
                .arg(
                    Arg::new("unit-noun")
                        .long("unit-noun")
                        .value_name("NOUN")
                        .help("Legacy release_id mode only: BRAN, HANG, EQUI/EQUIP, WALL, FLOOR, UNASSIGNED"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .default_value("200")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N")
                        .help("Maximum number of unit diff rows to emit"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("impact")
                .about("Explain which delivery units are impacted by component changes between releases")
                .arg(
                    Arg::new("from-release-id")
                        .long("from-release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Baseline release id"),
                )
                .arg(
                    Arg::new("to-release-id")
                        .long("to-release-id")
                        .required(true)
                        .value_name("ID")
                        .help("Target release id"),
                )
                .arg(
                    Arg::new("component-key")
                        .long("component-key")
                        .value_name("KEY")
                        .help("Optional component key filter, e.g. 1112:75144748307309"),
                )
                .arg(
                    Arg::new("refno-u64")
                        .long("refno-u64")
                        .value_parser(clap::value_parser!(u64))
                        .value_name("U64")
                        .help("Optional refno filter; converted to <dbnum>:<refno_u64>"),
                )
                .arg(
                    Arg::new("dbnum")
                        .long("dbnum")
                        .value_parser(clap::value_parser!(u32))
                        .value_name("DBNUM")
                        .help("Dbnum used with --refno-u64"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .default_value("200")
                        .value_parser(clap::value_parser!(usize))
                        .value_name("N")
                        .help("Maximum number of impact rows to emit"),
                )
                .arg(
                    Arg::new("project")
                        .long("project")
                        .value_name("PROJECT")
                        .help("Project name override; defaults to DbOption project_name"),
                )
                .arg(
                    Arg::new("ducklake-metadata")
                        .long("ducklake-metadata")
                        .value_name("FILE")
                        .help("DuckLake metadata path; defaults to output/<project>/model_versions/metadata.ducklake"),
                )
                .arg(
                    Arg::new("ducklake-data")
                        .long("ducklake-data")
                        .value_name("DIR")
                        .help("DuckLake data path; defaults to output/<project>/model_versions/data"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Print pretty JSON")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("history")
                .about("specs/022: PE/ATT time-travel by sesno (SurrealDB versioned)")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("snapshot")
                        .about("Fetch PE(+ATT) snapshot at a sesno anchor")
                        .arg(
                            Arg::new("refno")
                                .long("refno")
                                .value_name("REFNO")
                                .required_unless_present("pe-key")
                                .help("Element refno (u64 or a/b form)"),
                        )
                        .arg(
                            Arg::new("pe-key")
                                .long("pe-key")
                                .value_name("KEY")
                                .help("Override PE record id (e.g. pe:equi_001 for fixtures)"),
                        )
                        .arg(
                            Arg::new("sesno")
                                .long("sesno")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("dbnum")
                                .long("dbnum")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("json")
                                .long("json")
                                .action(clap::ArgAction::SetTrue),
                        ),
                )
                .subcommand(
                    Command::new("timeline")
                        .about("List content-changing anchors for one element in a sesno range")
                        .arg(
                            Arg::new("refno")
                                .long("refno")
                                .value_name("REFNO")
                                .required_unless_present("pe-key"),
                        )
                        .arg(
                            Arg::new("pe-key")
                                .long("pe-key")
                                .value_name("KEY")
                                .help("Override PE record id for fixtures"),
                        )
                        .arg(
                            Arg::new("from-sesno")
                                .long("from-sesno")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("to-sesno")
                                .long("to-sesno")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("dbnum")
                                .long("dbnum")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("json")
                                .long("json")
                                .action(clap::ArgAction::SetTrue),
                        ),
                )
                .subcommand(
                    Command::new("diff")
                        .about("Field-level PE/ATT diff for refnos between two sesnos")
                        .arg(
                            Arg::new("refnos")
                                .long("refnos")
                                .value_name("CSV")
                                .required_unless_present("pe-key")
                                .help("Comma-separated refnos"),
                        )
                        .arg(
                            Arg::new("pe-key")
                                .long("pe-key")
                                .value_name("KEY")
                                .help("Single-element fixture PE key (implies one synthetic refno)"),
                        )
                        .arg(
                            Arg::new("from-sesno")
                                .long("from-sesno")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("to-sesno")
                                .long("to-sesno")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("dbnum")
                                .long("dbnum")
                                .value_parser(clap::value_parser!(u32))
                                .required(true),
                        )
                        .arg(
                            Arg::new("json")
                                .long("json")
                                .action(clap::ArgAction::SetTrue),
                        ),
                ),
        )
}

pub async fn handle_model_version_command(
    matches: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<bool> {
    let Some(model_matches) = matches.subcommand_matches("model-version") else {
        return Ok(false);
    };

    match model_matches.subcommand() {
        Some(("register", sub)) => {
            let request = build_register_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = register_model_release(request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "registered staged release {} registration_status={:?} release_status={} package={}",
                    response.release.release_id,
                    response.status,
                    response.release.release_status.as_str(),
                    response.release.immutable_package_dir.display()
                );
                if let Some(unit_index) = &response.unit_index {
                    println!(
                        "unit_index units={} members={} unresolved={}",
                        unit_index.unit_count,
                        unit_index.member_count,
                        unit_index.unresolved_member_count
                    );
                }
            }
        }
        Some(("publish-history", sub)) => {
            let request = build_publish_history_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = publish_history_model_release(request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "published historical release {} status={:?} package={} replay_mode={} generation_by_command={}",
                    response.release.release.release_id,
                    response.release.status,
                    response.release.release.immutable_package_dir.display(),
                    response.safety_checks.replay_mode,
                    response.safety_checks.generation_performed_by_command
                );
                if let Some(asset_index) = &response.mesh_asset_index {
                    println!(
                        "mesh_assets present={} missing={} manifest={}",
                        asset_index.present_count,
                        asset_index.missing_count,
                        asset_index.manifest_path.display()
                    );
                }
                if let Some(scene_tree) = &response.safety_checks.scene_tree {
                    println!(
                        "scene_tree ready={} required={} tree={} db_meta={}",
                        scene_tree.tree_file_exists && scene_tree.db_meta_info_exists,
                        scene_tree.required,
                        scene_tree.tree_file.display(),
                        scene_tree.db_meta_info_file.display()
                    );
                }
                if let Some(unit_index) = &response.unit_index {
                    println!(
                        "unit_index units={} members={} unresolved={}",
                        unit_index.unit_count,
                        unit_index.member_count,
                        unit_index.unresolved_member_count
                    );
                }
            }
        }
        Some(("annotate", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = sub
                .get_one::<String>("release-id")
                .expect("required by clap");
            let release_quality = release_quality_from_matches(sub)?;
            let release_quality_reason = sub
                .get_one::<String>("release-quality-reason")
                .map(String::as_str);
            let validation_flags = validation_flags_from_matches(sub);
            let spec_info_fallback_count = sub.get_one::<u64>("spec-info-fallback-count").copied();
            let release = annotate_model_release(
                ducklake,
                release_id,
                release_quality,
                release_quality_reason,
                &validation_flags,
                spec_info_fallback_count,
            )?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&release)?);
            } else {
                println!(
                    "annotated release {} quality={} flags={}",
                    release.release_id,
                    release.release_quality.as_str(),
                    release.validation_flags.join(",")
                );
            }
        }
        Some(("audit-spec-info", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = sub
                .get_one::<String>("release-id")
                .expect("required by clap");
            let response = audit_release_spec_info(ducklake, release_id)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "audited release {} legacy_zero_spec_value_count={} manifest_count={:?} package={}",
                    response.release_id,
                    response.legacy_zero_spec_value_count,
                    response.manifest_spec_info_fallback_count,
                    response.package_dir.display()
                );
                println!("recommended_action={}", response.recommended_action);
            }
        }
        Some(("migrate", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let report = migrate_model_version_catalog(&project_name, ducklake)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "migrated DuckLake catalog project={} schema={} releases={} migrations={} metadata={} data={}",
                    report.project_name,
                    report.schema_name,
                    report.release_count,
                    report.schema_migration_count,
                    report.ducklake_metadata_path.display(),
                    report.ducklake_data_path.display()
                );
                println!(
                    "release_quality_columns_present={} missing_schema_migrations={} missing_tables={} missing_release_columns={}",
                    report.release_quality_columns_present,
                    report.missing_schema_migrations.join(","),
                    report.missing_tables.join(","),
                    report.missing_release_columns.join(",")
                );
                println!(
                    "required_schema_migrations={}",
                    report.required_schema_migrations.join(",")
                );
                println!(
                    "applied_schema_migrations={}",
                    report.applied_schema_migrations.join(",")
                );
            }
        }
        Some(("observe-source", sub)) => {
            let response = build_source_observation_response(sub, db_option_ext).await?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "observed source project={} dbnum={} stable={} latest_sesno={:?} manifest={} sha256={}",
                    response.project_name,
                    response.dbnum,
                    response.observation.quiescence.stable,
                    response.resolved_sesno,
                    response.observation_manifest_path.display(),
                    response.observation_manifest_hash
                );
                println!("status: {}", response.status);
                println!("recommended_action: {}", response.recommended_action);
            }
            if sub.get_flag("require-stable") && !response.ready_for_increment {
                anyhow::bail!(
                    "source observation is not stable/readable enough for increment: {}",
                    response.recommended_action
                );
            }
        }
        Some(("release-events", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = resolve_export_batch_id(sub)?;
            let response = get_model_release_events(ducklake, &release_id)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "release {} lifecycle={} status={} events={}",
                    response.release.release_id,
                    response.release.release_lifecycle.as_str(),
                    response.release.release_status.as_str(),
                    response.events.len()
                );
                for event in response.events {
                    println!(
                        "{} status={} reason={}",
                        event.created_at,
                        event.release_status.as_str(),
                        event.reason.unwrap_or_default()
                    );
                }
            }
        }
        Some(("reconcile-release", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = resolve_export_batch_id(sub)?;
            eprintln!(
                "info: reconcile-release operates on export-batch identity '{release_id}' (specs/023); unit status uses unit-v2-set-status"
            );
            let report = reconcile_model_release(
                ducklake,
                &release_id,
                sub.get_flag("publish-if-complete"),
                sub.get_flag("fail-if-unusable"),
            )?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "reconciled release {} previous_status={} current_status={} publishable={} applied={} action={}",
                    report.release.release_id,
                    report.previous_status.as_str(),
                    report.current_status.as_str(),
                    report.publishable,
                    report.applied,
                    report.action_taken
                );
                println!("recommended_action: {}", report.recommended_action);
                if !report.problems.is_empty() {
                    println!("problems: {}", report.problems.join(" | "));
                }
                if !report.warnings.is_empty() {
                    println!("warnings: {}", report.warnings.join(" | "));
                }
            }
        }
        Some(("prepare-history-replay", sub)) => {
            let request = build_prepare_history_replay_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = prepare_history_replay(db_option_ext, request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "prepared historical replay {} config={} namespace={} parquet={}",
                    response.release_id,
                    response.replay_config_path.display(),
                    response.replay_surreal_ns,
                    response.replay_parquet_dir.display()
                );
                println!("baseline_parse: {}", response.commands.baseline_parse);
                println!("baseline_generate: {}", response.commands.baseline_generate);
                println!("baseline_register: {}", response.commands.baseline_register);
                println!("generate: {}", response.commands.generate);
                println!("publish: {}", response.commands.publish);
                println!(
                    "baseline_safety: save_db_requested={} surreal_save_feature={} target_sesno_reconstruction_supported={} source_confirmed_at_from_sesno={}",
                    response.safety_checks.baseline_config_requests_save_db,
                    response.safety_checks.baseline_binary_supports_surreal_save,
                    response
                        .safety_checks
                        .baseline_target_sesno_reconstruction_supported,
                    response
                        .safety_checks
                        .baseline_source_confirmed_at_from_sesno
                );
                println!("baseline_warning: {}", response.baseline_plan_warning);
            }
        }
        Some(("prepare-physical-baseline-snapshot", sub)) => {
            let request = build_physical_baseline_snapshot_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = prepare_physical_baseline_snapshot(db_option_ext, request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "prepared physical baseline snapshot {} project={} dbnum={} snapshot={} config={}",
                    response.snapshot_id,
                    response.project_name,
                    response.dbnum,
                    response.snapshot_project_dir.display(),
                    response.config_path.display()
                );
                println!(
                    "baseline_state_manifest: {} sha256={}",
                    response.baseline_state_manifest_path.display(),
                    response.baseline_state_manifest_hash
                );
                println!(
                    "source_db_latest_sesno: {}",
                    response.source_db_latest_sesno
                );
                println!("parse: {}", response.commands.parse);
                println!(
                    "generate_full_model: {}",
                    response.commands.generate_full_model
                );
                println!(
                    "prepare-history-replay hint: {}",
                    response.commands.prepare_history_replay_hint
                );
            }
        }
        Some(("validate-baseline-state", sub)) => {
            let request = build_baseline_state_validation_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = validate_baseline_state_request(request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "validated baseline state project={} dbnum={} from_sesno={:?} ready={} manifest={} sha256={} source_db_latest_sesno={} replacement_db={}",
                    response.project_name,
                    response.dbnum,
                    response.from_sesno,
                    response.ready,
                    response.baseline_state_manifest_path.display(),
                    response.baseline_state_manifest_hash,
                    response.source_db_latest_sesno,
                    response.replacement_db_file.display()
                );
                println!(
                    "scene_tree: required={} tree_file={} exists={} db_meta_info={} exists={}",
                    response.scene_tree.required,
                    response.scene_tree.tree_file.display(),
                    response.scene_tree.tree_file_exists,
                    response.scene_tree.db_meta_info_file.display(),
                    response.scene_tree.db_meta_info_exists
                );
                println!("recommended_action: {}", response.recommended_action);
            }
        }
        Some(("run-command", sub)) => {
            let request = build_bounded_run_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = run_bounded_command(request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "run {} kind={} status={:?} pid={:?} exit_code={:?} elapsed_ms={} state={}",
                    response.run_id,
                    response.kind,
                    response.status,
                    response.pid,
                    response.exit_code,
                    response.elapsed_ms,
                    response.state_path.display()
                );
            }
        }
        Some(("run-status", sub)) => {
            let run_id = sub.get_one::<String>("run-id").expect("required by clap");
            let state_dir = runner_state_dir_from_matches(sub, db_option_ext);
            let response = read_bounded_run_status(&state_dir, run_id)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "run {} kind={} status={:?} pid={:?} exit_code={:?} elapsed_ms={} state={}",
                    response.run_id,
                    response.kind,
                    response.status,
                    response.pid,
                    response.exit_code,
                    response.elapsed_ms,
                    response.state_path.display()
                );
            }
        }
        Some(("cancel-run", sub)) => {
            let run_id = sub.get_one::<String>("run-id").expect("required by clap");
            let state_dir = runner_state_dir_from_matches(sub, db_option_ext);
            let response = request_bounded_run_cancel(
                &state_dir,
                run_id,
                sub.get_one::<String>("reason").cloned(),
            )?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "cancel requested for {} previous_status={:?} pid={:?} kill_attempted={} marker={}",
                    response.run_id,
                    response.previous_status,
                    response.pid,
                    response.kill_attempted,
                    response.cancel_path.display()
                );
            }
        }
        Some(("inspect-history-baseline", sub)) => {
            let request = build_history_baseline_inspect_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = inspect_history_baseline(request).await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "inspected historical baseline dbnum={} requested_sesno={} resolved_sesno={} visible_refnos={} parsed_sample={} parse_errors={} action={}",
                    response.header_dbnum,
                    response.requested_sesno,
                    response.resolved_sesno,
                    response.visible_refno_count,
                    response.parsed_sample_count,
                    response.parse_error_count,
                    response.recommended_next_action
                );
            }
        }
        Some(("validate-history-replay", sub)) => {
            let request = build_validate_history_replay_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let allow_patch_only = sub.get_flag("allow-patch-only");
            let response = validate_history_replay_package(request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "validated historical replay dbnum={} classification={} ready_for_publish={} package={} action={}",
                    response.dbnum,
                    response.classification,
                    response.ready_for_publish,
                    response.source_parquet_dir.display(),
                    response.recommended_action
                );
            }
            if !allow_patch_only {
                ensure_history_replay_publishable(&response)?;
            }
        }
        Some(("repair-missing-meshes", sub)) => {
            let request = build_missing_mesh_repair_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = repair_missing_meshes(db_option_ext, request).await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "repaired missing meshes dbnum={} attempted={} generated={} still_missing={} report={} action={}",
                    response.dbnum,
                    response.attempted_hashes,
                    response.generated_hashes,
                    response.still_missing_hashes,
                    response.report_file.display(),
                    response.recommended_action
                );
            }
        }
        Some(("restore-scene-tree-artifact", sub)) => {
            let request = build_scene_tree_artifact_restore_request(sub, db_option_ext)?;
            let json_output = sub.get_flag("json");
            let response = restore_scene_tree_artifact(request)?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "restored scene_tree dbnum={} copied={} meta_written={} source_hash={} target={} action={}",
                    response.dbnum,
                    response.tree_copied,
                    response.db_meta_written,
                    response.source_tree_sha256,
                    response.target_tree_file.display(),
                    response.recommended_action
                );
                if !response.added_ref0s.is_empty() {
                    println!("added_ref0s={:?}", response.added_ref0s);
                }
                if !response.warnings.is_empty() {
                    println!("warnings={}", response.warnings.join(" | "));
                }
            }
        }
        Some(("list", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let filter = if sub.get_flag("all-projects") {
                None
            } else {
                Some(project_name.as_str())
            };
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let response = list_model_releases(ducklake, filter)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                for release in response.releases {
                    println!(
                        "{} dbnum={} branch={} lifecycle={} quality={} legacy_status={} registered_at={} package={}",
                        release.release_id,
                        release.dbnum,
                        release.branch_id,
                        release.release_lifecycle.as_str(),
                        release.release_quality.as_str(),
                        release.release_status.as_str(),
                        release.registered_at,
                        release.immutable_package_dir.display()
                    );
                }
            }
        }
        Some(("index", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = sub
                .get_one::<String>("release-id")
                .expect("required by clap");
            let response = index_model_release_components(ducklake, release_id)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "indexed release {} components={} distinct_hashes={}",
                    response.release_id,
                    response.component_count,
                    response.distinct_component_hashes
                );
            }
        }
        Some(("index-units", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = sub
                .get_one::<String>("release-id")
                .expect("required by clap");
            eprintln!(
                "warning: index-units is DEPRECATED (specs/023); prefer write_unit_version_with_members_v2 / unit-v2-*"
            );
            if let Some((dbnum, sesno)) = parse_legacy_batch_id(release_id) {
                eprintln!(
                    "info: release_id parses as dbnum={dbnum} sesno={sesno}; register --index-units --sesno will sync into unit_versions_v2"
                );
            }
            let response = index_model_release_units(ducklake, release_id)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "indexed release {} units={} members={} unresolved={}",
                    response.release_id,
                    response.unit_count,
                    response.member_count,
                    response.unresolved_member_count
                );
            }
        }
        Some(("unit-v2-smoke", sub)) => {
            let work_dir = sub
                .get_one::<String>("work-dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::temp_dir().join(format!(
                        "aios-unit-v2-smoke-{}",
                        chrono::Utc::now().timestamp_millis()
                    ))
                });
            let report = ModelVersionDuckLakeStore::smoke_unit_version_v2(&work_dir)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "unit-v2-smoke ok={} derived_sesno={} first={} second={} conflict_rejected={} listed={} work_dir={}",
                    report.ok,
                    report.derived_sesno,
                    report.first_outcome,
                    report.second_outcome,
                    report.conflict_rejected,
                    report.listed_count,
                    report.work_dir.display()
                );
            }
        }
        Some(("unit-v2-get", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let store = ModelVersionDuckLakeStore::open(ducklake)?;
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let refno = *sub.get_one::<u64>("refno").expect("required");
            let sesno = *sub.get_one::<u32>("sesno").expect("required");
            let record = store.get_unit_version_v2(dbnum, refno, sesno)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&record)?);
            } else {
                match record {
                    Some(r) => println!(
                        "unit dbnum={} refno={} sesno={} hash={} members={}",
                        r.dbnum, r.unit_refno_u64, r.sesno, r.aggregate_hash, r.member_count
                    ),
                    None => println!("unit version not found"),
                }
            }
        }
        Some(("unit-v2-list", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let store = ModelVersionDuckLakeStore::open(ducklake)?;
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let refno = *sub.get_one::<u64>("refno").expect("required");
            let rows = store.list_unit_versions_v2(dbnum, refno)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for r in rows {
                    println!(
                        "sesno={} hash={} members={} indexed_at={}",
                        r.sesno, r.aggregate_hash, r.member_count, r.indexed_at
                    );
                }
            }
        }
        Some(("unit-v2-diff", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let store = ModelVersionDuckLakeStore::open(ducklake)?;
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required");
            let to_sesno = *sub.get_one::<u32>("to-sesno").expect("required");
            let refno = sub.get_one::<u64>("refno").copied();
            let limit = *sub.get_one::<usize>("limit").expect("default");
            let response =
                store.diff_unit_versions_v2(dbnum, from_sesno, to_sesno, refno, limit)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "diff dbnum={} {}->{} added={} deleted={} changed={} unchanged={} emitted={}",
                    response.dbnum,
                    response.from_sesno,
                    response.to_sesno,
                    response.summary.added,
                    response.summary.deleted,
                    response.summary.changed,
                    response.summary.unchanged,
                    response.summary.emitted
                );
                for row in response.rows {
                    println!(
                        "{} refno={} old_hash={:?} new_hash={:?}",
                        row.change_type, row.unit_refno_u64, row.old_aggregate_hash, row.new_aggregate_hash
                    );
                }
            }
        }
        Some(("unit-v2-set-status", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let store = ModelVersionDuckLakeStore::open_writer(ducklake)?;
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let refno = *sub.get_one::<u64>("refno").expect("required");
            let sesno = *sub.get_one::<u32>("sesno").expect("required");
            let status = sub.get_one::<String>("status").expect("required");
            let reason = sub.get_one::<String>("reason").map(String::as_str);
            let record = store.set_unit_version_status_v2(dbnum, refno, sesno, status, reason)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&record)?);
            } else {
                println!(
                    "unit status dbnum={} refno={} sesno={} status={}",
                    record.dbnum,
                    record.unit_refno_u64,
                    record.sesno,
                    record.status.unwrap_or_default()
                );
            }
        }
        Some(("unit-v2-events", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let store = ModelVersionDuckLakeStore::open(ducklake)?;
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let refno = *sub.get_one::<u64>("refno").expect("required");
            let sesno = *sub.get_one::<u32>("sesno").expect("required");
            let events = store.list_unit_version_events_v2(dbnum, refno, sesno)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&events)?);
            } else {
                println!("events={}", events.len());
                for event in events {
                    println!(
                        "{} status={} reason={}",
                        event.created_at,
                        event.status,
                        event.reason.unwrap_or_default()
                    );
                }
            }
        }
        Some(("index-assets", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = sub
                .get_one::<String>("release-id")
                .expect("required by clap");
            let mesh_root = mesh_root_from_matches(sub, db_option_ext);
            let mesh_base_url = sub.get_one::<String>("mesh-base-url").map(String::as_str);
            let response = index_model_release_mesh_assets(
                ducklake,
                release_id,
                &mesh_root,
                mesh_base_url,
                sub.get_flag("materialize"),
            )?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "indexed release {} mesh_assets={} present={} missing={} glb_checked={} glb_readable={} glb_unreadable={} manifest={}",
                    response.release_id,
                    response.geo_hash_count,
                    response.present_count,
                    response.missing_count,
                    response.glb_checked_count.unwrap_or(0),
                    response.glb_readable_count.unwrap_or(0),
                    response.glb_unreadable_count.unwrap_or(0),
                    response.manifest_path.display()
                );
            }
        }
        Some(("mesh-assets", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let release_id = sub
                .get_one::<String>("release-id")
                .expect("required by clap");
            let limit = sub
                .get_one::<usize>("limit")
                .copied()
                .expect("default value ensures this exists");
            let response = get_model_release_mesh_assets(
                ducklake,
                release_id,
                limit,
                sub.get_flag("missing-only"),
            )?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "release {} mesh_assets={} present={} missing={} emitted={}",
                    response.stats.release_id,
                    response.stats.geo_hash_count,
                    response.stats.present_count,
                    response.stats.missing_count,
                    response.assets.len()
                );
            }
        }
        Some(("validate-compare-readiness", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let from_release_id = sub
                .get_one::<String>("from-release-id")
                .expect("required by clap");
            let to_release_id = sub
                .get_one::<String>("to-release-id")
                .expect("required by clap");
            let response =
                validate_model_release_pair_readiness(ducklake, from_release_id, to_release_id)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "{} -> {} classification={} production_ready={} action={}",
                    response.from_release_id,
                    response.to_release_id,
                    response.classification,
                    response.production_ready,
                    response.recommended_action
                );
                for problem in &response.problems {
                    println!("problem: {problem}");
                }
                for warning in &response.warnings {
                    println!("warning: {warning}");
                }
            }
        }
        Some(("diff", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let from_release_id = sub
                .get_one::<String>("from-release-id")
                .expect("required by clap");
            let to_release_id = sub
                .get_one::<String>("to-release-id")
                .expect("required by clap");
            let limit = sub
                .get_one::<usize>("limit")
                .copied()
                .expect("default value ensures this exists");
            let change_type = sub.get_one::<String>("change-type").map(String::as_str);
            let response =
                diff_model_releases(ducklake, from_release_id, to_release_id, limit, change_type)?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "{} -> {} added={} deleted={} changed={} unchanged={} emitted={}",
                    response.from_release_id,
                    response.to_release_id,
                    response.summary.added,
                    response.summary.deleted,
                    response.summary.changed,
                    response.summary.unchanged,
                    response.summary.emitted
                );
            }
        }
        Some(("unit-diff", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let limit = sub
                .get_one::<usize>("limit")
                .copied()
                .expect("default value ensures this exists");
            let from_sesno = sub.get_one::<u32>("from-sesno").copied();
            let to_sesno = sub.get_one::<u32>("to-sesno").copied();
            let dbnum = sub.get_one::<u32>("dbnum").copied();
            let refno = sub.get_one::<u64>("refno").copied();
            let from_release_id = sub.get_one::<String>("from-release-id").cloned();
            let to_release_id = sub.get_one::<String>("to-release-id").cloned();

            let sesno_mode = match (dbnum, from_sesno, to_sesno) {
                (Some(db), Some(from), Some(to)) => Some((db, from, to)),
                (None, None, None) => None,
                _ => anyhow::bail!(
                    "unit-diff sesno mode requires --dbnum --from-sesno --to-sesno together (specs/023)"
                ),
            };

            if let Some((db, from, to)) = sesno_mode {
                if from_release_id.is_some() || to_release_id.is_some() {
                    eprintln!(
                        "warning: unit-diff ignoring deprecated --from-release-id/--to-release-id because sesno mode is set"
                    );
                }
                if sub.get_one::<String>("unit-noun").is_some() {
                    eprintln!(
                        "warning: --unit-noun is not applied in sesno mode; use unit-v2-diff filters later if needed"
                    );
                }
                let store = ModelVersionDuckLakeStore::open(ducklake)?;
                let response = store.diff_unit_versions_v2(db, from, to, refno, limit)?;
                if sub.get_flag("json") {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                } else {
                    println!(
                        "diff dbnum={} {}->{} added={} deleted={} changed={} unchanged={} emitted={}",
                        response.dbnum,
                        response.from_sesno,
                        response.to_sesno,
                        response.summary.added,
                        response.summary.deleted,
                        response.summary.changed,
                        response.summary.unchanged,
                        response.summary.emitted
                    );
                    for row in response.rows {
                        println!(
                            "{} refno={} old_hash={:?} new_hash={:?}",
                            row.change_type,
                            row.unit_refno_u64,
                            row.old_aggregate_hash,
                            row.new_aggregate_hash
                        );
                    }
                }
            } else {
                let from_release_id = from_release_id.ok_or_else(|| {
                    anyhow::anyhow!(
                        "unit-diff requires either (--dbnum --from-sesno --to-sesno) or (--from-release-id --to-release-id)"
                    )
                })?;
                let to_release_id = to_release_id.ok_or_else(|| {
                    anyhow::anyhow!(
                        "unit-diff requires either (--dbnum --from-sesno --to-sesno) or (--from-release-id --to-release-id)"
                    )
                })?;
                eprintln!(
                    "warning: unit-diff --from-release-id/--to-release-id is DEPRECATED (specs/023); prefer --dbnum --from-sesno --to-sesno"
                );
                // Map parseable db{{N}}-s{{M}} aliases onto v2 diff when both sides agree on dbnum.
                match (
                    parse_legacy_batch_id(&from_release_id),
                    parse_legacy_batch_id(&to_release_id),
                ) {
                    (Some((from_db, from)), Some((to_db, to))) if from_db == to_db => {
                        eprintln!(
                            "info: mapped release_id aliases to sesno mode dbnum={from_db} {from}->{to}"
                        );
                        let store = ModelVersionDuckLakeStore::open(ducklake)?;
                        let response =
                            store.diff_unit_versions_v2(from_db, from, to, refno, limit)?;
                        if sub.get_flag("json") {
                            println!("{}", serde_json::to_string_pretty(&response)?);
                        } else {
                            println!(
                                "diff dbnum={} {}->{} added={} deleted={} changed={} unchanged={} emitted={}",
                                response.dbnum,
                                response.from_sesno,
                                response.to_sesno,
                                response.summary.added,
                                response.summary.deleted,
                                response.summary.changed,
                                response.summary.unchanged,
                                response.summary.emitted
                            );
                        }
                    }
                    _ => {
                        let unit_noun =
                            sub.get_one::<String>("unit-noun").map(String::as_str);
                        let response = diff_model_release_units(
                            ducklake,
                            &from_release_id,
                            &to_release_id,
                            limit,
                            unit_noun,
                        )?;
                        if sub.get_flag("json") {
                            println!("{}", serde_json::to_string_pretty(&response)?);
                        } else {
                            println!(
                                "{} -> {} unit_added={} unit_deleted={} unit_changed={} unit_unchanged={} emitted={}",
                                response.from_release_id,
                                response.to_release_id,
                                response.summary.added,
                                response.summary.deleted,
                                response.summary.changed,
                                response.summary.unchanged,
                                response.summary.emitted
                            );
                        }
                    }
                }
            }
        }
        Some(("impact", sub)) => {
            let project_name = project_name_from_matches(sub, db_option_ext);
            let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
            let from_release_id = sub
                .get_one::<String>("from-release-id")
                .expect("required by clap");
            let to_release_id = sub
                .get_one::<String>("to-release-id")
                .expect("required by clap");
            let limit = sub
                .get_one::<usize>("limit")
                .copied()
                .expect("default value ensures this exists");
            let component_key = component_key_filter_from_matches(sub)?;
            let response = get_model_component_unit_impacts(
                ducklake,
                from_release_id,
                to_release_id,
                limit,
                component_key.as_deref(),
            )?;
            if sub.get_flag("json") {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!(
                    "{} -> {} component_changes={} impacted_units={} emitted={}",
                    response.from_release_id,
                    response.to_release_id,
                    response.summary.component_changes,
                    response.summary.impacted_units,
                    response.summary.emitted
                );
            }
        }
        Some(("history", hist)) => {
            handle_history_command(hist, db_option_ext).await?;
        }
        _ => unreachable!("subcommand_required by clap"),
    }

    Ok(true)
}

async fn build_source_observation_response(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelSourceObservationResponse> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let observation_id = sub
        .get_one::<String>("observation-id")
        .cloned()
        .unwrap_or_else(|| default_source_observation_id(dbnum));
    let manifest_path = sub
        .get_one::<String>("manifest-out")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("model_versions")
                .join("source_observations")
                .join(format!("{observation_id}.json"))
        });
    if manifest_path.exists() && !sub.get_flag("force") {
        anyhow::bail!(
            "source observation manifest already exists; pass --force to overwrite: {}",
            manifest_path.display()
        );
    }

    let source_db_file = resolve_source_db_file_for_observation(
        sub,
        db_option_ext,
        &project_name,
        dbnum,
        sub.get_flag("rescan-index"),
    )
    .await?;
    validate_observed_source_dbnum(&source_db_file, dbnum)?;

    let resolved_sesno = match sub.get_one::<u32>("resolved-sesno").copied() {
        Some(value) => Some(value),
        None => Some(read_source_db_latest_sesno(&project_name, &source_db_file)?),
    };
    let dependency_files = sub
        .get_many::<String>("dependency-file")
        .map(|values| values.map(PathBuf::from).collect())
        .unwrap_or_default();
    let requested_sesno = sub.get_one::<String>("requested-sesno").cloned();

    let observation = build_source_observation_manifest(SourceObservationBuildRequest {
        observation_id,
        project_name: project_name.clone(),
        dbnum,
        primary_file: source_db_file.clone(),
        dependency_files,
        requested_sesno: requested_sesno.clone(),
        resolved_sesno,
        quiescence_window_ms: sub
            .get_one::<u64>("quiescence-window-ms")
            .copied()
            .expect("default value ensures this exists"),
    })?;
    let observation_manifest_hash =
        write_source_observation_manifest(&manifest_path, &observation)?;
    let ready_for_increment = observation.quiescence.stable;
    let (status, recommended_action) = if ready_for_increment {
        (
            "stable".to_string(),
            "Source DB observation is stable; it may be used as immutable evidence for the next incremental parse/generation step.".to_string(),
        )
    } else {
        (
            "source_unstable".to_string(),
            "Source DB changed during the observation window; wait for a quiet window and rerun observe-source before parsing or generating.".to_string(),
        )
    };

    Ok(ModelSourceObservationResponse {
        project_name,
        dbnum,
        source_db_file: observation.primary.path.clone(),
        requested_sesno,
        resolved_sesno,
        ready_for_increment,
        status,
        observation_manifest_path: manifest_path,
        observation_manifest_hash,
        observation,
        recommended_action,
    })
}

async fn resolve_source_db_file_for_observation(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
    project_name: &str,
    dbnum: u32,
    rescan_index: bool,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = sub.get_one::<String>("source-db-file").map(PathBuf::from) {
        if !path.is_file() {
            anyhow::bail!(
                "source DB file is missing or not a file: {}",
                path.display()
            );
        }
        return Ok(path);
    }

    resolve_source_db_file_from_index(db_option_ext, project_name, dbnum, rescan_index).await
}

#[cfg(feature = "sqlite-index")]
async fn resolve_source_db_file_from_index(
    _db_option_ext: &DbOptionExt,
    project_name: &str,
    dbnum: u32,
    rescan_index: bool,
) -> anyhow::Result<PathBuf> {
    let index_path = crate::data_interface::db_index::default_index_path(project_name);
    if rescan_index || !index_path.exists() {
        crate::data_interface::db_index::rebuild_from_config(false).await?;
    }
    let store = crate::data_interface::db_index::DbIndexStore::open(&index_path)?;
    let record = store.file_by_dbnum(dbnum).ok_or_else(|| {
        anyhow::anyhow!(
            "dbnum {} was not found in db_index {}; pass --source-db-file or rerun with --rescan-index",
            dbnum,
            index_path.display()
        )
    })?;
    let path = PathBuf::from(record.file_path);
    if !path.is_file() {
        anyhow::bail!(
            "db_index resolved dbnum {} to a missing source DB file: {}",
            dbnum,
            path.display()
        );
    }
    Ok(path)
}

#[cfg(not(feature = "sqlite-index"))]
async fn resolve_source_db_file_from_index(
    _db_option_ext: &DbOptionExt,
    project_name: &str,
    dbnum: u32,
    _rescan_index: bool,
) -> anyhow::Result<PathBuf> {
    anyhow::bail!(
        "model-version observe-source needs --source-db-file for dbnum {} because this binary was not built with sqlite-index (project={})",
        dbnum,
        project_name
    );
}

fn validate_observed_source_dbnum(
    path: &std::path::Path,
    expected_dbnum: u32,
) -> anyhow::Result<()> {
    let info = parse_pdms_db::parse::parse_db_basic_info(path.to_path_buf());
    if info.dbnum != expected_dbnum {
        anyhow::bail!(
            "source DB dbnum mismatch: expected {}, got {} for {}",
            expected_dbnum,
            info.dbnum,
            path.display()
        );
    }
    Ok(())
}

fn read_source_db_latest_sesno(project_name: &str, path: &std::path::Path) -> anyhow::Result<u32> {
    let mut io = pdms_io::PdmsIO::new(project_name, path, false);
    io.open()
        .with_context(|| format!("open source DB for latest sesno failed: {}", path.display()))?;
    io.get_latest_sesno()
        .with_context(|| format!("read source DB latest sesno failed: {}", path.display()))
}

fn default_source_observation_id(dbnum: u32) -> String {
    format!(
        "source-db{}-{}",
        dbnum,
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ")
    )
}

#[derive(Debug, Serialize)]
struct ModelReleaseSpecInfoAuditResponse {
    release_id: String,
    package_dir: PathBuf,
    manifest_path: PathBuf,
    manifest_spec_info_fallback_count: Option<u64>,
    manifest_spec_info_validation_count: Option<u64>,
    instance_rows: u64,
    instance_zero_spec_value_rows: u64,
    tubing_rows: u64,
    tubing_zero_spec_value_rows: u64,
    legacy_zero_spec_value_count: u64,
    recommended_action: String,
}

#[derive(Default)]
struct SpecValueZeroCount {
    rows: u64,
    zero_rows: u64,
}

fn audit_release_spec_info(
    ducklake: ModelVersionDuckLakeConfig,
    release_id: &str,
) -> anyhow::Result<ModelReleaseSpecInfoAuditResponse> {
    let store = ModelVersionDuckLakeStore::open_readonly(ducklake)?;
    let release = store.get_release(release_id)?;
    let package_dir = release.immutable_package_dir.clone();
    let manifest_path = package_dir.join("manifest.json");
    let manifest_json: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).with_context(|| {
            format!("read release manifest failed: {}", manifest_path.display())
        })?)
        .with_context(|| {
            format!(
                "parse release manifest JSON failed: {}",
                manifest_path.display()
            )
        })?;
    let manifest_spec_info_fallback_count =
        manifest_u64_at(&manifest_json, &["spec_info_fallback_count"]);
    let manifest_spec_info_validation_count =
        manifest_u64_at(&manifest_json, &["spec_info_validation", "fallback_count"]);

    let instances = count_zero_spec_values(&package_dir.join("instances.parquet"), "instances")?;
    let tubings = count_zero_spec_values(&package_dir.join("tubings.parquet"), "tubings")?;
    let legacy_zero_spec_value_count = instances.zero_rows + tubings.zero_rows;
    let recommended_action = if manifest_spec_info_fallback_count.is_some() {
        "manifest already carries generated spec_info fallback evidence; prefer manifest count over legacy zero-row audit"
    } else if legacy_zero_spec_value_count > 0 {
        "legacy package has zero spec_value rows; keep quarantined or annotate only with explicit legacy-zero audit evidence"
    } else {
        "legacy package has no zero spec_value rows in instances/tubings; spec_info fallback flag can be reviewed for removal only with matching release evidence"
    }
    .to_string();

    Ok(ModelReleaseSpecInfoAuditResponse {
        release_id: release.release_id,
        package_dir,
        manifest_path,
        manifest_spec_info_fallback_count,
        manifest_spec_info_validation_count,
        instance_rows: instances.rows,
        instance_zero_spec_value_rows: instances.zero_rows,
        tubing_rows: tubings.rows,
        tubing_zero_spec_value_rows: tubings.zero_rows,
        legacy_zero_spec_value_count,
        recommended_action,
    })
}

fn manifest_u64_at(value: &Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_u64()
}

fn count_zero_spec_values(path: &Path, table_name: &str) -> anyhow::Result<SpecValueZeroCount> {
    use arrow_array::{Array, UInt64Array};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::fs::File;

    let file = File::open(path)
        .with_context(|| format!("open {table_name} parquet failed: {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| {
            format!(
                "read {table_name} parquet metadata failed: {}",
                path.display()
            )
        })?
        .build()
        .with_context(|| {
            format!(
                "create {table_name} parquet reader failed: {}",
                path.display()
            )
        })?;

    let mut result = SpecValueZeroCount::default();
    for batch in reader {
        let batch = batch.with_context(|| {
            format!("read {table_name} parquet batch failed: {}", path.display())
        })?;
        let spec_col = batch
            .column_by_name("spec_value")
            .and_then(|column| column.as_any().downcast_ref::<UInt64Array>())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} missing UInt64 spec_value column in {}",
                    table_name,
                    path.display()
                )
            })?;
        for row in 0..batch.num_rows() {
            result.rows += 1;
            if spec_col.is_null(row) || spec_col.value(row) == 0 {
                result.zero_rows += 1;
            }
        }
    }
    Ok(result)
}

fn build_register_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelReleaseRegisterRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let export_sesno = sub.get_one::<u32>("sesno").copied();
    let release_id = match sub.get_one::<String>("release-id") {
        Some(id) => id.to_string(),
        None => {
            let sesno = export_sesno.ok_or_else(|| {
                anyhow::anyhow!("register requires --sesno when --release-id is omitted (specs/023)")
            })?;
            legacy_batch_id_for_sesno(dbnum, sesno)
        }
    };
    let source_parquet_dir = sub
        .get_one::<String>("parquet-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("parquet")
                .join(dbnum.to_string())
        });
    let release_root = sub
        .get_one::<String>("release-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("model_versions")
                .join("releases")
        });
    let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
    let extra_metadata = parse_metadata_json(sub)?;

    Ok(ModelReleaseRegisterRequest {
        project_name,
        release_id,
        release_label: sub.get_one::<String>("release-label").cloned(),
        release_quality: release_quality_from_matches(sub)?,
        release_quality_reason: sub.get_one::<String>("release-quality-reason").cloned(),
        validation_flags: validation_flags_from_matches(sub),
        spec_info_fallback_count: sub.get_one::<u64>("spec-info-fallback-count").copied(),
        branch_id: sub
            .get_one::<String>("branch-id")
            .expect("default value ensures this exists")
            .to_string(),
        parent_release_id: sub.get_one::<String>("parent-release-id").cloned(),
        derivation_type: sub
            .get_one::<String>("derivation-type")
            .expect("default value ensures this exists")
            .to_string(),
        dbnum,
        export_sesno,
        source_parquet_dir,
        release_root,
        ducklake,
        extra_metadata,
        initial_status: ModelReleaseStatus::Staged,
        index_units: sub.get_flag("index-units"),
    })
}

fn build_publish_history_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelHistoryReleasePublishRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let to_sesno = sub
        .get_one::<u32>("to-sesno")
        .copied()
        .expect("required by clap");
    let release_id = sub
        .get_one::<String>("release-id")
        .cloned()
        .unwrap_or_else(|| legacy_batch_id_for_sesno(dbnum, to_sesno));
    let source_parquet_dir = sub
        .get_one::<String>("parquet-dir")
        .map(PathBuf::from)
        .expect("required by clap");
    let current_parquet_dir = sub
        .get_one::<String>("current-parquet-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("parquet")
                .join(dbnum.to_string())
        });
    let release_root = sub
        .get_one::<String>("release-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("model_versions")
                .join("releases")
        });
    let materialize_assets = sub.get_flag("materialize-assets");
    let mesh_root = if materialize_assets || sub.contains_id("mesh-root") {
        Some(mesh_root_from_matches(sub, db_option_ext))
    } else {
        None
    };
    let ducklake = ducklake_config_from_matches(sub, db_option_ext, &project_name);
    let extra_metadata = parse_metadata_json(sub)?;

    Ok(ModelHistoryReleasePublishRequest {
        project_name,
        release_id,
        release_label: sub.get_one::<String>("release-label").cloned(),
        release_quality: release_quality_from_matches(sub)?,
        release_quality_reason: sub.get_one::<String>("release-quality-reason").cloned(),
        validation_flags: validation_flags_from_matches(sub),
        spec_info_fallback_count: sub.get_one::<u64>("spec-info-fallback-count").copied(),
        branch_id: sub
            .get_one::<String>("branch-id")
            .expect("default value ensures this exists")
            .to_string(),
        parent_release_id: sub.get_one::<String>("parent-release-id").cloned(),
        dbnum,
        source_db_file: sub
            .get_one::<String>("source-db-file")
            .map(PathBuf::from)
            .expect("required by clap"),
        from_sesno: sub
            .get_one::<u32>("from-sesno")
            .copied()
            .expect("required by clap"),
        to_sesno,
        source_parquet_dir,
        current_parquet_dir,
        scene_tree_dir: sub.get_one::<String>("scene-tree-dir").map(PathBuf::from),
        require_scene_tree: sub.get_flag("require-scene-tree"),
        release_root,
        ducklake,
        extra_metadata,
        mesh_root,
        mesh_base_url: sub.get_one::<String>("mesh-base-url").cloned(),
        materialize_assets,
        index_units: sub.get_flag("index-units"),
    })
}

fn build_prepare_history_replay_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelHistoryReplayPrepareRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let release_id = sub
        .get_one::<String>("release-id")
        .expect("required by clap")
        .to_string();
    let current_parquet_dir = sub
        .get_one::<String>("current-parquet-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("parquet")
                .join(dbnum.to_string())
        });
    let base_config_arg = sub
        .get_one::<String>("base-config")
        .cloned()
        .unwrap_or_else(current_db_option_file_arg);
    let replay_config_arg = sub
        .get_one::<String>("replay-config-out")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("model_versions")
                .join("replay_configs")
                .join(&release_id)
        });
    let baseline_dbnums = sub
        .get_many::<u32>("baseline-dbnum")
        .map(|values| values.copied().collect())
        .unwrap_or_else(|| vec![dbnum]);

    Ok(ModelHistoryReplayPrepareRequest {
        project_name,
        release_id,
        release_label: sub.get_one::<String>("release-label").cloned(),
        baseline_release_id: sub.get_one::<String>("baseline-release-id").cloned(),
        branch_id: sub
            .get_one::<String>("branch-id")
            .expect("default value ensures this exists")
            .to_string(),
        parent_release_id: sub.get_one::<String>("parent-release-id").cloned(),
        dbnum,
        baseline_dbnums,
        source_db_file: sub
            .get_one::<String>("source-db-file")
            .map(PathBuf::from)
            .expect("required by clap"),
        from_sesno: sub
            .get_one::<u32>("from-sesno")
            .copied()
            .expect("required by clap"),
        to_sesno: sub
            .get_one::<u32>("to-sesno")
            .copied()
            .expect("required by clap"),
        base_config_arg,
        baseline_config_arg: sub
            .get_one::<String>("baseline-config-out")
            .map(PathBuf::from),
        replay_config_arg,
        replay_surreal_ns: sub.get_one::<String>("replay-surreal-ns").cloned(),
        replay_output_root: sub
            .get_one::<String>("replay-output-root")
            .map(PathBuf::from),
        current_parquet_dir,
        baseline_source_confirmed_at_from_sesno: sub
            .get_flag("baseline-source-confirmed-at-from-sesno"),
        force: sub.get_flag("force"),
    })
}

fn build_history_baseline_inspect_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<HistoryBaselineInspectRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let source_db_file = sub
        .get_one::<String>("source-db-file")
        .map(PathBuf::from)
        .expect("required by clap");
    if !source_db_file.is_file() {
        anyhow::bail!(
            "source DB file is missing or not a file: {}",
            source_db_file.display()
        );
    }

    Ok(HistoryBaselineInspectRequest {
        project_name,
        source_db_file,
        target_sesno: sub
            .get_one::<u32>("target-sesno")
            .copied()
            .expect("required by clap"),
        parse_sample_limit: sub
            .get_one::<usize>("parse-sample-limit")
            .copied()
            .expect("default value ensures this exists"),
        require_exact_sesno: !sub.get_flag("allow-nearest-sesno"),
        detail: sub.get_flag("detail"),
    })
}

fn build_physical_baseline_snapshot_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelPhysicalBaselineSnapshotRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let source_db_file = sub
        .get_one::<String>("source-db-file")
        .map(PathBuf::from)
        .expect("required by clap");
    if !source_db_file.is_file() {
        anyhow::bail!(
            "source DB file is missing or not a file: {}",
            source_db_file.display()
        );
    }
    let baseline_dbnums = sub
        .get_many::<u32>("baseline-dbnum")
        .map(|values| values.copied().collect())
        .unwrap_or_else(Vec::new);
    let base_config_arg = sub
        .get_one::<String>("base-config")
        .cloned()
        .unwrap_or_else(current_db_option_file_arg);

    Ok(ModelPhysicalBaselineSnapshotRequest {
        project_name,
        snapshot_id: sub
            .get_one::<String>("snapshot-id")
            .expect("required by clap")
            .to_string(),
        dbnum: sub
            .get_one::<u32>("dbnum")
            .copied()
            .expect("required by clap"),
        source_db_file,
        baseline_dbnums,
        base_config_arg,
        config_arg: sub.get_one::<String>("config-out").map(PathBuf::from),
        snapshot_root: sub.get_one::<String>("snapshot-root").map(PathBuf::from),
        output_root: sub.get_one::<String>("output-root").map(PathBuf::from),
        surreal_ns: sub.get_one::<String>("surreal-ns").cloned(),
        copy_files: sub.get_flag("copy-files"),
        force: sub.get_flag("force"),
    })
}

fn build_baseline_state_validation_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelBaselineStateValidationRequest> {
    Ok(ModelBaselineStateValidationRequest {
        project_name: project_name_from_matches(sub, db_option_ext),
        dbnum: Some(
            sub.get_one::<u32>("dbnum")
                .copied()
                .expect("required by clap"),
        ),
        from_sesno: sub.get_one::<u32>("from-sesno").copied(),
        baseline_state_manifest_path: sub
            .get_one::<String>("baseline-state-manifest")
            .map(PathBuf::from)
            .expect("required by clap"),
        baseline_state_manifest_hash: sub
            .get_one::<String>("baseline-state-manifest-hash")
            .cloned(),
        scene_tree_dir: sub.get_one::<String>("scene-tree-dir").map(PathBuf::from),
        require_scene_tree: sub.get_flag("require-scene-tree"),
    })
}

fn build_bounded_run_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<BoundedCommandRunRequest> {
    let state_dir = runner_state_dir_from_matches(sub, db_option_ext);
    let argv = bounded_argv_from_matches(sub)?;
    let env_values = sub
        .get_many::<String>("env")
        .map(|values| values.cloned().collect::<Vec<_>>());
    let env = parse_env_assignments(env_values)?;
    Ok(BoundedCommandRunRequest {
        run_id: sub
            .get_one::<String>("run-id")
            .expect("required by clap")
            .to_string(),
        kind: sub
            .get_one::<String>("kind")
            .expect("default value ensures this exists")
            .to_string(),
        state_dir,
        executable: sub.get_one::<String>("executable").map(PathBuf::from),
        argv,
        cwd: sub
            .get_one::<String>("cwd")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
        env,
        stdout_path: sub.get_one::<String>("stdout-path").map(PathBuf::from),
        stderr_path: sub.get_one::<String>("stderr-path").map(PathBuf::from),
        metrics_path: sub.get_one::<String>("metrics-path").map(PathBuf::from),
        timeout_secs: sub
            .get_one::<u64>("timeout-secs")
            .copied()
            .expect("default value ensures this exists"),
        stale_heartbeat_secs: sub.get_one::<u64>("stale-heartbeat-secs").copied(),
        source_db_file: sub.get_one::<String>("source-db-file").map(PathBuf::from),
        expected_source_db_sha256: sub.get_one::<String>("source-db-sha256").cloned(),
        poll_interval_ms: sub
            .get_one::<u64>("poll-interval-ms")
            .copied()
            .expect("default value ensures this exists"),
        force: sub.get_flag("force"),
    })
}

fn bounded_argv_from_matches(sub: &ArgMatches) -> anyhow::Result<Vec<String>> {
    let argv_json = sub.get_one::<String>("argv-json");
    let argv_file = sub.get_one::<String>("argv-file");
    match (argv_json, argv_file) {
        (Some(_), Some(_)) => anyhow::bail!("pass only one of --argv-json or --argv-file"),
        (Some(raw), None) => parse_argv_json(raw),
        (None, Some(path)) => {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("read argv file failed: {}", path))?;
            parse_argv_json(&content)
        }
        (None, None) => anyhow::bail!("one of --argv-json or --argv-file is required"),
    }
}

fn build_validate_history_replay_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelHistoryReplayValidationRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let current_parquet_dir = sub
        .get_one::<String>("current-parquet-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("parquet")
                .join(dbnum.to_string())
        });

    Ok(ModelHistoryReplayValidationRequest {
        project_name,
        dbnum,
        source_db_file: sub
            .get_one::<String>("source-db-file")
            .map(PathBuf::from)
            .expect("required by clap"),
        from_sesno: sub
            .get_one::<u32>("from-sesno")
            .copied()
            .expect("required by clap"),
        to_sesno: sub
            .get_one::<u32>("to-sesno")
            .copied()
            .expect("required by clap"),
        source_parquet_dir: sub
            .get_one::<String>("parquet-dir")
            .map(PathBuf::from)
            .expect("required by clap"),
        current_parquet_dir,
        scene_tree_dir: sub.get_one::<String>("scene-tree-dir").map(PathBuf::from),
        require_scene_tree: sub.get_flag("require-scene-tree"),
    })
}

fn build_missing_mesh_repair_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelMissingMeshRepairRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let report_file = sub
        .get_one::<String>("report-file")
        .map(PathBuf::from)
        .expect("required by clap");

    Ok(ModelMissingMeshRepairRequest {
        project_name,
        dbnum,
        report_file,
        mesh_root: mesh_root_from_matches(sub, db_option_ext),
        limit: sub.get_one::<usize>("limit").copied(),
        dry_run: sub.get_flag("dry-run"),
        retry_bad: sub.get_flag("retry-bad"),
    })
}

fn build_scene_tree_artifact_restore_request(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
) -> anyhow::Result<ModelSceneTreeArtifactRestoreRequest> {
    let project_name = project_name_from_matches(sub, db_option_ext);
    let dbnum = sub
        .get_one::<u32>("dbnum")
        .copied()
        .expect("required by clap");
    let source_scene_tree_dir = sub
        .get_one::<String>("source-scene-tree-dir")
        .map(PathBuf::from)
        .expect("required by clap");
    let target_scene_tree_dir = sub
        .get_one::<String>("target-scene-tree-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_output_dir_for(db_option_ext, &project_name).join("scene_tree"));

    Ok(ModelSceneTreeArtifactRestoreRequest {
        project_name,
        dbnum,
        source_scene_tree_dir,
        target_scene_tree_dir,
        overwrite_tree: sub.get_flag("overwrite-tree"),
        dry_run: sub.get_flag("dry-run"),
    })
}

/// specs/023：export-batch id = `--release-id` 或 `db{dbnum}-s{sesno}`。
fn resolve_export_batch_id(sub: &ArgMatches) -> anyhow::Result<String> {
    if let Some(id) = sub.get_one::<String>("release-id") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    match (
        sub.get_one::<u32>("dbnum").copied(),
        sub.get_one::<u32>("sesno").copied(),
    ) {
        (Some(dbnum), Some(sesno)) => Ok(legacy_batch_id_for_sesno(dbnum, sesno)),
        _ => anyhow::bail!(
            "require --release-id, or both --dbnum and --sesno (maps to db{{N}}-s{{M}} export batch)"
        ),
    }
}

fn project_name_from_matches(sub: &ArgMatches, db_option_ext: &DbOptionExt) -> String {
    sub.get_one::<String>("project")
        .cloned()
        .unwrap_or_else(|| db_option_ext.inner.project_name.clone())
}

fn runner_state_dir_from_matches(sub: &ArgMatches, db_option_ext: &DbOptionExt) -> PathBuf {
    let project_name = project_name_from_matches(sub, db_option_ext);
    sub.get_one::<String>("state-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            project_output_dir_for(db_option_ext, &project_name)
                .join("model_versions")
                .join("runs")
        })
}

fn ducklake_config_from_matches(
    sub: &ArgMatches,
    db_option_ext: &DbOptionExt,
    project_name: &str,
) -> ModelVersionDuckLakeConfig {
    let project_output_dir = project_output_dir_for(db_option_ext, project_name);
    let default_ducklake =
        ModelVersionDuckLakeConfig::for_project_output_dir(&project_output_dir, project_name);
    let metadata_path = sub
        .get_one::<String>("ducklake-metadata")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_ducklake.metadata_path.clone());
    let data_path = sub
        .get_one::<String>("ducklake-data")
        .map(PathBuf::from)
        .unwrap_or(default_ducklake.data_path);
    ModelVersionDuckLakeConfig::new(metadata_path, data_path)
}

fn mesh_root_from_matches(sub: &ArgMatches, db_option_ext: &DbOptionExt) -> PathBuf {
    sub.get_one::<String>("mesh-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| db_option_ext.inner.get_meshes_path())
}

fn release_quality_from_matches(sub: &ArgMatches) -> anyhow::Result<Option<ModelReleaseQuality>> {
    sub.get_one::<String>("release-quality")
        .map(|value| parse_release_quality(value))
        .transpose()
}

fn parse_release_quality(value: &str) -> anyhow::Result<ModelReleaseQuality> {
    match value.trim().to_ascii_lowercase().as_str() {
        "complete_visual" | "complete" => Ok(ModelReleaseQuality::CompleteVisual),
        "quarantined_visual" | "quarantined" | "quarantine" => {
            Ok(ModelReleaseQuality::QuarantinedVisual)
        }
        "degraded_visual" | "degraded" | "partial" => Ok(ModelReleaseQuality::DegradedVisual),
        "patch_only" | "patch-only" => Ok(ModelReleaseQuality::PatchOnly),
        "non_visual" | "non-visual" => Ok(ModelReleaseQuality::NonVisual),
        _ => anyhow::bail!(
            "invalid release quality '{}'; expected complete_visual, quarantined_visual, degraded_visual, patch_only, or non_visual",
            value
        ),
    }
}

fn validation_flags_from_matches(sub: &ArgMatches) -> Vec<String> {
    sub.get_many::<String>("validation-flag")
        .into_iter()
        .flatten()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn project_output_dir_for(db_option_ext: &DbOptionExt, project_name: &str) -> PathBuf {
    db_option_ext.get_output_root().join(project_name)
}

fn current_db_option_file_arg() -> String {
    std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string())
}

fn parse_metadata_json(sub: &ArgMatches) -> anyhow::Result<Value> {
    match sub.get_one::<String>("metadata-json") {
        Some(raw) => {
            let value: Value = serde_json::from_str(raw).context("parse --metadata-json")?;
            if !value.is_object() {
                anyhow::bail!("--metadata-json must be a JSON object");
            }
            Ok(value)
        }
        None => Ok(serde_json::json!({})),
    }
}

async fn handle_history_command(
    hist: &ArgMatches,
    _db_option_ext: &DbOptionExt,
) -> anyhow::Result<()> {
    // Surreal 连接由 main 在进入 model-version history 前 ensure_surreal_connected。
    match hist.subcommand() {
        Some(("snapshot", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let sesno = *sub.get_one::<u32>("sesno").expect("required");
            let pe_key = sub.get_one::<String>("pe-key").map(String::as_str);
            let refno = parse_history_refno(sub.get_one::<String>("refno"), pe_key)?;
            match aios_core::snapshot_at(refno, sesno, Some(dbnum), pe_key).await {
                Ok(snap) => {
                    if sub.get_flag("json") {
                        println!("{}", serde_json::to_string_pretty(&snap)?);
                    } else {
                        println!(
                            "snapshot pe_key={} requested_sesno={} resolved_sesno={} exact={} exists={} anchored_at={}",
                            snap.pe_key,
                            snap.requested_sesno,
                            snap.resolved_sesno,
                            snap.exact_anchor,
                            snap.exists,
                            snap.anchored_at
                        );
                        if let Some(pe) = &snap.pe {
                            println!("pe={}", serde_json::to_string_pretty(pe)?);
                        }
                    }
                }
                Err(e) => anyhow::bail!("{}", aios_core::format_history_error(&e)),
            }
        }
        Some(("timeline", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required");
            let to_sesno = *sub.get_one::<u32>("to-sesno").expect("required");
            let pe_key = sub.get_one::<String>("pe-key").map(String::as_str);
            let refno = parse_history_refno(sub.get_one::<String>("refno"), pe_key)?;
            match aios_core::timeline_with_pe_key(refno, from_sesno, to_sesno, dbnum, pe_key).await
            {
                Ok(points) => {
                    if sub.get_flag("json") {
                        println!("{}", serde_json::to_string_pretty(&points)?);
                    } else {
                        println!("timeline points={}", points.len());
                        for p in points {
                            println!(
                                "sesno={} changed={} exists={} hash={} at={}",
                                p.sesno,
                                p.changed_from_prev,
                                p.exists,
                                p.content_hash,
                                p.anchored_at
                            );
                        }
                    }
                }
                Err(e) => anyhow::bail!("{}", aios_core::format_history_error(&e)),
            }
        }
        Some(("diff", sub)) => {
            let dbnum = *sub.get_one::<u32>("dbnum").expect("required");
            let from_sesno = *sub.get_one::<u32>("from-sesno").expect("required");
            let to_sesno = *sub.get_one::<u32>("to-sesno").expect("required");
            let pe_key = sub.get_one::<String>("pe-key").map(String::as_str);
            let mut refnos = Vec::new();
            if let Some(csv) = sub.get_one::<String>("refnos") {
                for part in csv.split(',') {
                    let t = part.trim();
                    if t.is_empty() {
                        continue;
                    }
                    refnos.push(parse_history_refno(Some(&t.to_string()), None)?);
                }
            } else if pe_key.is_some() {
                refnos.push(parse_history_refno(None, pe_key)?);
            }
            if refnos.is_empty() {
                anyhow::bail!("--refnos or --pe-key is required");
            }
            match aios_core::diff_range_with_pe_keys(
                &refnos,
                from_sesno,
                to_sesno,
                dbnum,
                pe_key,
            )
            .await
            {
                Ok(rows) => {
                    if sub.get_flag("json") {
                        println!("{}", serde_json::to_string_pretty(&rows)?);
                    } else {
                        for row in rows {
                            println!(
                                "{:?} refno={} changes={}",
                                row.kind,
                                row.refno_u64,
                                row.changes.len()
                            );
                            for c in row.changes {
                                println!(
                                    "  {} old={:?} new={:?}",
                                    c.path, c.old, c.new
                                );
                            }
                        }
                    }
                }
                Err(e) => anyhow::bail!("{}", aios_core::format_history_error(&e)),
            }
        }
        _ => unreachable!("history subcommand_required by clap"),
    }
    Ok(())
}

fn parse_history_refno(
    refno: Option<&String>,
    pe_key: Option<&str>,
) -> anyhow::Result<aios_core::RefnoEnum> {
    use std::str::FromStr;
    if let Some(r) = refno {
        let normalized = r.trim().trim_start_matches('/').replace('\\', "/");
        return aios_core::RefnoEnum::from_str(&normalized)
            .map_err(|e| anyhow::anyhow!("invalid --refno '{r}': {e}"));
    }
    if pe_key.is_some() {
        // 夹具路径：无真实 refno 时用 0 占位，snapshot 走 pe_key_override
        return Ok(aios_core::RefnoEnum::from(aios_core::RefU64(0)));
    }
    anyhow::bail!("--refno or --pe-key is required");
}

fn component_key_filter_from_matches(sub: &ArgMatches) -> anyhow::Result<Option<String>> {
    if let Some(component_key) = sub
        .get_one::<String>("component-key")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(component_key.to_string()));
    }

    let Some(refno_u64) = sub.get_one::<u64>("refno-u64").copied() else {
        return Ok(None);
    };
    let Some(dbnum) = sub.get_one::<u32>("dbnum").copied() else {
        anyhow::bail!("--dbnum is required when --refno-u64 is used");
    };
    Ok(Some(format!("{}:{}", dbnum, refno_u64)))
}
