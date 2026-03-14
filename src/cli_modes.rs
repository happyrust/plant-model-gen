use std::path::{Path, PathBuf};
use std::str::FromStr;

use aios_core::pdms_types::{RefU64, RefnoEnum};
use anyhow::{Context, Result, anyhow};

use aios_core::init_surreal;
use aios_core::{DBType, query_mdb_db_nums};
use aios_database::fast_model::export_glb::GlbExporter;
use aios_database::fast_model::export_gltf::GltfExporter;
use aios_database::fast_model::export_gltf::export_gltf_for_refnos;
use aios_database::fast_model::export_model::export_obj::export_obj_for_refnos;
use aios_database::fast_model::export_instanced_bundle::export_instanced_bundle_for_refnos;
use aios_database::fast_model::export_model::export_obj::ObjExporter;
use aios_database::fast_model::export_model::export_room_instances::{
    RoomComputeValidationCase, RoomComputeValidationFixture,
};
use aios_database::perf_timer::{PerfReport, PerfTimer};
use aios_database::options::DbOptionExt;
// use aios_database::fast_model::export_xkt::XktExporter;
use aios_database::fast_model::model_exporter::{
    CommonExportConfig, ExportStats, GlbExportConfig, GltfExportConfig, ModelExporter,
    ObjExportConfig, XktExportConfig, collect_export_refnos,
};
use aios_database::fast_model::unit_converter::{LengthUnit, UnitConverter};
use std::collections::{BTreeMap, BTreeSet};

/// 统一的导出配置结构体
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// 参考号列表
    pub refnos_str: Vec<String>,
    /// 输出路径（可选）
    pub output_path: Option<String>,
    /// 过滤类型（可选）
    pub filter_nouns: Option<Vec<String>>,
    /// 是否包含子孙节点
    pub include_descendants: bool,
    /// 源单位
    pub source_unit: String,
    /// 目标单位
    pub target_unit: String,
    /// 是否详细输出
    pub verbose: bool,
    /// 是否重新生成 plant mesh
    pub regenerate_plant_mesh: bool,
    /// 数据库编号（用于按 SITE 导出）
    pub dbnum: Option<u32>,
    /// 是否使用基础颜色材质（非 PBR）
    pub use_basic_materials: bool,
    /// 是否运行所有 dbnum（全库导出模式）
    pub run_all_dbnos: bool,
    /// 是否按 SITE 拆分导出
    pub split_by_site: bool,
    /// 是否包含负实体（Neg 类型几何体）
    pub include_negative: bool,
    /// 是否导出 SVG 截面
    pub export_svg: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            refnos_str: Vec::new(),
            output_path: None,
            filter_nouns: None,
            include_descendants: true,
            source_unit: "mm".to_string(),
            target_unit: "mm".to_string(),
            verbose: false,
            regenerate_plant_mesh: false,
            dbnum: None,
            use_basic_materials: false,
            run_all_dbnos: false,
            split_by_site: false,
            include_negative: false,
            export_svg: false,
        }
    }
}

impl ExportConfig {
    /// 创建新的导出配置
    pub fn new(refnos_str: Vec<String>) -> Self {
        Self {
            refnos_str,
            export_svg: false,
            ..Default::default()
        }
    }

    /// 设置输出路径
    pub fn with_output_path(mut self, output_path: Option<String>) -> Self {
        self.output_path = output_path;
        self
    }

    /// 设置过滤类型
    pub fn with_filter_nouns(mut self, filter_nouns: Option<Vec<String>>) -> Self {
        self.filter_nouns = filter_nouns;
        self
    }

    /// 设置是否包含子孙节点
    pub fn with_include_descendants(mut self, include_descendants: bool) -> Self {
        self.include_descendants = include_descendants;
        self
    }

    /// 设置单位转换
    pub fn with_unit_conversion(mut self, source_unit: &str, target_unit: &str) -> Self {
        self.source_unit = source_unit.to_string();
        self.target_unit = target_unit.to_string();
        self
    }

    /// 设置详细输出
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// 设置重新生成 plant mesh
    pub fn with_regenerate_plant_mesh(mut self, regenerate_plant_mesh: bool) -> Self {
        self.regenerate_plant_mesh = regenerate_plant_mesh;
        self
    }

    /// 设置是否默认跑全库
    pub fn with_run_all_dbnos(mut self, run_all_dbnos: bool) -> Self {
        self.run_all_dbnos = run_all_dbnos;
        self
    }

    /// 设置数据库编号
    pub fn with_dbno(mut self, dbnum: Option<u32>) -> Self {
        self.dbnum = dbnum;
        self
    }

    /// 设置是否按 SITE 拆分导出
    pub fn with_split_by_site(mut self, split_by_site: bool) -> Self {
        self.split_by_site = split_by_site;
        self
    }

    /// 从命令行参数构建导出配置（用于全库导出模式）
    pub fn build_for_all_dbnos(
        output_path: Option<String>,
        filter_nouns: Option<Vec<String>>,
        include_descendants: bool,
        source_unit: String,
        target_unit: String,
        verbose: bool,
        regenerate_plant_mesh: bool,
        use_basic_materials: bool,
        split_by_site: bool,
        include_negative: bool,
        export_svg: bool,
    ) -> Self {
        Self {
            refnos_str: vec![],
            output_path,
            filter_nouns,
            include_descendants,
            source_unit,
            target_unit,
            verbose,
            regenerate_plant_mesh,
            dbnum: None,
            use_basic_materials,
            run_all_dbnos: true, // 关键：全库导出
            split_by_site,
            include_negative,
            export_svg,
        }
    }

    /// 从命令行参数构建 XKT 导出配置（用于全库导出模式）
    pub fn build_xkt_for_all_dbnos(
        output_path: Option<String>,
        filter_nouns: Option<Vec<String>>,
        include_descendants: bool,
        source_unit: String,
        target_unit: String,
        verbose: bool,
        regenerate_plant_mesh: bool,
        compress: bool,
        validate: bool,
        skip_mesh: bool,
        db_config: Option<String>,
        split_by_site: bool,
    ) -> Self {
        Self {
            refnos_str: vec![],
            output_path,
            filter_nouns,
            include_descendants,
            source_unit,
            target_unit,
            verbose,
            regenerate_plant_mesh,
            dbnum: None,
            use_basic_materials: false,
            run_all_dbnos: true, // 关键：全库导出
            split_by_site,
            include_negative: false,
            export_svg: false,
        }
    }

    /// 解析参考号
    pub fn parse_refnos(&self) -> Result<Vec<RefnoEnum>> {
        let mut refnos = Vec::new();
        for s in &self.refnos_str {
            let normalized = s.replace('_', "/");
            if let Ok(ref_u64) = RefU64::from_str(&normalized) {
                refnos.push(RefnoEnum::Refno(ref_u64));
            }
        }

        if refnos.is_empty() {
            return Err(anyhow!("无效的参考号"));
        }

        Ok(refnos)
    }

    /// 获取 mesh 目录（自动根据 default_lod 添加 LOD 子目录）
    pub fn get_mesh_dir(&self, db_option_ext: &DbOptionExt) -> PathBuf {
        let base_dir = if let Some(ref path) = db_option_ext.inner.meshes_path {
            PathBuf::from(path)
        } else {
            PathBuf::from("assets/meshes")
        };

        // 根据 default_lod 自动添加 LOD 子目录
        let lod = db_option_ext.inner.mesh_precision.default_lod;
        let lod_dir = base_dir.join(format!("lod_{:?}", lod));

        println!(
            "📂 使用 LOD 目录: {} (LOD 级别: {:?})",
            lod_dir.display(),
            lod
        );

        lod_dir
    }

    /// 打印导出参数
    pub fn print_export_params(&self, mesh_dir: &PathBuf) {
        println!("\n📋 导出参数:");
        println!("   - 参考号: {:?}", self.refnos_str);
        if let Some(ref nouns) = self.filter_nouns {
            println!("   - 过滤类型: {:?}", nouns);
        }
        println!("   - 包含子孙节点: {}", self.include_descendants);
        println!("   - Mesh 目录: {}", mesh_dir.display());
        println!("   - 源单位: {}", self.source_unit);
        println!("   - 目标单位: {}", self.target_unit);
        println!("   - 详细输出: {}", self.verbose);
        println!("   - 基础材质: {}", self.use_basic_materials);
        println!("   - 全库导出: {}", self.run_all_dbnos);
        println!("   - 按 SITE 拆分: {}", self.split_by_site);
    }
}

fn parse_length_unit(unit: &str) -> LengthUnit {
    match unit.to_lowercase().as_str() {
        "mm" => LengthUnit::Millimeter,
        "cm" => LengthUnit::Centimeter,
        "dm" => LengthUnit::Decimeter,
        "m" => LengthUnit::Meter,
        "in" => LengthUnit::Inch,
        "ft" => LengthUnit::Foot,
        "yd" => LengthUnit::Yard,
        _ => LengthUnit::Millimeter,
    }
}

fn normalize_refno_inputs(refnos: &[String]) -> Result<Vec<RefnoEnum>> {
    let mut parsed = Vec::new();
    for refno in refnos {
        let normalized = refno.replace('_', "/");
        let parsed_refno = RefU64::from_str(&normalized)
            .map(RefnoEnum::Refno)
            .map_err(|e| anyhow!("解析参考号失败: {} ({})", refno, e))?;
        parsed.push(parsed_refno);
    }
    Ok(parsed)
}

/// 关闭占用指定端口的进程（避免 file 模式下 RocksDB 排他锁冲突）。
///
/// - macOS/Linux: 通过 `lsof` 查找占用端口的 PID，再用 `kill -9` 强制终止。
/// - Windows: 通过 `netstat` 查找 LISTENING 状态的 PID，再用 `taskkill` 强制终止。
pub fn kill_process_on_port(port: u16) {
    let pids = find_pids_on_port(port);
    if pids.is_empty() {
        return;
    }
    for pid in &pids {
        println!("🔪 关闭占用端口 {} 的进程 (PID={})...", port, pid);
        if kill_pid(pid) {
            println!("   ✅ 进程 {} 已终止", pid);
        }
    }
    // 等待端口释放
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// 查找占用指定端口的进程 PID 列表。
#[cfg(unix)]
pub(crate) fn find_pids_on_port(port: u16) -> Vec<String> {
    // lsof -ti :<port>  → 仅输出 PID，每行一个
    let output = match std::process::Command::new("lsof")
        .args(["-ti", &format!(":{}", port)])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("⚠️  无法执行 lsof: {}", e);
            return vec![];
        }
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 查找占用指定端口的进程 PID 列表。
#[cfg(windows)]
pub(crate) fn find_pids_on_port(port: u16) -> Vec<String> {
    let output = match std::process::Command::new("netstat")
        .args(["-ano"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("⚠️  无法执行 netstat: {}", e);
            return vec![];
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let listen_pattern = format!(":{}", port);
    let mut pids = vec![];
    for line in stdout.lines() {
        if !line.contains(&listen_pattern) || !line.contains("LISTENING") {
            continue;
        }
        if let Some(pid) = line.split_whitespace().last() {
            if !pid.is_empty() && pid != "0" {
                pids.push(pid.to_string());
            }
        }
    }
    pids
}

/// 强制终止指定 PID 的进程，返回是否成功。
#[cfg(unix)]
fn kill_pid(pid: &str) -> bool {
    match std::process::Command::new("kill")
        .args(["-9", pid])
        .output()
    {
        Ok(r) if r.status.success() => true,
        Ok(r) => {
            let stderr = String::from_utf8_lossy(&r.stderr);
            eprintln!("   ⚠️  kill 退出码 {}: {}", r.status, stderr.trim());
            false
        }
        Err(e) => {
            eprintln!("   ⚠️  无法执行 kill: {}", e);
            false
        }
    }
}

/// 强制终止指定 PID 的进程，返回是否成功。
#[cfg(windows)]
fn kill_pid(pid: &str) -> bool {
    match std::process::Command::new("taskkill")
        .args(["/F", "/PID", pid])
        .output()
    {
        Ok(r) if r.status.success() => true,
        Ok(r) => {
            let stderr = String::from_utf8_lossy(&r.stderr);
            eprintln!("   ⚠️  taskkill 退出码 {}: {}", r.status, stderr.trim());
            false
        }
        Err(e) => {
            eprintln!("   ⚠️  无法执行 taskkill: {}", e);
            false
        }
    }
}

/// 连接 SurrealDB（固定输入数据源）。
///
/// ws 模式下会先检测目标端口是否可达：
/// - 已启动 → 直接连接
/// - 未启动 → 使用 `[web_server]` 配置自动拉起 SurrealDB 后台进程，等待就绪后再连接
///
/// file 模式下会先关闭可能占用数据目录的 ws server 进程（RocksDB 排他锁）。
async fn ensure_surreal_connected(db_option_ext: &DbOptionExt) -> Result<()> {
    use aios_core::options::DbConnMode;

    let sdb_cfg = db_option_ext.inner.effective_surrealdb();

    if sdb_cfg.mode == DbConnMode::Ws {
        let ip = if sdb_cfg.ip == "localhost" {
            "127.0.0.1"
        } else {
            &sdb_cfg.ip
        };
        let addr = format!("{}:{}", ip, sdb_cfg.port);
        let is_local_target =
            sdb_cfg.ip == "localhost" || sdb_cfg.ip == "127.0.0.1" || sdb_cfg.ip == "::1";

        // 快速探测端口
        let reachable = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);

        if reachable {
            println!("\n📡 SurrealDB 已在 {} 运行，直接连接...", addr);
        } else if is_local_target {
            let auto_start = db_option_ext.inner.web_server.auto_start_surreal;
            if auto_start {
                println!(
                    "\n⚠️  SurrealDB 未在 {} 运行，尝试自动启动（auto_start_surreal = true）...",
                    addr
                );
                auto_start_surreal(&db_option_ext.inner).await?;
            } else {
                anyhow::bail!(
                    "SurrealDB 未在 {} 运行。请手动启动 SurrealDB，或设置 [web_server] auto_start_surreal = true 以自动启动。\n   \
                    手动启动示例: surreal start --user root --pass root --bind 0.0.0.0:{} rocksdb://<数据路径>",
                    addr,
                    sdb_cfg.port
                );
            }
        } else {
            anyhow::bail!(
                "SurrealDB 远端地址不可达: {}。请先启动远端服务或检查网络/配置。",
                addr
            );
        }
    } else {
        // 嵌入式 file 模式：先关闭可能占用数据目录的 ws server（RocksDB 排他锁）
        let port = sdb_cfg.port;
        kill_process_on_port(port);
        println!("\n📡 连接数据库（SurrealDB 嵌入式模式）...");
    }

    aios_database::fast_model::utils::ensure_surreal_init()
        .await
        .context("初始化 SurrealDB 失败（需要读取 PDMS 输入数据）")?;
    println!("✅ 数据库连接成功");
    Ok(())
}

/// 使用 `surreal start` 自动启动本地 SurrealDB 后台进程。
///
/// 配置优先级：
/// - surreal_bin: 环境变量 SURREAL_BIN > [web_server].surreal_bin > "surreal"
/// - 数据路径: [web_server].surreal_data_path > [surrealdb].path > 默认 db-data/{project}_{port}.rdb
/// - 监听地址: [web_server].surreal_bind（如 0.0.0.0:8020）
///
/// 清理 RocksDB 残留 LOCK 文件（仅在没有 surreal 进程运行时才清理）。
/// 当未启用 kv-rocksdb 时使用此本地实现，跨平台：macOS/Linux 用 pgrep，Windows 用 tasklist。
#[cfg(not(feature = "kv-rocksdb"))]
fn cleanup_stale_rocksdb_lock_local(data_path: &str) {
    let lock_path = std::path::Path::new(data_path).join("LOCK");
    if !lock_path.exists() {
        return;
    }
    #[cfg(unix)]
    let has_surreal_process = std::process::Command::new("pgrep")
        .arg("surreal")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    #[cfg(windows)]
    let has_surreal_process = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq surreal.exe", "/NH"])
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.contains("surreal.exe")
        })
        .unwrap_or(false);
    if has_surreal_process {
        println!("   ⚠️  LOCK 文件存在且有 surreal 进程在运行，跳过清理");
        return;
    }
    match std::fs::remove_file(&lock_path) {
        Ok(()) => println!("   🧹 已清理残留 LOCK 文件: {}", lock_path.display()),
        Err(e) => println!("   ⚠️  无法删除 LOCK 文件: {} ({})", lock_path.display(), e),
    }
}

/// 启动前会检测 RocksDB LOCK 文件：若无 surreal 进程持有则自动清理残留锁，避免崩溃后无法重启。
async fn auto_start_surreal(db_option: &aios_core::options::DbOption) -> Result<()> {
    let sdb_cfg = db_option.effective_surrealdb();
    let ws_cfg = &db_option.web_server;

    let data_path = ws_cfg.effective_data_path(db_option.surrealdb.path.as_deref());
    let db_uri = format!("rocksdb://{}", data_path);
    let bind = ws_cfg.surreal_bind.clone();
    let surreal_bin = std::env::var("SURREAL_BIN").unwrap_or_else(|_| ws_cfg.surreal_bin.clone());

    // 清理残留 LOCK 文件（无 surreal 进程时属于崩溃残留）
    #[cfg(feature = "kv-rocksdb")]
    aios_core::cleanup_stale_rocksdb_lock(
        &data_path,
        std::env::var("AIOS_FORCE_LOCK")
            .map(|v| v == "1")
            .unwrap_or(false),
    );
    #[cfg(not(feature = "kv-rocksdb"))]
    cleanup_stale_rocksdb_lock_local(&data_path);

    println!(
        "🚀 启动 SurrealDB: {} start --bind {} {}",
        surreal_bin, bind, db_uri
    );

    let result = try_start_surreal_process(
        &surreal_bin,
        &sdb_cfg.user,
        &sdb_cfg.password,
        &bind,
        &db_uri,
    )
    .await;

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let err_msg = e.to_string();
            // LOCK 文件冲突：可能在检测和启动之间有残留，再清理一次重试
            if err_msg.contains("LOCK")
                || err_msg.contains("lock file")
                || err_msg.contains("Resource temporarily unavailable")
            {
                println!("⚠️  检测到 LOCK 文件冲突，清理后重试...");
                #[cfg(feature = "kv-rocksdb")]
                aios_core::cleanup_stale_rocksdb_lock(
                    &data_path,
                    std::env::var("AIOS_FORCE_LOCK")
                        .map(|v| v == "1")
                        .unwrap_or(false),
                );
                #[cfg(not(feature = "kv-rocksdb"))]
                cleanup_stale_rocksdb_lock_local(&data_path);
                try_start_surreal_process(
                    &surreal_bin,
                    &sdb_cfg.user,
                    &sdb_cfg.password,
                    &bind,
                    &db_uri,
                )
                .await
            } else {
                Err(e)
            }
        }
    }
}

/// 启动 surreal 进程并等待端口就绪
async fn try_start_surreal_process(
    surreal_bin: &str,
    user: &str,
    password: &str,
    bind: &str,
    db_uri: &str,
) -> Result<()> {
    let mut child = std::process::Command::new(surreal_bin)
        .arg("start")
        .arg("--user")
        .arg(user)
        .arg("--pass")
        .arg(password)
        .arg("--bind")
        .arg(bind)
        .arg("--log")
        .arg("warn")
        .arg(db_uri)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("无法启动 SurrealDB（请确认 '{}' 在 PATH 中）", surreal_bin))?;

    println!("   PID: {}", child.id());

    // 等待端口就绪（最多 60 秒）
    let timeout_secs = 60u64;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        // 检查子进程是否已经退出（启动失败）
        if let Some(status) = child.try_wait().ok().flatten() {
            let stderr_output = child
                .stderr
                .take()
                .map(|mut s| {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut s, &mut buf).ok();
                    buf
                })
                .unwrap_or_default();
            anyhow::bail!(
                "SurrealDB 进程已退出（exit={}）。\nstderr: {}",
                status,
                if stderr_output.is_empty() {
                    "(空)"
                } else {
                    &stderr_output
                }
            );
        }

        if std::time::Instant::now() > deadline {
            anyhow::bail!(
                "SurrealDB 启动超时（{}s），端口 {} 仍未就绪。请手动检查。",
                timeout_secs,
                bind
            );
        }
        if let Ok(Ok(_)) = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::net::TcpStream::connect(bind),
        )
        .await
        {
            println!("✅ SurrealDB 已就绪 ({})", bind);
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

fn report_obj_export_outcome(label: &str, output_file: &str, stats: &ExportStats) -> Result<()> {
    if stats.geometry_count == 0 {
        println!("⚠️  {}跳过导出：无几何可导出 -> {}", label, output_file);
        return Ok(());
    }

    if !Path::new(output_file).exists() {
        anyhow::bail!(
            "OBJ 导出异常：geometry_count={} 但输出文件不存在: {}",
            stats.geometry_count,
            output_file
        );
    }

    println!("✅ {}导出成功: {}", label, output_file);
    Ok(())
}

fn resolve_room_compute_generation_target(
    expected: RefnoEnum,
    noun_upper: &str,
    owner: Option<(RefnoEnum, &str)>,
) -> RefnoEnum {
    if noun_upper == "BRAN" || noun_upper == "HANG" {
        return expected;
    }

    if let Some((owner_refno, owner_noun)) = owner {
        if owner_noun == "BRAN" || owner_noun == "HANG" {
            return owner_refno;
        }
    }

    expected
}

fn build_room_compute_panel_gen_refnos(
    panel_refno: RefnoEnum,
    extra_targets: Vec<RefnoEnum>,
) -> Vec<RefnoEnum> {
    let mut gen_refnos = vec![panel_refno];
    gen_refnos.extend(extra_targets);
    gen_refnos.sort();
    gen_refnos.dedup();
    gen_refnos
}

fn build_room_compute_panel_gen_option(
    db_option_ext: &DbOptionExt,
    manual_db_nums: Option<Vec<u32>>,
) -> DbOptionExt {
    use aios_database::options::MeshFormat;

    let mut gen_opt = db_option_ext.clone();
    gen_opt.inner.gen_model = true;
    gen_opt.inner.gen_mesh = true;
    gen_opt.inner.replace_mesh = Some(true);
    gen_opt.inner.manual_db_nums = manual_db_nums.map(|mut nums| {
        nums.sort_unstable();
        nums.dedup();
        nums
    });

    // 确保导出 GLB 格式，以便生成 inst_relate_aabb
    if !gen_opt.mesh_formats.contains(&MeshFormat::Glb) {
        gen_opt.mesh_formats.push(MeshFormat::Glb);
    }

    gen_opt
}

fn derive_room_compute_panel_dbnums(refnos: &[RefnoEnum]) -> Vec<u32> {
    use aios_database::data_interface::db_meta_manager::db_meta;

    let _ = db_meta().ensure_loaded();
    let mut dbnums: Vec<u32> = refnos
        .iter()
        .filter_map(|r| db_meta().get_dbnum_by_refno(*r))
        .filter(|&dbnum| dbnum > 0)
        .collect();
    dbnums.sort_unstable();
    dbnums.dedup();
    dbnums
}

fn build_room_compute_panel_spatial_index_roots(
    panel_refno: RefnoEnum,
    gen_refnos: &[RefnoEnum],
) -> Vec<RefnoEnum> {
    let mut roots: Vec<RefnoEnum> = gen_refnos
        .iter()
        .copied()
        .filter(|refno| *refno != panel_refno)
        .collect();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        roots.push(panel_refno);
    }
    roots
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn build_room_compute_panel_calc_options()
-> aios_database::fast_model::room_model::RoomComputeOptions {
    aios_database::fast_model::room_model::RoomComputeOptions::default()
        .with_prebuilt_spatial_index()
        .with_surreal_query()
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn build_room_compute_panel_spatial_index_items(
    geom_insts: Vec<aios_core::GeomInstQuery>,
    local_aabb_map: &std::collections::HashMap<String, parry3d::bounding_volume::Aabb>,
    spec_value_map: &std::collections::HashMap<i64, i64>,
) -> Vec<(i64, String, i64, f64, f64, f64, f64, f64, f64)> {
    use std::collections::BTreeMap;

    let mut merged: BTreeMap<i64, (String, i64, f64, f64, f64, f64, f64, f64)> = BTreeMap::new();

    for geom_inst in geom_insts {
        let Some(aabb) = resolve_room_compute_panel_geom_inst_aabb(&geom_inst, local_aabb_map)
            .or_else(|| geom_inst.world_aabb.map(|aabb| aabb.0))
        else {
            continue;
        };
        let id = geom_inst.refno.refno().0 as i64;
        let spec_value = *spec_value_map.get(&id).unwrap_or(&0);
        let noun = if !geom_inst.insts.is_empty() && geom_inst.insts.iter().all(|inst| inst.is_tubi)
        {
            "TUBI".to_string()
        } else {
            "UNKNOWN".to_string()
        };

        merged
            .entry(id)
            .and_modify(|entry| {
                if entry.0 == "UNKNOWN" && noun != "UNKNOWN" {
                    entry.0 = noun.clone();
                }
                if entry.1 == 0 && spec_value != 0 {
                    entry.1 = spec_value;
                }
                entry.2 = entry.2.min(aabb.mins.x as f64);
                entry.3 = entry.3.max(aabb.maxs.x as f64);
                entry.4 = entry.4.min(aabb.mins.y as f64);
                entry.5 = entry.5.max(aabb.maxs.y as f64);
                entry.6 = entry.6.min(aabb.mins.z as f64);
                entry.7 = entry.7.max(aabb.maxs.z as f64);
            })
            .or_insert((
                noun,
                spec_value,
                aabb.mins.x as f64,
                aabb.maxs.x as f64,
                aabb.mins.y as f64,
                aabb.maxs.y as f64,
                aabb.mins.z as f64,
                aabb.maxs.z as f64,
            ));
    }

    merged
        .into_iter()
        .map(
            |(id, (noun, spec_value, minx, maxx, miny, maxy, minz, maxz))| {
                (id, noun, spec_value, minx, maxx, miny, maxy, minz, maxz)
            },
        )
        .collect()
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn resolve_room_compute_panel_geom_inst_aabb(
    geom_inst: &aios_core::GeomInstQuery,
    local_aabb_map: &std::collections::HashMap<String, parry3d::bounding_volume::Aabb>,
) -> Option<parry3d::bounding_volume::Aabb> {
    use aios_core::geometry::csg::UNIT_MESH_SCALE;
    use aios_database::fast_model::shared::aabb_apply_transform;
    use parry3d::bounding_volume::BoundingVolume;

    let mut merged_world_aabb: Option<parry3d::bounding_volume::Aabb> = None;

    for inst in &geom_inst.insts {
        let geo_hash = inst
            .geo_hash
            .strip_prefix("t_")
            .unwrap_or(inst.geo_hash.as_str());
        let Some(local_aabb) = local_aabb_map.get(geo_hash) else {
            continue;
        };
        if local_aabb.extents().magnitude() <= 1e-4 {
            continue;
        }

        let mut final_transform = if geom_inst.has_neg {
            inst.geo_transform.0
        } else {
            geom_inst.world_trans.0 * inst.geo_transform.0
        };
        if inst.unit_flag {
            final_transform.scale /= UNIT_MESH_SCALE;
        }

        let world_aabb = aabb_apply_transform(local_aabb, &final_transform);
        merged_world_aabb = Some(match merged_world_aabb {
            Some(acc) => acc.merged(&world_aabb),
            None => world_aabb,
        });
    }

    merged_world_aabb
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum RoomVerifyCaseStatus {
    Passed,
    MissingPersistedResults,
    ScopeMismatch,
    ExpectationMismatch,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Clone)]
struct RoomVerifyCaseOutcome {
    case_id: String,
    room_number: String,
    panel_refno: String,
    status: RoomVerifyCaseStatus,
    details: Vec<String>,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Default)]
struct RoomPersistedRelations {
    room_panels: BTreeMap<String, BTreeSet<RefnoEnum>>,
    room_components_by_panel: BTreeMap<(String, RefnoEnum), BTreeSet<RefnoEnum>>,
    component_panels_by_room: BTreeMap<String, BTreeMap<RefnoEnum, BTreeSet<RefnoEnum>>>,
    total_room_panel_records: usize,
    total_room_relate_records: usize,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
#[derive(Debug, Default, Clone, Copy)]
struct RoomVerifySummary {
    total_cases: usize,
    passed_cases: usize,
    failed_cases: usize,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
impl RoomVerifySummary {
    fn from_outcomes(outcomes: &[RoomVerifyCaseOutcome]) -> Self {
        let total_cases = outcomes.len();
        let passed_cases = outcomes
            .iter()
            .filter(|outcome| outcome.status == RoomVerifyCaseStatus::Passed)
            .count();
        Self {
            total_cases,
            passed_cases,
            failed_cases: total_cases.saturating_sub(passed_cases),
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn parse_refno_for_verify(raw: &str, field_name: &str) -> Result<RefnoEnum> {
    RefnoEnum::from_str(&raw.replace('_', "/"))
        .map_err(|_| anyhow!("{} 不是有效 refno: {}", field_name, raw))
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
async fn load_room_persisted_relations() -> Result<RoomPersistedRelations> {
    use aios_database::fast_model::export_model::export_room_instances::{
        query_room_panel_relations_for_verify, query_room_relations_for_verify,
    };

    let room_panel_records = query_room_panel_relations_for_verify().await?;
    let room_relate_records = query_room_relations_for_verify().await?;

    let mut persisted = RoomPersistedRelations {
        total_room_panel_records: room_panel_records.len(),
        total_room_relate_records: room_relate_records.len(),
        ..Default::default()
    };

    for record in room_panel_records {
        persisted
            .room_panels
            .entry(record.room_num)
            .or_default()
            .insert(record.panel_refno);
    }

    for record in room_relate_records {
        persisted
            .room_components_by_panel
            .entry((record.room_num.clone(), record.panel_refno))
            .or_default()
            .insert(record.component_refno);
        persisted
            .component_panels_by_room
            .entry(record.room_num)
            .or_default()
            .entry(record.component_refno)
            .or_default()
            .insert(record.panel_refno);
    }

    Ok(persisted)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn verify_room_case(
    case: &RoomComputeValidationCase,
    persisted: &RoomPersistedRelations,
) -> Result<RoomVerifyCaseOutcome> {
    let panel_refno = parse_refno_for_verify(&case.panel_refno, "panel_refno")?;
    let expected_components: Vec<RefnoEnum> = case
        .expected_components
        .iter()
        .map(|component| parse_refno_for_verify(component, "expected_components"))
        .collect::<Result<Vec<_>>>()?;

    let mut details = Vec::new();
    let room_panels = persisted.room_panels.get(&case.room_number);
    let room_components = persisted
        .room_components_by_panel
        .get(&(case.room_number.clone(), panel_refno));

    let status = match (room_panels, room_components) {
        (None, None) => {
            details.push(format!(
                "room_number={} 在 room_panel_relate 与 room_relate 中都没有持久化结果，请先运行 room compute 或扩大 compute 覆盖范围",
                case.room_number
            ));
            RoomVerifyCaseStatus::MissingPersistedResults
        }
        (Some(panels), None) => {
            if !panels.contains(&panel_refno) {
                details.push(format!(
                    "room_number={} 已有持久化面板结果，但不包含期望 panel {}",
                    case.room_number, panel_refno
                ));
                RoomVerifyCaseStatus::ScopeMismatch
            } else {
                details.push(format!(
                    "room_number={} 已包含 panel {}，但缺少对应的 room_relate 持久化结果",
                    case.room_number, panel_refno
                ));
                RoomVerifyCaseStatus::MissingPersistedResults
            }
        }
        (None, Some(_components)) => {
            details.push(format!(
                "room_number={} 在 room_relate 中存在构件结果，但 room_panel_relate 缺少 panel {}，持久化结果不完整",
                case.room_number, panel_refno
            ));
            RoomVerifyCaseStatus::MissingPersistedResults
        }
        (Some(panels), Some(components)) => {
            if !panels.contains(&panel_refno) {
                details.push(format!(
                    "room_number={} 已有其他面板的持久化结果，但不包含期望 panel {}",
                    case.room_number, panel_refno
                ));
                RoomVerifyCaseStatus::ScopeMismatch
            } else {
                let missing_components: Vec<String> = expected_components
                    .iter()
                    .filter(|component| !components.contains(component))
                    .map(|component| component.to_string())
                    .collect();

                if missing_components.is_empty() {
                    details.push(format!(
                        "room_number={} / panel={} 的持久化结果命中 {} 个期望构件",
                        case.room_number,
                        panel_refno,
                        expected_components.len()
                    ));
                    RoomVerifyCaseStatus::Passed
                } else {
                    let stale_panels: Vec<String> = missing_components
                        .iter()
                        .filter_map(|component| {
                            let component_refno = RefnoEnum::from_str(component).ok()?;
                            let panels = persisted
                                .component_panels_by_room
                                .get(&case.room_number)?
                                .get(&component_refno)?;
                            if panels.is_empty() || panels.contains(&panel_refno) {
                                None
                            } else {
                                Some(format!(
                                    "{} 当前挂在其它 panel: {}",
                                    component,
                                    panels
                                        .iter()
                                        .map(|panel| panel.to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ))
                            }
                        })
                        .collect();

                    details.push(format!("缺少期望构件: {}", missing_components.join(", ")));
                    if !stale_panels.is_empty() {
                        details.extend(stale_panels);
                        details.push(
                            "组件虽然出现在同房间其它 panel 上，但当前 room/panel 已有持久化覆盖；按真实期望不匹配处理，而不是 coverage 缺失"
                                .to_string(),
                        );
                        RoomVerifyCaseStatus::ExpectationMismatch
                    } else {
                        RoomVerifyCaseStatus::ExpectationMismatch
                    }
                }
            }
        }
    };

    Ok(RoomVerifyCaseOutcome {
        case_id: case.case_id.clone(),
        room_number: case.room_number.clone(),
        panel_refno: case.panel_refno.clone(),
        status,
        details,
    })
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
fn print_room_verify_report(
    fixture: &RoomComputeValidationFixture,
    outcomes: &[RoomVerifyCaseOutcome],
    summary: RoomVerifySummary,
) {
    println!("\n🔍 房间持久化结果校验");
    println!("==========================================");
    println!("📄 Fixture: {}", fixture.description);

    for outcome in outcomes {
        let status_label = match outcome.status {
            RoomVerifyCaseStatus::Passed => "✅ PASS",
            RoomVerifyCaseStatus::MissingPersistedResults => "❌ PRECONDITION",
            RoomVerifyCaseStatus::ScopeMismatch => "❌ COVERAGE",
            RoomVerifyCaseStatus::ExpectationMismatch => "❌ MISMATCH",
        };
        println!(
            "{} {} room={} panel={}",
            status_label, outcome.case_id, outcome.room_number, outcome.panel_refno
        );
        for detail in &outcome.details {
            println!("   - {}", detail);
        }
    }

    println!("\n📊 汇总:");
    println!("   - 总案例数: {}", summary.total_cases);
    println!("   - 通过: {}", summary.passed_cases);
    println!("   - 失败: {}", summary.failed_cases);

    if summary.failed_cases > 0 {
        let failing_cases = outcomes
            .iter()
            .filter(|outcome| outcome.status != RoomVerifyCaseStatus::Passed)
            .map(|outcome| outcome.case_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("   - 失败 case_id: {}", failing_cases);
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn room_verify_json_mode(input: &Path, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🧪 校验房间持久化结果（只读模式）");
    println!("==========================================");
    println!("   - 输入文件: {}", input.display());
    println!("   - 行为: 只读校验，不重算、不写 room_relate / room_panel_relate");

    let fixture = RoomComputeValidationFixture::load_from_path(input)?;
    if fixture.test_cases.is_empty() {
        anyhow::bail!("验证 fixture 不包含任何 test_cases: {}", input.display());
    }

    ensure_surreal_connected(db_option_ext).await?;
    let persisted = load_room_persisted_relations().await?;

    if persisted.total_room_panel_records == 0 && persisted.total_room_relate_records == 0 {
        anyhow::bail!(
            "未检测到任何持久化房间计算结果（room_panel_relate / room_relate 为空），请先运行 room compute"
        );
    }

    let mut outcomes = Vec::with_capacity(fixture.test_cases.len());
    for case in &fixture.test_cases {
        outcomes.push(verify_room_case(case, &persisted)?);
    }

    let summary = RoomVerifySummary::from_outcomes(&outcomes);
    print_room_verify_report(&fixture, &outcomes, summary);

    if summary.failed_cases > 0 {
        anyhow::bail!(
            "房间验证失败: checked={}, passed={}, failed={}",
            summary.total_cases,
            summary.passed_cases,
            summary.failed_cases
        );
    }

    println!("\n✅ 所有请求案例均已通过持久化结果校验");
    Ok(())
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]
pub async fn room_verify_json_mode(_input: &Path, _db_option_ext: &DbOptionExt) -> Result<()> {
    Err(anyhow!(
        "房间验证需要 sqlite-index 特性，请使用 --features sqlite-index 编译"
    ))
}

#[cfg(test)]
mod room_verify_tests {
    use super::*;

    fn make_case(expected_components: &[&str]) -> RoomComputeValidationCase {
        RoomComputeValidationCase {
            case_id: "case-1".to_string(),
            description: "test".to_string(),
            room_number: "540".to_string(),
            panel_refno: "24381/35798".to_string(),
            expected_components: expected_components
                .iter()
                .map(|item| item.to_string())
                .collect(),
            notes: String::new(),
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    fn make_persisted_relations() -> RoomPersistedRelations {
        let panel = RefnoEnum::from("24381/35798");
        let component = RefnoEnum::from("24381/145019");
        let other_panel = RefnoEnum::from("24381/35799");

        let mut room_panels = BTreeMap::new();
        room_panels.insert("540".to_string(), BTreeSet::from([panel]));

        let mut room_components_by_panel = BTreeMap::new();
        room_components_by_panel.insert(("540".to_string(), panel), BTreeSet::from([component]));

        let mut component_panels_by_room = BTreeMap::new();
        component_panels_by_room.insert(
            "540".to_string(),
            BTreeMap::from([(component, BTreeSet::from([other_panel]))]),
        );

        RoomPersistedRelations {
            room_panels,
            room_components_by_panel,
            component_panels_by_room,
            total_room_panel_records: 1,
            total_room_relate_records: 1,
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    #[test]
    fn verify_room_case_passes_with_matching_persisted_data() {
        let mut persisted = make_persisted_relations();
        let panel = RefnoEnum::from("24381/35798");
        let component = RefnoEnum::from("24381/145019");
        persisted.component_panels_by_room.insert(
            "540".to_string(),
            BTreeMap::from([(component, BTreeSet::from([panel]))]),
        );

        let outcome = verify_room_case(&make_case(&["24381/145019"]), &persisted)
            .expect("verification should succeed");

        assert_eq!(outcome.status, RoomVerifyCaseStatus::Passed);
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    #[test]
    fn verify_room_case_reports_missing_persisted_results() {
        let persisted = RoomPersistedRelations::default();

        let outcome = verify_room_case(&make_case(&["24381/145019"]), &persisted)
            .expect("verification should classify missing precompute state");

        assert_eq!(
            outcome.status,
            RoomVerifyCaseStatus::MissingPersistedResults
        );
        assert!(outcome.details[0].contains("请先运行 room compute"));
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    #[test]
    fn verify_room_case_reports_expectation_mismatch_when_component_exists_on_other_panel() {
        let mut persisted = make_persisted_relations();
        let panel = RefnoEnum::from("24381/35798");
        let component = RefnoEnum::from("24381/145019");
        persisted.room_components_by_panel.insert(
            ("540".to_string(), panel),
            BTreeSet::new(),
        );
        persisted.component_panels_by_room.insert(
            "540".to_string(),
            BTreeMap::from([(component, BTreeSet::from([RefnoEnum::from("24381/35799")]))]),
        );

        let outcome = verify_room_case(&make_case(&["24381/145019"]), &persisted)
            .expect("verification should classify other-panel component as mismatch");

        assert_eq!(outcome.status, RoomVerifyCaseStatus::ExpectationMismatch);
        assert!(
            outcome
                .details
                .iter()
                .any(|detail| detail.contains("当前挂在其它 panel"))
        );
        assert!(
            outcome
                .details
                .iter()
                .any(|detail| detail.contains("按真实期望不匹配处理"))
        );
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    #[test]
    fn verify_room_case_reports_expectation_mismatch_for_missing_component() {
        let mut persisted = make_persisted_relations();
        persisted
            .component_panels_by_room
            .insert("540".to_string(), BTreeMap::new());

        let outcome = verify_room_case(&make_case(&["24381/999999"]), &persisted)
            .expect("verification should classify missing expected component");

        assert_eq!(outcome.status, RoomVerifyCaseStatus::ExpectationMismatch);
        assert!(outcome.details[0].contains("缺少期望构件"));
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
async fn rebuild_room_compute_panel_spatial_index(
    db_option_ext: &DbOptionExt,
    root_refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<()> {
    use aios_core::query_insts;
    use aios_core::{SurrealQueryExt, project_primary_db};
    use aios_database::fast_model::query_provider::query_multi_descendants_with_self;
    use aios_database::fast_model::{EXIST_MESH_GEO_HASHES, preload_mesh_cache};
    use aios_database::spatial_index::SqliteSpatialIndex;
    use aios_database::sqlite_index::SqliteAabbIndex;
    use serde::Deserialize;
    use surrealdb::types::SurrealValue;

    if root_refnos.is_empty() {
        return Ok(());
    }

    let idx_path = SqliteSpatialIndex::default_path();
    if idx_path.exists() {
        std::fs::remove_file(&idx_path)?;
    }
    let idx = SqliteAabbIndex::open(&idx_path)?;
    idx.init_schema()?;

    let mut expanded_refnos = root_refnos.to_vec();
    let descendants = query_multi_descendants_with_self(root_refnos, &[], false).await?;
    expanded_refnos.extend(descendants);
    expanded_refnos.sort();
    expanded_refnos.dedup();

    #[derive(Debug, Deserialize, SurrealValue)]
    struct SpecValueRow {
        refno: RefnoEnum,
        spec_value: Option<i64>,
        owner_refno: Option<RefnoEnum>,
    }

    let spec_value_map = if expanded_refnos.is_empty() {
        std::collections::HashMap::new()
    } else {
        let pe_keys: Vec<String> = expanded_refnos.iter().map(|r| r.to_pe_key()).collect();
        let sql = format!(
            r#"
            SELECT
                id as refno,
                ->inst_relate[0].spec_value as spec_value,
                ->inst_relate[0].owner_refno as owner_refno
            FROM [{}]
            "#,
            pe_keys.join(",")
        );
        let rows: Vec<SpecValueRow> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        let mut direct: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        let mut owners: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for row in rows {
            let id = row.refno.refno().0 as i64;
            let spec_value = row.spec_value.unwrap_or(0);
            direct.insert(id, spec_value);
            if let Some(owner_refno) = row.owner_refno {
                owners.insert(id, owner_refno.refno().0 as i64);
            }
        }

        let mut effective = std::collections::HashMap::new();
        for id in expanded_refnos.iter().map(|r| r.refno().0 as i64) {
            let mut spec_value = *direct.get(&id).unwrap_or(&0);
            if spec_value == 0 {
                if let Some(owner_id) = owners.get(&id) {
                    spec_value = *direct.get(owner_id).unwrap_or(&0);
                }
            }
            effective.insert(id, spec_value);
        }
        effective
    };

    preload_mesh_cache();
    let local_aabb_map: std::collections::HashMap<String, parry3d::bounding_volume::Aabb> =
        EXIST_MESH_GEO_HASHES
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();

    println!("🔍 [DEBUG] 展开后的 refnos 数量: {}", expanded_refnos.len());
    if verbose {
        for refno in &expanded_refnos {
            println!("   - {}", refno);
        }
    }

    let geom_insts = match query_insts(&expanded_refnos, true).await {
        Ok(insts) => insts,
        Err(e) => {
            println!("⚠️ query_insts 失败: {}", e);
            println!("   回退到直接查询 inst_relate_aabb...");

            // 回退方案：直接从 inst_relate_aabb 表查询 aabb
            use aios_core::{SurrealQueryExt, model_primary_db};
            use serde::Deserialize;
            use surrealdb::types::SurrealValue;

            #[derive(Debug, Deserialize, SurrealValue)]
            struct InstRelateAabbQuery {
                #[serde(rename = "in")]
                refno: RefnoEnum,
                aabb_id: Option<aios_core::types::PlantAabb>,
            }

            let refno_strs: Vec<String> = expanded_refnos
                .iter()
                .map(|r| format!("inst_relate_aabb:`{}`", r))
                .collect();
            let sql = format!(
                "SELECT in, aabb_id.d as aabb_id FROM [{}]",
                refno_strs.join(",")
            );

            let aabb_records: Vec<InstRelateAabbQuery> = model_primary_db()
                .query_take(&sql, 0)
                .await
                .unwrap_or_default();

            println!("   查询到 {} 条 inst_relate_aabb 记录", aabb_records.len());

            // 转换为 GeomInstQuery 格式（只填充必要字段）
            aabb_records
                .into_iter()
                .filter_map(|rec| {
                    rec.aabb_id.map(|aabb| aios_core::GeomInstQuery {
                        refno: rec.refno,
                        owner: rec.refno,
                        world_trans: aios_core::PlantTransform::default(),
                        world_aabb: Some(aabb),
                        insts: Vec::new(),
                        has_neg: false,
                    })
                })
                .collect()
        }
    };
    println!(
        "🔍 [DEBUG] query_insts 返回的 GeomInstQuery 数量: {}",
        geom_insts.len()
    );

    for (idx, geom_inst) in geom_insts.iter().enumerate() {
        println!(
            "   [{}] refno={}, insts.len()={}, has_neg={}, world_aabb={:?}",
            idx,
            geom_inst.refno,
            geom_inst.insts.len(),
            geom_inst.has_neg,
            geom_inst.world_aabb.is_some()
        );
        if geom_inst.insts.is_empty() {
            println!("      ⚠️ 该构件没有 inst 记录（inst_relate 可能缺失）");
        }
    }

    let items =
        build_room_compute_panel_spatial_index_items(geom_insts, &local_aabb_map, &spec_value_map);
    if verbose {
        println!(
            "   - 空间索引根节点: {}，展开后节点: {}，本地 AABB 缓存: {}",
            root_refnos.len(),
            expanded_refnos.len(),
            local_aabb_map.len()
        );
    }
    println!("   - 已写入 SQLite 空间索引项: {}", items.len());
    idx.insert_aabbs_with_items_and_spec_values(items)?;

    Ok(())
}

/// --debug-model 模式：直接生成模型，不清理、不强制 FORCE_REPLACE_MESH。
/// 增量补充缺失的 inst_geo/mesh/布尔结果。
pub async fn run_generate_model(
    config: &ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<aios_database::fast_model::gen_model::GenModelResult> {
    println!("\n🔧 --debug-model：开始增量生成几何体数据...");

    ensure_surreal_connected(db_option_ext).await?;

    use aios_database::fast_model::gen_all_geos_data;
    let target_refnos = collect_regen_target_refnos(config).await?;
    let gen_result = gen_all_geos_data(target_refnos, db_option_ext, None, None).await?;
    println!("✅ 模型增量生成完成");
    Ok(gen_result)
}

/// 集中执行 --regen-model 的模型重建逻辑。
/// 在所有导出函数之前调用一次，不再分散到各导出函数内部。
pub async fn run_regen_model(
    config: &ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<aios_database::fast_model::gen_model::GenModelResult> {
    println!("\n🔄 --regen-model：开始重新生成几何体数据...");
    println!("   - 强制开启 replace_mesh、gen_mesh 和 apply_boolean_operation");

    // 1. 设置环境变量
    unsafe {
        std::env::set_var("FORCE_REPLACE_MESH", "true");
    }

    // 2. 构建 override 后的 DbOption（不影响原始配置）
    let mut db_option_override = db_option_ext.clone();
    db_option_override.inner.replace_mesh = Some(true);
    db_option_override.inner.gen_mesh = true;
    db_option_override.inner.apply_boolean_operation = true;

    // 3. 连接 SurrealDB（gen_all_geos_data 需要读取 PE/属性/世界矩阵等输入数据）
    ensure_surreal_connected(db_option_ext).await?;

    // 4. 确定目标 refnos 并执行生成
    use aios_database::fast_model::gen_all_geos_data;
    let target_refnos = collect_regen_target_refnos(config).await?;

    // 使用 SurrealDB 极简清理（单条 DELETE）
    aios_database::fast_model::gen_model::pdms_inst_surreal::pre_cleanup_for_regen_surreal(&target_refnos).await?;

    // 4.1 从目标 refnos 推导 dbnum，覆盖配置文件中的 manual_db_nums
    if !target_refnos.is_empty() && config.dbnum.is_none() {
        use aios_database::data_interface::db_meta_manager::db_meta;
        let _ = db_meta().ensure_loaded();
        let mut derived_dbnums: Vec<u32> = target_refnos
            .iter()
            .filter_map(|r| db_meta().get_dbnum_by_refno(*r))
            .filter(|&d| d > 0)
            .collect();
        derived_dbnums.sort_unstable();
        derived_dbnums.dedup();
        if !derived_dbnums.is_empty() {
            println!("   - 从 refnos 推导 manual_db_nums: {:?}", derived_dbnums);
            db_option_override.inner.manual_db_nums = Some(derived_dbnums);
        }
    }
    let result = gen_all_geos_data(target_refnos, &db_option_override, None, None).await;

    // 5. 清理环境变量（无论成功/失败都执行）
    unsafe {
        std::env::remove_var("FORCE_REPLACE_MESH");
    }

    let gen_result = result?;
    println!("✅ 模型重新生成完成");
    Ok(gen_result)
}

/// 根据 ExportConfig 确定需要 regen 的目标 refno 集合。
/// - 有 dbnum → 查询该 dbnum 下所有 SITE
/// - 有 refnos → 展开子孙节点
/// - 都没有（全库模式）→ 查询所有 dbnum 的 SITE
async fn collect_regen_target_refnos(config: &ExportConfig) -> Result<Vec<RefnoEnum>> {
    if let Some(dbnum) = config.dbnum {
        // 按 dbnum → 查询所有 SITE
        use aios_database::fast_model::query_provider;
        let sites: Vec<RefnoEnum> =
            query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
        if sites.is_empty() {
            anyhow::bail!("dbnum={} 下未找到任何 SITE，无法 regen", dbnum);
        }
        println!(
            "   - regen 目标: dbnum={} 下 {} 个 SITE",
            dbnum,
            sites.len()
        );
        Ok(sites)
    } else if !config.refnos_str.is_empty() {
        // 按 refnos → 展开子孙
        let refnos = config.parse_refnos()?;
        let expanded =
            collect_export_refnos(&refnos, config.include_descendants, None, config.verbose)
                .await?;
        println!("   - regen 目标: {} 个 refno", expanded.len());
        Ok(expanded)
    } else if config.run_all_dbnos {
        // 全库模式 → 查询所有 dbnum 的 SITE
        use aios_database::fast_model::query_provider;
        let dbnos: Vec<u32> = query_mdb_db_nums(None, DBType::DESI).await?;
        let mut all_sites = Vec::new();
        for db in &dbnos {
            let sites: Vec<RefnoEnum> =
                query_provider::query_by_type(&["SITE"], *db as i32, None).await?;
            all_sites.extend(sites);
        }
        println!(
            "   - regen 目标: {} 个 dbnum, 共 {} 个 SITE",
            dbnos.len(),
            all_sites.len()
        );
        Ok(all_sites)
    } else {
        anyhow::bail!("--regen-model 需要指定 refnos、--dbnum 或启用全库模式");
    }
}

/// 导出 OBJ 模型模式
pub async fn export_obj_mode(config: ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🎯 OBJ 导出模式");
    println!("================");

    ensure_surreal_connected(db_option_ext).await?;

    // 如果需要导出 SVG，设置环境变量
    if config.export_svg {
        println!("🎨 启用 SVG 截面导出");
        unsafe {
            std::env::set_var("EXPORT_SVG", "true");
        }
    } else {
        unsafe {
            std::env::remove_var("EXPORT_SVG");
        }
    }

    // 获取 mesh 目录
    let mesh_dir = config.get_mesh_dir(db_option_ext);

    // 打印导出参数
    config.print_export_params(&mesh_dir);

    // 如果未指定 dbnum 且未提供 refnos，但要求全库导出，则在此处理
    if config.run_all_dbnos && config.dbnum.is_none() && config.refnos_str.is_empty() {
        println!("\n🔁 进入全库 OBJ 导出模式 (MDB 所有 dbnum)");
        let dbnos = query_mdb_db_nums(None, DBType::DESI).await?;
        if dbnos.is_empty() {
            println!("⚠️ MDB 未返回任何 dbnum，跳过导出");
            return Ok(());
        }
        for db in dbnos {
            let mut per_db_config = config.clone();
            per_db_config.dbnum = Some(db);
            if let Err(e) = export_obj_mode_for_db(&per_db_config, db_option_ext).await {
                println!("❌ 导出 dbnum={} 失败: {}", db, e);
            }
        }
        println!("\n🎉 全库 OBJ 导出完成");
        return Ok(());
    }

    // 检查是否指定了 dbnum
    if config.dbnum.is_some() {
        export_obj_mode_for_db(&config, db_option_ext).await?;
    } else {
        // refno 导出统一走标准 descendants-aware OBJ 导出链路，避免 debug-model 仅导出根节点本体。
        let refnos = normalize_refno_inputs(&config.refnos_str)?;
        for refno in &refnos {
            let final_output_path = if let Some(ref path) = config.output_path {
                path.clone()
            } else {
                let base_name = get_output_filename_for_refno(*refno).await;
                format!(
                    "{}/{}",
                    db_option_ext.get_project_output_dir().display(),
                    base_name
                )
            };

            println!("\n🔄 导出 {} -> {} ...", refno, final_output_path);
            export_obj_for_refnos(
                &[*refno],
                &mesh_dir,
                &final_output_path,
                config.filter_nouns.as_deref(),
                config.include_descendants,
            )
            .await?;
        }
    }

    println!("\n🎉 导出完成!");
    Ok(())
}

async fn export_obj_mode_for_db(config: &ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    let mesh_dir = config.get_mesh_dir(db_option_ext);
    let dbnum = config
        .dbnum
        .expect("dbnum required in export_obj_mode_for_db");
    println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
    println!("📊 查询该数据库下的所有 SITE...");

    use aios_database::fast_model::query_provider;
    let sites: Vec<RefnoEnum> =
        query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
    println!("   - 找到 {} 个 SITE", sites.len());

    if sites.is_empty() {
        println!("⚠️  未找到任何 SITE，跳过导出");
        return Ok(());
    }

    let exporter = ObjExporter::new();

    // 检查是否按 SITE 拆分（默认合并）
    if config.split_by_site {
        // 拆分模式：每个 SITE 单独导出
        println!("\n📂 拆分模式：每个 SITE 导出为独立文件");
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "obj").await;
            let output_file = format!(
                "{}/{}",
                db_option_ext.get_project_output_dir().display(),
                site_name
            );
            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );
            let export_config = ObjExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::default(),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    // dbnum/SITE 导出：默认仍使用 SurrealDB（全库查询与命名依赖）。
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            match exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                Err(e) => {
                    println!(
                        "❌ [{}/{}] 导出失败: {} - {}",
                        idx + 1,
                        sites.len(),
                        output_file,
                        e
                    );
                }
                Ok(stats) => {
                    let label = format!("[{}/{}] ", idx + 1, sites.len());
                    report_obj_export_outcome(&label, &output_file, &stats)?;
                }
            }
        }
    } else {
        // 默认合并模式：将所有 SITE 合并到一个文件
        println!("\n🔀 合并模式：将所有 SITE 合并到一个文件（默认）");
        let output_file = format!(
            "{}/dbno_{}.obj",
            db_option_ext.get_project_output_dir().display(),
            dbnum
        );
        println!(
            "🔄 导出合并文件: {} (包含 {} 个 SITE)",
            output_file,
            sites.len()
        );

        let export_config = ObjExportConfig {
            common: CommonExportConfig {
                include_descendants: config.include_descendants,
                filter_nouns: config.filter_nouns.clone(),
                verbose: config.verbose,
                unit_converter: UnitConverter::default(),
                use_basic_materials: config.use_basic_materials,
                include_negative: config.include_negative,
                allow_surrealdb: true,
                cache_dir: None,
            },
        };

        // 将所有 SITE 一次性导出
        match exporter
            .export(&sites, &mesh_dir, &output_file, export_config)
            .await
        {
            Err(e) => {
                println!("❌ 导出失败: {} - {}", output_file, e);
            }
            Ok(stats) => {
                report_obj_export_outcome("", &output_file, &stats)?;
            }
        }
    }

    Ok(())
}

/// 导出 GLB 模型模式
pub async fn export_glb_mode(config: ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🎯 GLB 导出模式");
    println!("================");

    ensure_surreal_connected(db_option_ext).await?;

    // 获取 mesh 目录
    let mesh_dir = config.get_mesh_dir(db_option_ext);

    // 打印导出参数
    config.print_export_params(&mesh_dir);

    // 全库导出（无 dbnum 且无 refnos）
    if config.run_all_dbnos && config.dbnum.is_none() && config.refnos_str.is_empty() {
        println!("\n🔁 进入全库 GLB 导出模式 (MDB 所有 dbnum)");
        let dbnos: Vec<u32> = query_mdb_db_nums(None, DBType::DESI).await?;
        if dbnos.is_empty() {
            println!("⚠️ MDB 未返回任何 dbnum，跳过导出");
            return Ok(());
        }
        for db in dbnos {
            let mut per_db_config = config.clone();
            per_db_config.dbnum = Some(db);
            if let Err(e) = export_glb_mode_for_db(&per_db_config, db_option_ext).await {
                println!("❌ 导出 dbnum={} 失败: {}", db, e);
            }
        }
        println!("\n🎉 全库 GLB 导出完成");
        return Ok(());
    }

    // 检查是否指定了 dbnum
    if config.dbnum.is_some() {
        export_glb_mode_for_db(&config, db_option_ext).await?;
    } else {
        // 原有逻辑：按 refnos 导出
        // 解析参考号
        let refnos = config.parse_refnos()?;

        let exporter = GlbExporter::new();
        for refno in &refnos {
            let final_output_path = if let Some(ref path) = config.output_path {
                path.clone()
            } else {
                let base_name = get_output_filename_for_refno(*refno).await;
                // 确保输出到 output/{project_name} 目录
                format!(
                    "{}/{}.glb",
                    db_option_ext.get_project_output_dir().display(),
                    base_name.replace(".obj", "")
                )
            };

            println!("\n🔄 导出 {} -> {} ...", refno, final_output_path);

            let export_config = GlbExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            let _ = GlbExporter::new()
                .export(&[*refno], &mesh_dir, &final_output_path, export_config)
                .await?;

            println!("✅ 导出成功: {}", final_output_path);
        }
    }

    println!("\n🎉 导出完成!");
    Ok(())
}

async fn export_glb_mode_for_db(config: &ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    let mesh_dir = config.get_mesh_dir(db_option_ext);
    let dbnum = config
        .dbnum
        .expect("dbnum required in export_glb_mode_for_db");
    println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
    println!("📊 查询该数据库下的所有 SITE...");

    use aios_database::fast_model::query_provider;
    let sites: Vec<RefnoEnum> =
        query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
    println!("   - 找到 {} 个 SITE", sites.len());

    if sites.is_empty() {
        println!("⚠️  未找到任何 SITE，跳过导出");
        return Ok(());
    }

    let exporter = GlbExporter::new();

    // 检查是否按 SITE 拆分（默认合并）
    if config.split_by_site {
        // 拆分模式：每个 SITE 单独导出
        println!("\n📂 拆分模式：每个 SITE 导出为独立文件");
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "glb").await;
            let output_file = format!(
                "{}/{}",
                db_option_ext.get_project_output_dir().display(),
                site_name
            );
            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );
            let export_config = GlbExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::default(),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            if let Err(e) = exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                println!(
                    "❌ [{}/{}] 导出失败: {} - {}",
                    idx + 1,
                    sites.len(),
                    output_file,
                    e
                );
            } else {
                println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
            }
        }
    } else {
        // 默认合并模式：将所有 SITE 合并到一个文件
        println!("\n🔀 合并模式：将所有 SITE 合并到一个文件（默认）");
        let output_file = format!(
            "{}/dbno_{}.glb",
            db_option_ext.get_project_output_dir().display(),
            dbnum
        );
        println!(
            "🔄 导出合并文件: {} (包含 {} 个 SITE)",
            output_file,
            sites.len()
        );

        let export_config = GlbExportConfig {
            common: CommonExportConfig {
                include_descendants: config.include_descendants,
                filter_nouns: config.filter_nouns.clone(),
                verbose: config.verbose,
                unit_converter: UnitConverter::default(),
                use_basic_materials: config.use_basic_materials,
                include_negative: config.include_negative,
                allow_surrealdb: true,
                cache_dir: None,
            },
        };

        // 将所有 SITE 一次性导出
        if let Err(e) = exporter
            .export(&sites, &mesh_dir, &output_file, export_config)
            .await
        {
            println!("❌ 导出失败: {} - {}", output_file, e);
        } else {
            println!("✅ 导出成功: {}", output_file);
        }
    }

    Ok(())
}

/// 导出 glTF 模型模式
pub async fn export_gltf_mode(config: ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🎯 glTF 导出模式");
    println!("================");

    ensure_surreal_connected(db_option_ext).await?;

    // 获取 mesh 目录
    let mesh_dir = config.get_mesh_dir(db_option_ext);

    // 打印导出参数
    config.print_export_params(&mesh_dir);

    // 全库导出（无 dbnum 且无 refnos）
    if config.run_all_dbnos && config.dbnum.is_none() && config.refnos_str.is_empty() {
        println!("\n🔁 进入全库 GLTF 导出模式 (MDB 所有 dbnum)");
        let dbnos: Vec<u32> = query_mdb_db_nums(None, DBType::DESI).await?;
        if dbnos.is_empty() {
            println!("⚠️ MDB 未返回任何 dbnum，跳过导出");
            return Ok(());
        }
        for db in dbnos {
            let mut per_db_config = config.clone();
            per_db_config.dbnum = Some(db);
            if let Err(e) = export_gltf_mode_for_db(&per_db_config, db_option_ext).await {
                println!("❌ 导出 dbnum={} 失败: {}", db, e);
            }
        }
        println!("\n🎉 全库 GLTF 导出完成");
        return Ok(());
    }

    // 检查是否指定了 dbnum
    if let Some(dbnum) = config.dbnum {
        println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
        println!("📊 查询该数据库下的所有 SITE...");

        use aios_database::fast_model::query_provider;
        let sites: Vec<RefnoEnum> =
            query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
        println!("   - 找到 {} 个 SITE", sites.len());

        if sites.is_empty() {
            println!("⚠️  未找到任何 SITE，跳过导出");
            return Ok(());
        }

        let exporter = GltfExporter::new();
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "gltf").await;
            let output_file = format!(
                "{}/{}",
                db_option_ext.get_project_output_dir().display(),
                site_name
            );

            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );

            let export_config = GltfExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            match exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                Ok(_) => {
                    println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
                }
                Err(e) => {
                    println!(
                        "❌ [{}/{}] 导出失败: {} - {}",
                        idx + 1,
                        sites.len(),
                        output_file,
                        e
                    );
                }
            }
        }
    } else {
        // 原有逻辑：按 refnos 导出
        // 解析参考号
        let refnos = config.parse_refnos()?;

        let exporter = GltfExporter::new();
        for refno in &refnos {
            let final_output_path = if let Some(ref path) = config.output_path {
                path.clone()
            } else {
                let base_name = get_output_filename_for_refno(*refno).await;
                // 确保输出到 output/{project_name} 目录
                format!(
                    "{}/{}.gltf",
                    db_option_ext.get_project_output_dir().display(),
                    base_name.replace(".obj", "")
                )
            };

            println!("\n🔄 导出 {} -> {} ...", refno, final_output_path);

            let export_config = GltfExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            exporter
                .export(&[*refno], &mesh_dir, &final_output_path, export_config)
                .await?;

            println!("✅ 导出成功: {}", final_output_path);
        }
    }

    println!("\n🎉 导出完成!");
    Ok(())
}

async fn export_gltf_mode_for_db(config: &ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    let mesh_dir = config.get_mesh_dir(db_option_ext);
    let dbnum = config
        .dbnum
        .expect("dbnum required in export_gltf_mode_for_db");
    println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
    println!("📊 查询该数据库下的所有 SITE...");

    use aios_database::fast_model::query_provider;
    let sites: Vec<RefnoEnum> =
        query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
    println!("   - 找到 {} 个 SITE", sites.len());

    if sites.is_empty() {
        println!("⚠️  未找到任何 SITE，跳过导出");
        return Ok(());
    }

    let exporter = GltfExporter::new();

    // 检查是否按 SITE 拆分（默认合并）
    if config.split_by_site {
        // 拆分模式：每个 SITE 单独导出
        println!("\n📂 拆分模式：每个 SITE 导出为独立文件");
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "gltf").await;
            let output_file = format!(
                "{}/{}",
                db_option_ext.get_project_output_dir().display(),
                site_name
            );
            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );
            let export_config = GltfExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::default(),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            if let Err(e) = exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                println!(
                    "❌ [{}/{}] 导出失败: {} - {}",
                    idx + 1,
                    sites.len(),
                    output_file,
                    e
                );
            } else {
                println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
            }
        }
    } else {
        // 默认合并模式：将所有 SITE 合并到一个文件
        println!("\n🔀 合并模式：将所有 SITE 合并到一个文件（默认）");
        let output_file = format!(
            "{}/dbno_{}.gltf",
            db_option_ext.get_project_output_dir().display(),
            dbnum
        );
        println!(
            "🔄 导出合并文件: {} (包含 {} 个 SITE)",
            output_file,
            sites.len()
        );

        let export_config = GltfExportConfig {
            common: CommonExportConfig {
                include_descendants: config.include_descendants,
                filter_nouns: config.filter_nouns.clone(),
                verbose: config.verbose,
                unit_converter: UnitConverter::default(),
                use_basic_materials: config.use_basic_materials,
                include_negative: config.include_negative,
                allow_surrealdb: true,
                cache_dir: None,
            },
        };

        // 将所有 SITE 一次性导出
        if let Err(e) = exporter
            .export(&sites, &mesh_dir, &output_file, export_config)
            .await
        {
            println!("❌ 导出失败: {} - {}", output_file, e);
        } else {
            println!("✅ 导出成功: {}", output_file);
        }
    }

    Ok(())
}

/// 获取输出文件名（优先基于 PE.name；失败则回退为 refno）
pub async fn get_output_filename_for_refno(refno: RefnoEnum) -> String {
    use aios_database::fast_model::query_provider;

    // 1. 尝试获取 PE 的 name
    if let Ok(Some(pe)) = query_provider::get_pe(refno).await {
        let name = pe.name;

        // 如果 PE.name 不为空，使用它
        if !name.is_empty() {
            let clean_name = sanitize_filename(&name);
            return format!("{}.obj", clean_name);
        }

        // 如果 PE.name 为空，尝试从 NamedAttrMap 获取 NAME 属性
        if let Ok(attmap) = aios_core::get_named_attmap(refno).await {
            if let Some(attr_name) = attmap.get_as_string("NAME") {
                if !attr_name.is_empty() {
                    let clean_name = sanitize_filename(&attr_name);
                    return format!("{}.obj", clean_name);
                }
            }
        }
    }

    // 2. 如果 name 为空或查询失败，使用 refno
    format!("{}.obj", refno.to_string().replace('/', "_"))
}

/// 获取 SITE 名称用于导出（带 dbnum 前缀）
pub async fn get_site_name_for_export(refno: RefnoEnum, dbnum: u32, extension: &str) -> String {
    use aios_database::fast_model::query_provider;

    // 1. 尝试获取 PE 的 name
    let site_name = if let Ok(Some(pe)) = query_provider::get_pe(refno).await {
        let name = pe.name;

        // 如果 PE.name 不为空，使用它
        if !name.is_empty() {
            sanitize_filename(&name)
        } else {
            // 尝试从 NamedAttrMap 获取 NAME 属性
            if let Ok(attmap) = aios_core::get_named_attmap(refno).await {
                if let Some(attr_name) = attmap.get_as_string("NAME") {
                    if !attr_name.is_empty() {
                        sanitize_filename(&attr_name)
                    } else {
                        refno.to_string().replace('/', "_")
                    }
                } else {
                    refno.to_string().replace('/', "_")
                }
            } else {
                refno.to_string().replace('/', "_")
            }
        }
    } else {
        // 如果查询失败，使用 refno
        refno.to_string().replace('/', "_")
    };

    format!("{}_{}.{}", dbnum, site_name, extension)
}

fn sanitize_filename(name: &str) -> String {
    let mut result = name
        .replace('/', "_")
        .replace('\\', "_")
        .replace(':', "_")
        .replace('*', "_")
        .replace('?', "_")
        .replace('"', "_")
        .replace('<', "_")
        .replace('>', "_")
        .replace('|', "_")
        .replace(' ', "_");

    // 移除开头的斜线（第一个字符如果是 _，说明原来第一个字符是 /，需要去掉）
    if result.starts_with('_') {
        result = result.strip_prefix('_').unwrap_or(&result).to_string();
    }

    result
}

#[cfg(feature = "grpc")]
use clap::ArgMatches;

#[cfg(feature = "grpc")]
/// 启动 GRPC 服务器模式
pub async fn start_grpc_server_mode(
    matches: &ArgMatches,
    _db_option_ext: DbOptionExt,
) -> Result<()> {
    use aios_database::grpc_service::{init_grpc_logging, server::GrpcServerConfig};

    // 初始化日志
    init_grpc_logging()?;

    // 获取端口配置
    let port: u16 = matches
        .get_one::<String>("grpc-port")
        .unwrap()
        .parse()
        .map_err(|_| anyhow!("Invalid port number"))?;

    // 创建服务器配置
    let config = GrpcServerConfig {
        host: "0.0.0.0".to_string(),
        port,
        max_concurrent_tasks: 4,
        enable_reflection: true,
    };

    println!(
        "Starting AIOS Database GRPC Server...\nServer will listen on {}:{}",
        config.host, config.port
    );

    aios_database::grpc_service::server::start_grpc_server_with_config(config).await?;

    Ok(())
}

async fn export_instanced_bundle_mode(
    config: ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use std::sync::Arc;

    println!("\n🎯 Instanced Bundle 导出模式");
    println!("================");

    // 解析参考号
    let refnos: Vec<RefnoEnum> = if config.refnos_str.is_empty() {
        return Err(anyhow!("请指定参考号"));
    } else {
        config
            .refnos_str
            .iter()
            .map(|s| RefU64::from_str(s).map(RefnoEnum::Refno))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("解析参考号失败: {}", e))?
    };

    println!("   - 参考号数量: {}", refnos.len());
    if config.verbose {
        for refno in &refnos {
            println!("      {}", refno);
        }
    }

    // 确定输出目录
    let output_dir = config.output_path.clone().unwrap_or_else(|| {
        let first_refno = refnos[0].to_string().replace('/', "_");
        format!(
            "{}/instanced-bundle/{}",
            db_option_ext.get_project_output_dir().display(),
            first_refno
        )
    });

    println!("   - 输出目录: {}", output_dir);

    // 获取 mesh 目录
    let mesh_dir = PathBuf::from(db_option_ext.get_meshes_path());
    println!("   - Mesh 目录: {}", mesh_dir.display());

    // 执行导出
    export_instanced_bundle_for_refnos(
        &refnos,
        &mesh_dir,
        &PathBuf::from(&output_dir),
        Arc::new(db_option_ext.inner.clone()),
        config.verbose,
    )
    .await?;

    println!("\n✅ Instanced Bundle 导出完成");
    println!("   输出目录: {}", output_dir);

    Ok(())
}

/// 统一的模型导出模式（支持多种格式）
pub async fn export_model_mode(
    format: &str,
    config: ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    match format.to_lowercase().as_str() {
        "obj" => {
            let obj_config = config.with_unit_conversion("mm", "mm");
            export_obj_mode(obj_config, db_option_ext).await
        }
        "glb" => {
            let glb_config = config.with_unit_conversion("mm", "mm");
            export_glb_mode(glb_config, db_option_ext).await
        }
        "gltf" => {
            let gltf_config = config.with_unit_conversion("mm", "mm");
            export_gltf_mode(gltf_config, db_option_ext).await
        }
        "xkt" => {
            return Err(anyhow!("XKT 导出功能已禁用，需要重新启用 gen_model 特性"));
        }
        "instanced-bundle" | "instanced_bundle" => {
            export_instanced_bundle_mode(config, db_option_ext).await
        }
        _ => Err(anyhow!(
            "不支持的导出格式: {}，支持的格式: obj, glb, gltf, xkt, instanced-bundle",
            format
        )),
    }
}

/// 导出所有 inst_relate 实体（Prepack LOD 格式）
pub async fn export_all_relates_mode(
    dbnum: Option<u32>,
    verbose: bool,
    output_override: Option<PathBuf>,
    owner_types: Option<Vec<String>>,
    name_config_path: Option<PathBuf>,
    export_all_lods: bool,
    export_refnos: Option<String>,
    source_unit: String,
    target_unit: String,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::export_model::NameConfig;
    use aios_database::fast_model::export_model::export_prepack_lod::export_all_relates_prepack_lod;
    use std::sync::Arc;

    println!("\n🎯 导出所有 inst_relate 实体模式");
    println!("============================");

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 加载名称配置（如果提供了路径）
    let name_config = if let Some(path) = name_config_path {
        Some(NameConfig::load_from_excel(&path)?)
    } else {
        None
    };

    // 调用导出函数（通过 Deref 访问内部的 DbOption）
    let db_option = Arc::new((**db_option_ext).clone());
    export_all_relates_prepack_lod(
        dbnum,
        verbose,
        output_override,
        owner_types,
        name_config,
        db_option,
        export_all_lods,
        export_refnos,
        source_unit,
        target_unit,
    )
    .await?;

    println!("\n🎉 导出完成！");
    Ok(())
}

pub async fn export_all_parquet_mode(
    dbnum: Option<u32>,
    verbose: bool,
    output_override: Option<PathBuf>,
    owner_types: Option<Vec<String>>,
    name_config_path: Option<PathBuf>,
    export_all_lods: bool,
    export_refnos: Option<String>,
    source_unit: String,
    target_unit: String,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::export_model::NameConfig;
    use aios_database::fast_model::export_model::export_prepack_lod::export_all_relates_prepack_lod_parquet;
    use std::sync::Arc;

    println!("\n🎯 导出所有 inst_relate 实体模式 (Parquet)");
    println!("============================");

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 加载名称配置（如果提供了路径）
    let name_config = if let Some(path) = name_config_path {
        Some(NameConfig::load_from_excel(&path)?)
    } else {
        None
    };

    // 调用导出函数（通过 Deref 访问内部的 DbOption）
    let db_option = Arc::new((**db_option_ext).clone());
    export_all_relates_prepack_lod_parquet(
        dbnum,
        verbose,
        output_override,
        owner_types,
        name_config,
        db_option,
        export_all_lods,
        export_refnos,
        source_unit,
        target_unit,
    )
    .await?;

    println!("\n🎉 导出完成！");
    Ok(())
}

/// 导出指定 dbnum 的实例数据为简化 JSON 格式（含 AABB）
///
/// # 参数
/// - `autorun`: 若为 `true`（默认），缓存缺失时自动生成模型数据；若为 `false`，则询问用户确认
/// - `root_refno`: 若提供，则仅导出该 refno 下的 visible 子孙节点；否则导出整个 dbnum
/// - `from_cache`: 若为 `true`，使用 model cache 导出；若为 `false`（默认），使用 SurrealDB 导出
/// - `detailed`: 若为 `true`，使用详细格式（version 3）；若为 `false`（默认），使用精简格式（version 4）
pub async fn export_dbnum_instances_json_mode(
    dbnum: u32,
    verbose: bool,
    output_override: Option<PathBuf>,
    db_option_ext: &DbOptionExt,
    autorun: bool,
    root_refno: Option<RefnoEnum>,
    from_cache: bool,
    detailed: bool,
) -> Result<()> {
    use aios_database::fast_model::export_model::export_prepack_lod::{
        export_dbnum_instances_json, export_dbnum_instances_json_from_cache,
    };
    use std::sync::Arc;

    println!("\n🎯 导出 dbnum 实例数据为 JSON（含 AABB）");
    println!("====================================");

    // 设置输出目录
    let output_dir =
        output_override.unwrap_or_else(|| db_option_ext.get_project_output_dir().join("instances"));

    if from_cache {
        async fn cache_has_any_tubi(
            cache_dir: &std::path::Path,
            dbnum: u32,
        ) -> anyhow::Result<bool> {
            let cache =
                aios_database::fast_model::instance_cache::InstanceCacheManager::new(cache_dir)
                    .await?;
            let refnos = cache.list_refnos(dbnum);
            for &refno in &refnos {
                if let Some(info) = cache.get_inst_info(dbnum, refno).await {
                    if info.tubi.is_some() {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }

        async fn gen_tubi_into_cache(
            dbnum: u32,
            db_option_ext: &DbOptionExt,
            root_refno: Option<RefnoEnum>,
        ) -> anyhow::Result<()> {
            use aios_core::geometry::{EleGeosInfo, TubiData};
            use aios_core::rs_surreal::geometry_query::PlantTransform;
            use aios_core::shape::pdms_shape::RsVec3;
            use aios_core::types::PlantAabb;
            use aios_core::{SurrealQueryExt, project_primary_db};
            use aios_database::fast_model::gen_model::tree_index_manager::TreeIndexManager;
            use aios_database::fast_model::model_cache::ModelCacheContext;
            use serde::{Deserialize, Serialize};
            use surrealdb::types as surrealdb_types;
            use surrealdb::types::SurrealValue;

            // 方案 B：tubi 导出以 tubi_relate 为准。
            // 这里“只读 SurrealDB + 写 model cache”，把 tubi_relate 的最小必要信息落到 cache：
            // owner_refno, leave_refno, arrive_refno, index/order, world_trans/aabb, start/end。
            ensure_surreal_connected(db_option_ext).await?;

            let Some(ctx) = ModelCacheContext::try_from_db_option(db_option_ext).await? else {
                anyhow::bail!("model cache 上下文不可用，无法写入缓存");
            };

            let branch_refnos: Vec<RefnoEnum> = if let Some(r) = root_refno.filter(|r| r.is_valid())
            {
                let is_branch = TreeIndexManager::with_default_dir(vec![dbnum])
                    .load_index(dbnum)
                    .ok()
                    .and_then(|idx| idx.node_meta(r.refno()))
                    .is_some_and(|m| {
                        let bran = aios_core::tool::db_tool::db1_hash("BRAN");
                        let hang = aios_core::tool::db_tool::db1_hash("HANG");
                        m.noun == bran || m.noun == hang
                    });
                if is_branch {
                    vec![r]
                } else {
                    // 显式指定了 root_refno 但它不是 BRAN/HANG，则不做“全库补齐”，避免意外长耗时。
                    return Ok(());
                }
            } else {
                let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
                let mut v = manager.query_noun_refnos("BRAN", None);
                v.extend(manager.query_noun_refnos("HANG", None));
                v
            };

            if branch_refnos.is_empty() {
                return Ok(());
            }

            #[derive(Serialize, Deserialize, Debug, SurrealValue)]
            struct TubiRelateRow {
                pub owner_refno: RefnoEnum,
                pub leave_refno: RefnoEnum,
                pub arrive_refno: RefnoEnum,
                #[serde(default)]
                pub world_trans: Option<PlantTransform>,
                #[serde(default)]
                pub world_aabb: Option<PlantAabb>,
                #[serde(default)]
                pub start_pt: Option<RsVec3>,
                #[serde(default)]
                pub end_pt: Option<RsVec3>,
                #[serde(default)]
                pub index: Option<i64>,
            }

            let cache_manager = ctx.cache_arc();
            for owner in &branch_refnos {
                let owner_att = aios_core::get_named_attmap(*owner)
                    .await
                    .unwrap_or_default();
                let owner_type = owner_att.get_type_str().to_string();

                // 注意：tubi_relate 的复合 ID 为 [owner_refno, index]；
                // in/out 对应 leave/arrive；refno 导出侧以 leave_refno 为主键。
                let pe_key = owner.to_pe_key();
                let sql = format!(
                    r#"
                    SELECT
                        id[0] as owner_refno,
                        in as leave_refno,
                        out as arrive_refno,
                        world_trans.d as world_trans,
                        aabb.d as world_aabb,
                        start_pt.d as start_pt,
                        end_pt.d as end_pt,
                        id[1] as index
                    FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                    "#
                );
                let rows: Vec<TubiRelateRow> = project_primary_db().query_take(&sql, 0).await?;
                if rows.is_empty() {
                    continue;
                }

                let mut shape_insts = aios_core::geometry::ShapeInstancesData::default();
                for row in rows {
                    let info = EleGeosInfo {
                        refno: row.leave_refno,
                        sesno: owner_att.sesno(),
                        owner_refno: row.owner_refno,
                        owner_type: owner_type.clone(),
                        cata_hash: Some(aios_core::prim_geo::basic::TUBI_GEO_HASH.to_string()),
                        visible: true,
                        aabb: row.world_aabb.map(|a| a.0),
                        world_transform: row.world_trans.unwrap_or_default().0,
                        tubi: Some(TubiData {
                            start_pt: row.start_pt.map(|p| p.0),
                            end_pt: row.end_pt.map(|p| p.0),
                            arrive_refno: Some(row.arrive_refno),
                            index: row.index.and_then(|i| u32::try_from(i).ok()),
                            ..Default::default()
                        }),
                        is_solid: true,
                        ..Default::default()
                    };
                    shape_insts.insert_tubi(row.leave_refno, info);
                }

                cache_manager.insert_from_shape(dbnum, &shape_insts);
            }

            let _ = ctx.cache().close().await;
            Ok(())
        }

        let cache_dir = db_option_ext.get_model_cache_dir();
        let mesh_dir = ExportConfig::default().get_mesh_dir(db_option_ext);
        let mesh_lod_tag = format!("{:?}", db_option_ext.inner.mesh_precision.default_lod);

        // 缓存导出也需要查询数据库获取 noun/name，所以需要初始化 SurrealDB 连接
        println!("🔌 初始化数据库连接...");
        if let Err(e) = ensure_surreal_connected(db_option_ext).await {
            eprintln!("⚠️  数据库连接失败: {}，将继续但 noun/name 可能为空", e);
        } else if verbose {
            println!("✅ 数据库连接成功");
        }

        // 若显式指定了 root_refno（通过 --debug-model），则优先对该 BRAN/HANG 做一次 tubi 刷新，
        // 以便“已有 cache 但 tubi 缺失/不完整”的情况下也能得到正确导出。
        if root_refno.is_some() {
            println!("\n🔄 检测到 root_refno：尝试刷新该 BRAN/HANG 的 tubi...");
            if let Err(e) = gen_tubi_into_cache(dbnum, db_option_ext, root_refno).await {
                eprintln!("⚠️  tubi 刷新失败（将继续直接导出）: {}", e);
            }
        }

        let result = export_dbnum_instances_json_from_cache(
            dbnum,
            &output_dir,
            &cache_dir,
            Some(&mesh_dir),
            Some(mesh_lod_tag.as_str()),
            verbose,
            None,
            detailed,
        )
        .await;
        match result {
            Ok((stats, trans_count, aabb_count)) => {
                // 若 tubi 为空，则对现有 cache 做一次“tubi-only 补齐”，再重试导出。
                if !cache_has_any_tubi(&cache_dir, dbnum).await.unwrap_or(false) {
                    println!(
                        "\n⚠️  检测到 cache.inst_tubi_map 为空：将尝试补齐 BRAN/HANG tubi 并重新导出..."
                    );
                    if let Err(e) = gen_tubi_into_cache(dbnum, db_option_ext, root_refno).await {
                        eprintln!("⚠️  tubi 补齐失败（将继续输出当前导出结果）: {}", e);
                    } else {
                        println!("✅ tubi 补齐完成，重新导出...");
                        if let Ok((stats, trans_count, aabb_count)) =
                            export_dbnum_instances_json_from_cache(
                                dbnum,
                                &output_dir,
                                &cache_dir,
                                Some(&mesh_dir),
                                Some(mesh_lod_tag.as_str()),
                                verbose,
                                None,
                                detailed,
                            )
                            .await
                        {
                            println!("\n🎉 导出完成！（缓存路径）");
                            println!("📊 统计信息:");
                            println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
                            println!("   - 子节点数量: {}", stats.descendant_count);
                            println!("   - 输出文件大小: {} 字节", stats.output_file_size);
                            println!("   - 变换矩阵数量 (trans): {}", trans_count);
                            println!("   - 包围盒数量 (aabb): {}", aabb_count);
                            println!("   - 耗时: {:?}", stats.elapsed_time);
                            return Ok(());
                        }
                    }
                }

                println!("\n🎉 导出完成！（缓存路径）");
                println!("📊 统计信息:");
                println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
                println!("   - 子节点数量: {}", stats.descendant_count);
                println!("   - 输出文件大小: {} 字节", stats.output_file_size);
                println!("   - 变换矩阵数量 (trans): {}", trans_count);
                println!("   - 包围盒数量 (aabb): {}", aabb_count);
                println!("   - 耗时: {:?}", stats.elapsed_time);
                return Ok(());
            }
            Err(e) => {
                // 检测是否是缓存缺失错误，提供自动/交互式生成选项
                let err_msg = e.to_string();
                if err_msg.contains("缓存中未找到") || err_msg.contains("批次数据") {
                    println!("\n⚠️  dbnum={} 尚未生成模型数据（缓存为空）", dbnum);

                    // autorun 模式：自动开始生成；否则询问用户
                    let should_generate = if autorun {
                        println!("🔄 autorun 模式已开启，自动开始生成模型数据...");
                        true
                    } else {
                        println!();
                        print!("是否现在开始生成模型数据？(y/n): ");
                        use std::io::{self, Write};
                        io::stdout().flush().ok();

                        let mut input = String::new();
                        if io::stdin().read_line(&mut input).is_ok() {
                            let answer = input.trim().to_lowercase();
                            answer == "y" || answer == "yes"
                        } else {
                            false
                        }
                    };

                    if should_generate {
                        println!("\n🚀 开始生成 dbnum={} 的模型数据...", dbnum);

                        // 调用模型生成逻辑
                        use aios_database::fast_model::gen_all_geos_data;
                        use aios_database::versioned_db::database::sync_pdms;

                        // 连接数据库（生成需要从 SurrealDB 读取输入数据）
                        ensure_surreal_connected(db_option_ext).await?;

                        // Step 1: 检测 TreeIndex 是否存在，若缺失则通过 gen_tree_only 解析生成
                        let tree_path = db_option_ext
                            .get_project_output_dir()
                            .join("scene_tree")
                            .join(format!("{}.tree", dbnum));
                        if !tree_path.exists() {
                            println!("📂 检测到 TreeIndex 缺失: {}", tree_path.display());
                            println!("🔄 正在通过 PDMS 解析生成 TreeIndex (gen_tree_only 模式)...");

                            let mut parse_option = db_option_ext.inner.clone();
                            parse_option.gen_tree_only = true;
                            parse_option.total_sync = true;
                            parse_option.manual_db_nums = Some(vec![dbnum]);
                            parse_option.save_db = Some(false); // 不写入 SurrealDB

                            if let Err(e) = sync_pdms(&parse_option).await {
                                println!("⚠️  TreeIndex 生成失败: {}", e);
                                println!("   请确保 PDMS 数据库文件存在且可访问");
                                return Err(anyhow!("TreeIndex 生成失败: {}", e));
                            }

                            println!("✅ TreeIndex 生成完成");
                        }

                        // Step 2: 构建生成配置
                        let mut db_option_clone = db_option_ext.inner.clone();
                        db_option_clone.manual_db_nums = Some(vec![dbnum]);
                        db_option_clone.gen_mesh = true;
                        db_option_clone.replace_mesh = Some(true);

                        let mut db_option_ext_override = db_option_ext.clone();
                        db_option_ext_override.inner = db_option_clone;
                        db_option_ext_override.inner.save_db = Some(false); // 不写回 SurrealDB
                        db_option_ext_override.export_instances = false; // 禁用自动导出，由我们的代码单独处理
                        // IndexTree 已默认启用：无需模式开关

                        unsafe {
                            std::env::set_var("FORCE_REPLACE_MESH", "true");
                        }

                        // Step 3: 生成模型（仅写入 model cache）
                        // 捕获错误但继续尝试导出（缓存可能已有部分数据）
                        match gen_all_geos_data(vec![], &db_option_ext_override, None, None).await {
                            Ok(_) => {
                                println!("✅ 模型生成完成");
                            }
                            Err(e) => {
                                eprintln!("⚠️  模型生成过程中出现错误: {}", e);
                                eprintln!("   尝试继续导出已生成的缓存数据...");
                            }
                        }

                        unsafe {
                            std::env::remove_var("FORCE_REPLACE_MESH");
                        }
                        println!("\n🔄 重新尝试导出...");

                        // 重新尝试导出
                        let retry_result = export_dbnum_instances_json_from_cache(
                            dbnum,
                            &output_dir,
                            &cache_dir,
                            Some(&mesh_dir),
                            Some(mesh_lod_tag.as_str()),
                            verbose,
                            None,
                            detailed,
                        )
                        .await;

                        match retry_result {
                            Ok((stats, trans_count, aabb_count)) => {
                                // 与首次导出一致：若 tubi 为空，则尝试补齐后再导出一次。
                                if !cache_has_any_tubi(&cache_dir, dbnum).await.unwrap_or(false) {
                                    println!(
                                        "\n⚠️  检测到 cache.inst_tubi_map 为空：将尝试补齐 BRAN/HANG tubi 并重新导出..."
                                    );
                                    if let Err(e) = gen_tubi_into_cache(
                                        dbnum,
                                        &db_option_ext_override,
                                        root_refno,
                                    )
                                    .await
                                    {
                                        eprintln!(
                                            "⚠️  tubi 补齐失败（将继续输出当前导出结果）: {}",
                                            e
                                        );
                                    } else {
                                        println!("✅ tubi 补齐完成，重新导出...");
                                        if let Ok((stats, trans_count, aabb_count)) =
                                            export_dbnum_instances_json_from_cache(
                                                dbnum,
                                                &output_dir,
                                                &cache_dir,
                                                Some(&mesh_dir),
                                                Some(mesh_lod_tag.as_str()),
                                                verbose,
                                                None,
                                                detailed,
                                            )
                                            .await
                                        {
                                            println!("\n🎉 导出完成！（缓存路径）");
                                            println!("📊 统计信息:");
                                            println!(
                                                "   - BRAN/HANG/EQUI 分组数量: {}",
                                                stats.refno_count
                                            );
                                            println!("   - 子节点数量: {}", stats.descendant_count);
                                            println!(
                                                "   - 输出文件大小: {} 字节",
                                                stats.output_file_size
                                            );
                                            println!("   - 变换矩阵数量 (trans): {}", trans_count);
                                            println!("   - 包围盒数量 (aabb): {}", aabb_count);
                                            println!("   - 耗时: {:?}", stats.elapsed_time);
                                            return Ok(());
                                        }
                                    }
                                }

                                println!("\n🎉 导出完成！（缓存路径）");
                                println!("📊 统计信息:");
                                println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
                                println!("   - 子节点数量: {}", stats.descendant_count);
                                println!("   - 输出文件大小: {} 字节", stats.output_file_size);
                                println!("   - 变换矩阵数量 (trans): {}", trans_count);
                                println!("   - 包围盒数量 (aabb): {}", aabb_count);
                                println!("   - 耗时: {:?}", stats.elapsed_time);
                                return Ok(());
                            }
                            Err(retry_e) => {
                                return Err(retry_e);
                            }
                        }
                    }

                    // 用户拒绝或 autorun=false 时无效输入，给出手动命令建议
                    println!("\n💡 建议：请手动运行以下命令生成模型数据：");
                    println!(
                        "   cargo run --bin aios-database -- --debug-model --dbnum {} --regen-model",
                        dbnum
                    );
                    return Err(anyhow!(
                        "dbnum={} 尚未生成模型数据，请先生成后再导出",
                        dbnum
                    ));
                }
                return Err(e);
            }
        }
    }

    // 连接数据库
    ensure_surreal_connected(db_option_ext).await?;

    // 调用导出函数（SurrealDB 路径，内部已包含增量合并 trans/aabb）
    let db_option = Arc::new((**db_option_ext).clone());
    let stats = export_dbnum_instances_json(
        dbnum,
        &output_dir,
        db_option,
        verbose,
        None, // 使用默认毫米单位
        root_refno,
        detailed,
    )
    .await?;

    // 注：trans/aabb 已在 export_dbnum_instances_json 内部增量合并导出
    // stats.mesh_files_found = trans 总数, stats.mesh_files_missing = aabb 总数
    // stats.node_count = 新增 trans 数, stats.mesh_count = 新增 aabb 数

    println!("\n🎉 导出完成！");
    println!("📊 统计信息:");
    println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
    println!("   - 子节点数量: {}", stats.descendant_count);
    println!("   - 几何引用数量: {}", stats.geometry_count);
    println!("   - 输出文件大小: {} 字节", stats.output_file_size);
    println!(
        "   - 变换矩阵数量 (trans): {} (+{})",
        stats.mesh_files_found, stats.node_count
    );
    println!(
        "   - 包围盒数量 (aabb): {} (+{})",
        stats.mesh_files_missing, stats.mesh_count
    );
    println!("   - 耗时: {:?}", stats.elapsed_time);
    Ok(())
}

/// 导出指定 dbnum 的实例数据为多表 Parquet 格式
///
/// 输出文件：
/// - `instances.parquet`     — 一行一个实例 refno
/// - `geo_instances.parquet` — 一行一个几何引用 (refno × geo_index)
/// - `tubings.parquet`       — 一行一个 TUBI 段
/// - `transforms.parquet`            — 共享表（去重的变换矩阵）
/// - `aabb.parquet`                  — 共享表（去重的包围盒）
/// - `manifest.json`                 — 导出元信息
#[cfg(feature = "parquet-export")]
pub async fn export_dbnum_instances_parquet_mode(
    dbnum: u32,
    verbose: bool,
    output_override: Option<PathBuf>,
    db_option_ext: &DbOptionExt,
    root_refno: Option<RefnoEnum>,
) -> Result<()> {
    use aios_database::fast_model::export_model::export_dbnum_instances_parquet::export_dbnum_instances_parquet;
    use std::sync::Arc;

    println!("\n🎯 导出 dbnum 实例数据为 Parquet（多表）");
    println!("====================================");

    // 设置输出目录（按 dbnum 分目录，避免不同库互相覆盖）
    let base_output_dir =
        output_override.unwrap_or_else(|| db_option_ext.get_project_output_dir().join("parquet"));
    let output_dir = base_output_dir.join(dbnum.to_string());

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 调用导出函数
    let db_option = Arc::new((**db_option_ext).clone());
    let stats = export_dbnum_instances_parquet(
        dbnum,
        &output_dir,
        db_option,
        verbose,
        None, // 使用默认毫米单位
        root_refno,
    )
    .await?;

    println!("\n🎉 Parquet 导出完成！");
    println!("📊 统计信息:");
    println!("   - 实例数量 (instances): {}", stats.instance_count);
    println!(
        "   - 几何引用数量 (geo_instances): {}",
        stats.geo_instance_count
    );
    println!("   - TUBI 数量 (tubings): {}", stats.tubing_count);
    println!("   - 变换矩阵数量 (transforms): {}", stats.transform_count);
    println!("   - 包围盒数量 (aabb): {}", stats.aabb_count);
    println!("   - 总文件大小: {} 字节", stats.total_bytes);
    println!("   - 耗时: {:?}", stats.elapsed);
    println!("   - 输出目录: {}", output_dir.display());

    Ok(())
}

/// 导出指定 dbnum 的 PDMS Tree（TreeIndex + name/noun/children_count）为 Parquet
///
/// 输出目录默认为：output/<project>/scene_tree_parquet/
#[cfg(feature = "parquet-export")]
pub async fn export_pdms_tree_parquet_mode(
    dbnum: u32,
    verbose: bool,
    output_override: Option<PathBuf>,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::export_model::export_pdms_tree_parquet::export_pdms_tree_parquet;

    println!("\n🎯 导出 PDMS Tree 为 Parquet（供前端查询模型树）");
    println!("==============================================");
    println!("   - dbnum: {}", dbnum);

    // 输出目录：默认 output/<project>/scene_tree_parquet
    let output_dir = output_override.unwrap_or_else(|| {
        db_option_ext
            .get_project_output_dir()
            .join("scene_tree_parquet")
    });
    println!("   - 输出目录: {}", output_dir.display());

    println!("📡 连接数据库...");
    if let Err(e) = init_surreal().await {
        println!("⚠️  数据库连接失败，将回退到离线 name 兜底模式：{e}");
    } else {
        println!("✅ 数据库连接成功");
    }

    let stats = export_pdms_tree_parquet(dbnum, &output_dir, verbose).await?;

    println!("\n🎉 PDMS Tree Parquet 导出完成！");
    println!("📊 统计信息:");
    println!("   - 节点数量: {}", stats.node_count);
    println!(
        "   - 输出文件: {}",
        output_dir.join(&stats.file_name).display()
    );
    println!("   - 文件大小: {} 字节", stats.total_bytes);
    println!("   - generated_at: {}", stats.generated_at);

    Ok(())
}

/// 导出 WORL -> SITE 节点列表为 Parquet（替代后端 e3d children 对 WORL 的特判）
///
/// 输出目录默认为：output/<project>/scene_tree_parquet/
#[cfg(feature = "parquet-export")]
pub async fn export_world_sites_parquet_mode(
    verbose: bool,
    output_override: Option<PathBuf>,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::export_model::export_pdms_tree_parquet::export_world_sites_parquet;

    println!("\n🎯 导出 WORL -> SITE 节点列表为 Parquet（供前端查询）");
    println!("==========================================================");

    // 输出目录：默认 output/<project>/scene_tree_parquet
    let output_dir = output_override.unwrap_or_else(|| {
        db_option_ext
            .get_project_output_dir()
            .join("scene_tree_parquet")
    });
    println!("   - 输出目录: {}", output_dir.display());

    println!("📡 连接数据库...");
    if let Err(e) = init_surreal().await {
        println!("⚠️  数据库连接失败，将回退到离线 tree 扫描模式：{e}");
    } else {
        println!("✅ 数据库连接成功");
    }

    let stats = export_world_sites_parquet(&output_dir, verbose).await?;

    println!("\n🎉 WORL->SITE Parquet 导出完成！");
    println!("📊 统计信息:");
    println!("   - world_refno: {}", stats.world_refno);
    println!("   - SITE 数量: {}", stats.site_count);
    println!(
        "   - 输出文件: {}",
        output_dir.join(&stats.file_name).display()
    );
    println!("   - 文件大小: {} 字节", stats.total_bytes);
    println!("   - generated_at: {}", stats.generated_at);

    Ok(())
}

/// 导入 instances.json 到 SQLite 空间索引
#[cfg(feature = "sqlite-index")]
pub fn import_spatial_index_mode(
    json_path: &Path,
    sqlite_path: &Path,
    verbose: bool,
) -> Result<()> {
    use aios_database::sqlite_index::{ImportConfig, SqliteAabbIndex, i64_to_refno_str};

    println!("\n🗃️ 导入 instances.json 到 SQLite 空间索引");
    println!("==========================================");
    println!("   - 输入文件: {}", json_path.display());
    println!("   - 输出文件: {}", sqlite_path.display());

    // 检查输入文件是否存在
    if !json_path.exists() {
        return Err(anyhow!("输入文件不存在: {}", json_path.display()));
    }

    // 确保输出目录存在
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 如果 SQLite 文件已存在，先删除
    if sqlite_path.exists() {
        if verbose {
            println!("   ⚠️ 删除已存在的 SQLite 文件");
        }
        std::fs::remove_file(sqlite_path)?;
    }

    // 创建 SQLite 索引
    let idx = SqliteAabbIndex::open(sqlite_path)?;
    idx.init_schema()?;
    println!("   ✅ SQLite 索引创建成功");

    // 导入配置：EQUI 粗粒度，BRAN/HANG 细粒度
    let config = ImportConfig::default();
    if verbose {
        println!(
            "   配置: EQUI 粗粒度={}, BRAN/HANG 细粒度={}",
            config.equi_coarse, config.bran_fine
        );
    }

    // 执行导入
    let stats = idx.import_from_instances_json(json_path, &config)?;

    println!("\n🎉 导入完成！");
    println!("📊 统计信息:");
    println!("   - EQUI (粗粒度): {}", stats.equi_count);
    println!("   - Children (细粒度): {}", stats.children_count);
    println!("   - Tubings (细粒度): {}", stats.tubings_count);
    println!("   - 总计遍历: {}", stats.total_inserted);
    println!("   - 去重后唯一记录: {}", stats.unique_count);

    // 验证查询
    if verbose {
        let all_aabbs = idx.query_all_aabbs()?;
        println!("\n🔍 验证查询:");
        println!("   查询到 {} 条 AABB 记录", all_aabbs.len());
        if let Some((id, minx, maxx, miny, maxy, minz, maxz)) = all_aabbs.first() {
            let refno = i64_to_refno_str(*id);
            println!(
                "   示例: refno={}, AABB=[{:.1},{:.1}]x[{:.1},{:.1}]x[{:.1},{:.1}]",
                refno, minx, maxx, miny, maxy, minz, maxz
            );
        }
    }

    Ok(())
}

#[cfg(not(feature = "sqlite-index"))]
pub fn import_spatial_index_mode(
    _json_path: &Path,
    _sqlite_path: &Path,
    _verbose: bool,
) -> Result<()> {
    Err(anyhow!(
        "当前二进制未包含 SQLite 空间索引导入能力，请使用默认方式重新构建 aios-database"
    ))
}

// ============ 房间计算 CLI 模式 ============

/// 房间计算配置
#[derive(Debug, Clone)]
pub struct RoomComputeCliConfig {
    /// 房间关键词（可选，为空则使用配置文件中的默认值）
    pub room_keywords: Option<Vec<String>>,
    /// 数据库编号列表（可选，为空则处理所有）
    pub db_nums: Option<Vec<u32>>,
    /// 是否强制重建
    pub force_rebuild: bool,
    /// 是否详细输出
    pub verbose: bool,
}

/// 从数据库构建 AABB 空间索引
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
async fn build_spatial_index_from_db(db_nums: Option<&[u32]>, verbose: bool) -> Result<()> {
    use aios_core::model_primary_db;
    use aios_database::spatial_index::SqliteSpatialIndex;
    use aios_database::sqlite_index::SqliteAabbIndex;
    use std::time::Instant;
    use surrealdb::types::SurrealValue;

    println!("\n🗃️ 构建空间索引");
    println!("==========================================");

    let start_time = Instant::now();
    let idx_path = SqliteSpatialIndex::default_path();

    // 删除旧索引
    if idx_path.exists() {
        std::fs::remove_file(&idx_path)?;
        println!("   ✅ 已删除旧索引文件");
    }

    // 创建索引
    let idx = SqliteAabbIndex::open(&idx_path)?;
    idx.init_schema()?;
    println!("   ✅ 索引文件创建成功: {}", idx_path.display());

    // 查询所有构件的 AABB
    let sql = if db_nums.is_some() {
        "SELECT id, noun, world_aabb FROM pe WHERE world_aabb != NONE AND dbnum IN $dbnums"
            .to_string()
    } else {
        "SELECT id, noun, world_aabb FROM pe WHERE world_aabb != NONE".to_string()
    };

    println!("   🔍 查询构件 AABB...");

    #[derive(serde::Deserialize, SurrealValue)]
    struct AabbRecord {
        id: RefnoEnum,
        noun: String,
        world_aabb: aios_core::types::PlantAabb,
    }

    let mut q = model_primary_db().query(&sql);
    if let Some(nums) = db_nums {
        q = q.bind(("dbnums", nums.to_vec()));
    }
    let records: Vec<AabbRecord> = q.await?.take(0)?;

    println!("   📊 查询到 {} 个构件", records.len());

    // 批量插入
    const BATCH_SIZE: usize = 10000;
    let mut inserted = 0;

    for chunk in records.chunks(BATCH_SIZE) {
        let items: Vec<_> = chunk
            .iter()
            .map(|r| {
                let id = r.id.refno().0 as i64;
                let inner_aabb = &r.world_aabb.0;
                (
                    id,
                    r.noun.clone(),
                    inner_aabb.mins.x as f64,
                    inner_aabb.maxs.x as f64,
                    inner_aabb.mins.y as f64,
                    inner_aabb.maxs.y as f64,
                    inner_aabb.mins.z as f64,
                    inner_aabb.maxs.z as f64,
                )
            })
            .collect();

        idx.insert_aabbs_with_items(items)?;
        inserted += chunk.len();

        if verbose {
            println!("   ⏳ 已插入 {}/{} 个构件", inserted, records.len());
        }
    }

    let duration = start_time.elapsed();
    println!("   ✅ 空间索引构建完成");
    println!("   📊 总计: {} 个构件", inserted);
    println!("   ⏱️  耗时: {:.2}s", duration.as_secs_f64());

    Ok(())
}

/// 从 inst_relate_aabb 普通表构建 AABB 空间索引
///
/// 与 `build_spatial_index_from_db`（读 pe.world_aabb）不同，
/// 此函数读取 `gen_all_geos_data` 实际写入的 `inst_relate_aabb` + `aabb` 表，
/// 适用于增量生成后的 compute-panel 场景。
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
async fn build_spatial_index_from_inst_relate(verbose: bool) -> Result<()> {
    use aios_database::fast_model::room_model::refresh_sqlite_spatial_index_from_inst_relate_aabb;
    use std::time::Instant;

    println!("\n🗃️ 构建空间索引 (from inst_relate_aabb + booled_aabb override)");
    println!("==========================================");

    let start_time = Instant::now();
    let inserted = refresh_sqlite_spatial_index_from_inst_relate_aabb(None, None).await?;

    let duration = start_time.elapsed();
    println!("   ✅ 空间索引构建完成");
    println!("   📊 总计: {} 个构件", inserted);
    if verbose {
        println!("   ℹ️ 已切换为分块全量刷新路径");
    }
    println!("   ⏱️  耗时: {:.2}s", duration.as_secs_f64());

    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn rebuild_room_spatial_index_mode(verbose: bool) -> Result<()> {
    println!("\n🗃️ 正式重建房间计算 SQLite 空间索引");
    println!("==========================================");

    ensure_surreal_connected(&aios_database::options::get_db_option_ext()).await?;
    build_spatial_index_from_inst_relate(verbose).await
}

/// 房间计算 CLI 模式
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn room_compute_mode(
    room_keywords: Option<Vec<String>>,
    db_nums: Option<Vec<u32>>,
    refno_root: Option<RefnoEnum>,
    gen_panels_mesh: bool,
    report_json: Option<PathBuf>,
    verbose: bool,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::{RoomBuildStats, build_room_relations};
    use std::time::Instant;

    #[cfg(feature = "profile")]
    let _trace_path =
        aios_database::profiling::init_chrome_tracing_for_db_option(db_option_ext, "room_compute");
    #[cfg(feature = "profile")]
    let _root_span = tracing::info_span!("room_compute_mode").entered();

    println!("\n🏠 房间计算模式");
    println!("==========================================");

    let start_time = Instant::now();
    let mut perf_timer = PerfTimer::new("room_compute");
    perf_timer.mark("parse_cli_args");

    let keywords = room_keywords
        .clone()
        .unwrap_or_else(|| db_option_ext.get_room_key_word());
    println!("   - 房间关键词: {:?}", keywords);

    if let Some(ref nums) = db_nums {
        println!("   - 数据库编号: {:?}", nums);
    } else {
        println!("   - 数据库编号: 全部");
    }
    if let Some(ref root) = refno_root {
        println!("   - refno 子树根: {}", root);
    }
    println!(
        "   - 预生成面板模型: {}",
        if gen_panels_mesh {
            "是"
        } else {
            "否（使用 --gen-panels-mesh 启用）"
        }
    );
    if let Some(path) = &report_json {
        println!("   - JSON 报告: {}", path.display());
    }

    println!("\n📡 初始化数据库连接...");
    perf_timer.mark("init_surreal");
    init_surreal().await?;

    // 前置检查：inst_relate_aabb 是否有数据
    {
        println!("   🔍 检查 inst_relate_aabb 表状态...");
        perf_timer.mark("check_inst_relate_aabb");

        // 方法1: 直接 count，不返回 record 字段，避免 SurrealDB 3.x "Expected any, got record" 问题
        let has_data = match aios_core::SUL_DB
            .query("SELECT count() as cnt FROM inst_relate_aabb GROUP ALL")
            .await
        {
            Ok(mut resp) => {
                let rows: Result<Vec<serde_json::Value>, _> = resp.take(0);
                match rows {
                    Ok(vals) if !vals.is_empty() => {
                        let cnt = vals[0].get("cnt").and_then(|v| v.as_u64()).unwrap_or(0);
                        println!("   📊 inst_relate_aabb 记录数: {}", cnt);
                        cnt > 0
                    }
                    Ok(_) => {
                        println!("   ⚠️  count 查询返回空结果");
                        false
                    }
                    Err(e) => {
                        // count 查询也失败，回退用不含 record 的探测
                        println!(
                            "   ⚠️  count GROUP ALL 反序列化失败: {}，尝试备选探测...",
                            e
                        );
                        match aios_core::SUL_DB
                            .query("SELECT record::id(refno) as rid FROM inst_relate_aabb LIMIT 1")
                            .await
                        {
                            Ok(mut resp2) => {
                                let probe: Result<Vec<serde_json::Value>, _> = resp2.take(0);
                                match probe {
                                    Ok(rows) if !rows.is_empty() => {
                                        println!("   ✅ 备选查询确认 inst_relate_aabb 有数据");
                                        true
                                    }
                                    _ => false,
                                }
                            }
                            Err(_) => false,
                        }
                    }
                }
            }
            Err(e) => {
                anyhow::bail!("inst_relate_aabb 查询失败: {}\n请检查数据库连接", e);
            }
        };

        if !has_data {
            anyhow::bail!(
                "inst_relate_aabb 表为空，请先执行模型生成（--debug-model / --regen-model）"
            );
        }
    }

    // 注意：不再调用 build_spatial_index_from_db（从 pe.world_aabb 构建，当前 pe 表无此字段）。
    // build_room_relations 内部的 ensure_spatial_index_ready 会从 inst_relate_aabb 表构建空间索引。

    println!("\n🔄 开始构建房间关系...");
    perf_timer.mark("build_room_relations");

    let stats = build_room_relations(&db_option_ext.inner, db_nums.as_deref(), refno_root).await?;

    let duration = start_time.elapsed();
    perf_timer.mark("print_summary");

    println!("\n🎉 房间计算完成！");
    println!("==========================================");
    println!("📊 统计信息:");
    println!("   - 处理房间数: {}", stats.total_rooms);
    println!("   - 处理面板数: {}", stats.total_panels);
    println!("   - 处理构件数: {}", stats.total_components);
    println!("   - 构建耗时: {}ms", stats.build_time_ms);
    println!("   - 缓存命中率: {:.2}%", stats.cache_hit_rate * 100.0);
    println!("   - 内存使用: {:.2}MB", stats.memory_usage_mb);
    println!("   - 总耗时: {:.2}s", duration.as_secs_f64());

    let report = RoomComputeCliReport::for_full_compute(
        &mut perf_timer,
        stats,
        &duration,
        room_keywords.as_deref(),
        db_nums.as_deref(),
        refno_root,
        gen_panels_mesh,
        &report_json,
    );
    print_room_compute_stage_summary(&report);
    maybe_write_room_compute_report(&report_json, &report)?;

    Ok(())
}

/// 指定单个面板 refno 执行房间计算
///
/// 自动生成所需模型：
/// - panel refno 本身会被加入增量生成列表
/// - expect-refnos 会检查 owner noun，若为 BRAN/HANG 则切换到生成 owner 的模型
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn room_compute_panel_mode(
    panel_refno_str: &str,
    expect_refnos: Option<Vec<String>>,
    rebuild_spatial_index: bool,
    report_json: Option<PathBuf>,
    verbose: bool,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::room_model::{
        RoomComputeOptions, cal_room_refnos_with_options, save_room_relate,
    };
    use std::collections::HashSet;
    use std::str::FromStr;
    use std::time::Instant;

    #[cfg(feature = "profile")]
    let _trace_path = aios_database::profiling::init_chrome_tracing_for_db_option(
        db_option_ext,
        "room_compute_panel",
    );

    println!("\n🏠 单面板房间计算模式");
    println!("==========================================");

    let start_time = Instant::now();
    let mut perf_timer = PerfTimer::new("room_compute_panel");
    perf_timer.mark("parse_cli_args");

    let panel_refno = RefnoEnum::from_str(&panel_refno_str.replace('_', "/"))
        .map_err(|_| anyhow!("无效的面板 refno: {}", panel_refno_str))?;
    println!("   - 面板 refno: {}", panel_refno);
    if let Some(ref expected) = expect_refnos {
        println!("   - 期望命中: {:?}", expected);
    }
    println!(
        "   - SQLite 空间索引: {}",
        if rebuild_spatial_index {
            "显式重建局部索引"
        } else {
            "复用现有索引"
        }
    );
    if let Some(path) = &report_json {
        println!("   - JSON 报告: {}", path.display());
    }

    perf_timer.mark("ensure_surreal_connected");
    ensure_surreal_connected(db_option_ext).await?;

    if rebuild_spatial_index {
        perf_timer.mark("prepare_generation_targets");
        // ========== 仅在显式指定时才生成模型并重建局部索引 ==========
        let mut extra_gen_refnos: Vec<RefnoEnum> = Vec::new();
        let mut expected_root_refnos: Vec<RefnoEnum> = Vec::new();

        if let Some(ref expected) = expect_refnos {
            for exp_str in expected {
                let exp = RefnoEnum::from_str(&exp_str.replace('_', "/"))
                    .map_err(|_| anyhow!("无效的期望 refno: {}", exp_str))?;
                expected_root_refnos.push(exp);
                extra_gen_refnos.push(exp);

                let pe = aios_core::rs_surreal::get_pe(exp).await?;
                if let Some(pe) = pe {
                    let noun_upper = pe.noun.to_uppercase();
                    let target = if noun_upper == "BRAN" || noun_upper == "HANG" {
                        println!("   📦 {} (noun={}) 直接加入生成列表", exp, noun_upper);
                        resolve_room_compute_generation_target(exp, &noun_upper, None)
                    } else {
                        let owner = pe.get_owner();
                        let owner_pe = aios_core::rs_surreal::get_pe(owner).await?;
                        if let Some(owner_pe) = owner_pe {
                            let owner_noun = owner_pe.noun.to_uppercase();
                            let resolved = resolve_room_compute_generation_target(
                                exp,
                                &noun_upper,
                                Some((owner, owner_noun.as_str())),
                            );
                            if resolved == owner {
                                println!(
                                    "   📦 {} (noun={}) → 切换到 owner {} (noun={})",
                                    exp, noun_upper, owner, owner_noun
                                );
                                extra_gen_refnos.push(owner);
                            } else {
                                println!(
                                    "   📦 {} (noun={}, owner noun={}) 直接加入",
                                    exp, noun_upper, owner_noun
                                );
                            }
                            resolved
                        } else {
                            exp
                        }
                    };
                    extra_gen_refnos.push(target);
                } else {
                    println!("   ⚠️ {} 未找到 PE 记录，跳过自动生成", exp);
                }
            }
        }

        let gen_refnos = build_room_compute_panel_gen_refnos(panel_refno, extra_gen_refnos);
        let derived_dbnums = derive_room_compute_panel_dbnums(&gen_refnos);
        println!("\n🔨 自动生成模型 ({} 个目标)...", gen_refnos.len());
        for r in &gen_refnos {
            println!("   - {}", r);
        }
        if !derived_dbnums.is_empty() {
            println!("   - 关联数据库编号: {:?}", derived_dbnums);
        }

        use aios_database::fast_model::gen_all_geos_data;
        aios_core::set_debug_model_enabled(true);
        let gen_opt = build_room_compute_panel_gen_option(
            db_option_ext,
            if derived_dbnums.is_empty() {
                None
            } else {
                Some(derived_dbnums.clone())
            },
        );
        perf_timer.mark("generate_panel_models");
        gen_all_geos_data(gen_refnos.clone(), &gen_opt, None, None).await?;

        println!("✅ 模型生成完成");
        println!("\n⚠️  提示：模型已生成，但空间索引未更新。");
        println!("   如需更新空间索引，请运行：");
        println!("   cargo run --bin aios-database -- room rebuild-spatial-index");
    } else {
        println!("\n🗃️ 复用现有 SQLite 空间索引，不执行模型生成与局部索引重建");
    }

    // ========== 执行房间计算（复用刚刚构建的最小 SQLite 空间索引） ==========
    let mesh_dir = db_option_ext.inner.get_meshes_path();
    let options = build_room_compute_panel_calc_options();
    let exclude = HashSet::new();

    println!("\n🔄 计算面板 {} 的房间归属...", panel_refno);
    perf_timer.mark("compute_panel_relations");
    let result = cal_room_refnos_with_options(&mesh_dir, panel_refno, &exclude, options).await?;

    let duration = start_time.elapsed();
    perf_timer.mark("print_summary");

    println!("\n🎉 计算完成！");
    println!("==========================================");
    println!("   - 面板: {}", panel_refno);
    println!("   - 命中构件数: {}", result.len());
    println!("   - 耗时: {:.2}s", duration.as_secs_f64());

    if verbose || result.len() <= 50 {
        for r in &result {
            println!("   - {}", r);
        }
    } else {
        println!("   (构件过多，使用 --verbose 查看全部)");
    }

    // 验证期望构件
    if let Some(ref expected) = expect_refnos {
        println!("\n📋 期望验证:");
        let mut all_pass = true;
        for exp_str in expected {
            let exp = RefnoEnum::from_str(&exp_str.replace('_', "/"))
                .map_err(|_| anyhow!("无效的期望 refno: {}", exp_str))?;
            if result.contains(&exp) {
                println!("  ✅ {} — 命中", exp);
            } else {
                println!("  ❌ {} — 未命中", exp);
                all_pass = false;
            }
        }
        if !all_pass {
            anyhow::bail!("期望验证失败：部分构件未命中");
        }
        println!("  ✅ 全部验证通过");
    }

    if !result.is_empty() {
        perf_timer.mark("save_room_relate");
        save_room_relate(panel_refno, &result, "manual").await?;
        println!("💾 已保存 {} 条房间关系", result.len());
    }

    let within_refnos = result.iter().copied().collect::<Vec<_>>();
    let report = RoomComputeCliReport::for_single_panel(
        &mut perf_timer,
        panel_refno,
        &within_refnos,
        expect_refnos.as_deref(),
        rebuild_spatial_index,
        &duration,
        &report_json,
    );
    print_room_compute_stage_summary(&report);
    maybe_write_room_compute_report(&report_json, &report)?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpatialQueryVerifyResultItem {
    pub refno: String,
    pub noun: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpatialQueryVerifySnapshot {
    pub success: bool,
    pub query_refno: String,
    pub distance_mm: f32,
    pub include_self: bool,
    pub expected_refnos: Vec<String>,
    pub matched_expected_refnos: Vec<String>,
    pub missing_expected_refnos: Vec<String>,
    pub result_count: usize,
    pub results: Vec<SpatialQueryVerifyResultItem>,
}

fn canonicalize_refno_str(refno: &str) -> String {
    refno.trim().replace('_', "/")
}

pub fn build_spatial_query_verify_snapshot(
    query_refno: &str,
    distance_mm: f32,
    include_self: bool,
    expected_refnos: &[String],
    results: Vec<(String, String)>,
) -> SpatialQueryVerifySnapshot {
    let mut normalized_results = results
        .into_iter()
        .map(|(refno, noun)| SpatialQueryVerifyResultItem {
            refno: canonicalize_refno_str(&refno),
            noun,
        })
        .collect::<Vec<_>>();
    normalized_results.sort_by(|a, b| a.refno.cmp(&b.refno).then_with(|| a.noun.cmp(&b.noun)));
    normalized_results.dedup_by(|a, b| a.refno == b.refno && a.noun == b.noun);

    let mut normalized_expected = expected_refnos
        .iter()
        .map(|s| canonicalize_refno_str(s))
        .collect::<Vec<_>>();
    normalized_expected.sort();
    normalized_expected.dedup();

    let result_refnos = normalized_results
        .iter()
        .map(|item| item.refno.clone())
        .collect::<std::collections::BTreeSet<_>>();

    let matched_expected_refnos = normalized_expected
        .iter()
        .filter(|refno| result_refnos.contains(*refno))
        .cloned()
        .collect::<Vec<_>>();
    let missing_expected_refnos = normalized_expected
        .iter()
        .filter(|refno| !result_refnos.contains(*refno))
        .cloned()
        .collect::<Vec<_>>();

    SpatialQueryVerifySnapshot {
        success: missing_expected_refnos.is_empty(),
        query_refno: canonicalize_refno_str(query_refno),
        distance_mm,
        include_self,
        expected_refnos: normalized_expected,
        matched_expected_refnos,
        missing_expected_refnos,
        result_count: normalized_results.len(),
        results: normalized_results,
    }
}

pub fn write_spatial_query_snapshot_file(
    output_path: &Path,
    snapshot: &SpatialQueryVerifySnapshot,
) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create snapshot dir: {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(output_path, text)
        .with_context(|| format!("write verify json: {}", output_path.display()))?;
    Ok(())
}

pub fn verify_spatial_query_snapshot_file(
    expected_path: &Path,
    snapshot: &SpatialQueryVerifySnapshot,
) -> Result<()> {
    let expected_text = std::fs::read_to_string(expected_path)
        .with_context(|| format!("read verify json: {}", expected_path.display()))?;
    let expected_json: serde_json::Value = serde_json::from_str(&expected_text)
        .with_context(|| format!("parse verify json: {}", expected_path.display()))?;
    let actual_json = serde_json::to_value(snapshot)?;

    if expected_json == actual_json {
        return Ok(());
    }

    let actual_path = expected_path.with_extension("actual.json");
    if let Some(parent) = actual_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create actual snapshot dir: {}", parent.display()))?;
    }
    std::fs::write(&actual_path, serde_json::to_string_pretty(snapshot)?)
        .with_context(|| format!("write actual verify json: {}", actual_path.display()))?;

    Err(anyhow!(
        "verify-json mismatch: expected={}, actual={}",
        expected_path.display(),
        actual_path.display()
    ))
}

/// 基于 refno 的 SQLite 空间范围查询，并可用 expect-refnos / verify-json 做回归验证。
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn spatial_query_refno_mode(
    query_refno_str: &str,
    distance_mm: f32,
    include_self: bool,
    build_spatial: bool,
    expect_refnos: Option<Vec<String>>,
    verify_json_path: Option<&Path>,
    write_verify_json_path: Option<&Path>,
    verbose: bool,
) -> Result<()> {
    use aios_database::spatial_index::SqliteSpatialIndex;
    use parry3d::bounding_volume::Aabb;

    if !distance_mm.is_finite() || distance_mm <= 0.0 {
        anyhow::bail!("distance-mm 必须是正数，当前值: {}", distance_mm);
    }

    let query_refno_display = canonicalize_refno_str(query_refno_str);
    let query_refno_id = aios_database::sqlite_index::refno_str_to_i64(&query_refno_display)
        .ok_or_else(|| anyhow!("无效的 refno: {}", query_refno_str))?;
    let query_refno = RefU64(query_refno_id as u64);

    println!("\n🔎 SQLite 空间范围查询验证");
    println!("==========================================");
    println!("   - 查询 refno: {}", query_refno_display);
    println!("   - 距离: {:.3} mm", distance_mm);
    println!("   - 包含自身: {}", if include_self { "是" } else { "否" });
    println!("   - 刷新索引: {}", if build_spatial { "是" } else { "否" });

    if build_spatial {
        println!("\n🗃️ 刷新 SQLite 空间索引...");
        ensure_surreal_connected(&aios_database::options::get_db_option_ext()).await?;
        build_spatial_index_from_inst_relate(verbose).await?;
        println!("✅ spatial_index.sqlite 已刷新");
    }

    let spatial_index = SqliteSpatialIndex::with_default_path()?;
    println!(
        "   - 索引路径: {}",
        SqliteSpatialIndex::default_path().display()
    );

    let target_aabb = spatial_index
        .get_aabb(query_refno)?
        .ok_or_else(|| anyhow!("未找到指定 refno 的 AABB: {}", query_refno_display))?;

    let query_aabb = Aabb::new(
        [
            target_aabb.mins.x - distance_mm,
            target_aabb.mins.y - distance_mm,
            target_aabb.mins.z - distance_mm,
        ]
        .into(),
        [
            target_aabb.maxs.x + distance_mm,
            target_aabb.maxs.y + distance_mm,
            target_aabb.maxs.z + distance_mm,
        ]
        .into(),
    );

    let mut ids = spatial_index.query_intersect(&query_aabb)?;
    ids.sort_by_key(|id| id.0);

    let mut results = Vec::new();
    for id in ids {
        if !include_self && id.0 == query_refno.0 {
            continue;
        }
        let noun = spatial_index
            .get_noun(id)?
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let refno = aios_database::sqlite_index::i64_to_refno_str(id.0 as i64).replace('_', "/");
        results.push((refno, noun));
    }

    let expected_refnos = expect_refnos.unwrap_or_default();
    let snapshot = build_spatial_query_verify_snapshot(
        &query_refno_display,
        distance_mm,
        include_self,
        &expected_refnos,
        results,
    );

    if let Some(path) = write_verify_json_path {
        write_spatial_query_snapshot_file(path, &snapshot)?;
        println!("💾 已写入 verify-json: {}", path.display());
    }

    if let Some(path) = verify_json_path {
        verify_spatial_query_snapshot_file(path, &snapshot)?;
        println!("✅ verify-json 校验通过: {}", path.display());
    }

    println!("\n{}", serde_json::to_string_pretty(&snapshot)?);

    if !snapshot.missing_expected_refnos.is_empty() {
        anyhow::bail!(
            "期望验证失败：缺少 {}",
            snapshot.missing_expected_refnos.join(", ")
        );
    }

    Ok(())
}

/// 清理房间关系数据
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn room_clean_mode(db_option_ext: &DbOptionExt) -> Result<()> {
    use aios_core::SurrealQueryExt;

    println!("\n🗑️ 清理房间关系数据");
    println!("==========================================");

    init_surreal().await?;

    // 查询当前数据量
    let room_count: Vec<usize> = aios_core::SUL_DB
        .query_take("SELECT VALUE count() FROM room_relate GROUP ALL", 0)
        .await
        .unwrap_or_default();
    let panel_count: Vec<usize> = aios_core::SUL_DB
        .query_take("SELECT VALUE count() FROM room_panel_relate GROUP ALL", 0)
        .await
        .unwrap_or_default();

    let rc = room_count.first().copied().unwrap_or(0);
    let pc = panel_count.first().copied().unwrap_or(0);

    if rc == 0 && pc == 0 {
        println!("   数据库中无房间关系数据，无需清理");
        return Ok(());
    }

    println!("   - room_relate 记录数: {}", rc);
    println!("   - room_panel_relate 记录数: {}", pc);

    aios_core::SUL_DB
        .query_response("DELETE room_relate; DELETE room_panel_relate;")
        .await?;

    println!("✅ 清理完成");
    Ok(())
}

/// 房间计算 CLI 模式（无 sqlite-index 特性时的占位实现）
#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]
pub async fn room_compute_mode(
    _room_keywords: Option<Vec<String>>,
    _db_nums: Option<Vec<u32>>,
    _refno_root: Option<RefnoEnum>,
    _gen_panels_mesh: bool,
    _report_json: Option<PathBuf>,
    _verbose: bool,
    _db_option_ext: &DbOptionExt,
) -> Result<()> {
    Err(anyhow!(
        "当前二进制未包含房间计算所需的 SQLite 空间能力，请使用默认方式重新构建 aios-database"
    ))
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]
pub async fn room_compute_panel_mode(
    _panel_refno_str: &str,
    _expect_refnos: Option<Vec<String>>,
    _rebuild_spatial_index: bool,
    _report_json: Option<PathBuf>,
    _verbose: bool,
    _db_option_ext: &DbOptionExt,
) -> Result<()> {
    Err(anyhow!(
        "当前二进制未包含房间计算所需的 SQLite 空间能力，请使用默认方式重新构建 aios-database"
    ))
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]
pub async fn room_clean_mode(_db_option_ext: &DbOptionExt) -> Result<()> {
    Err(anyhow!(
        "当前二进制未包含房间计算所需的 SQLite 空间能力，请使用默认方式重新构建 aios-database"
    ))
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]
pub async fn spatial_query_refno_mode(
    _query_refno_str: &str,
    _distance_mm: f32,
    _include_self: bool,
    _build_spatial: bool,
    _expect_refnos: Option<Vec<String>>,
    _verify_json_path: Option<&Path>,
    _write_verify_json_path: Option<&Path>,
    _verbose: bool,
) -> Result<()> {
    Err(anyhow!(
        "当前二进制未包含 SQLite 空间查询能力，请使用默认方式重新构建 aios-database"
    ))
}

/// 导出房间实例数据 CLI 模式
///
/// 导出房间计算结果为 JSON 格式：
/// - `room_relations.json`: 房间号 → 构件列表的简单映射
/// - `room_geometries.json`: 房间 AABB + 面板几何实例
pub async fn export_room_instances_mode(output_dir: Option<PathBuf>, verbose: bool) -> Result<()> {
    use aios_database::fast_model::export_model::export_room_instances::export_room_instances;

    println!("\n🏠 导出房间实例数据");
    println!("====================================");

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 设置输出目录
    let output_path = output_dir.unwrap_or_else(|| PathBuf::from("output/room_instances"));

    println!("📁 输出目录: {}", output_path.display());

    // 调用导出函数
    let (relations_stats, geometries_stats) = export_room_instances(&output_path, verbose).await?;

    println!("\n🎉 导出完成！");
    println!("📊 统计信息:");
    println!("   - room_relations.json:");
    println!("     - 房间数: {}", relations_stats.total_rooms);
    println!("     - 构件数: {}", relations_stats.total_components);
    println!("     - 耗时: {} ms", relations_stats.export_time_ms);
    println!("   - room_geometries.json:");
    println!("     - 房间数: {}", geometries_stats.total_rooms);
    println!("     - 面板数: {}", geometries_stats.total_panels);
    println!("     - 耗时: {} ms", geometries_stats.export_time_ms);

    Ok(())
}

#[derive(Debug, serde::Serialize)]
struct RoomComputeCliReport {
    schema_version: u32,
    command: String,
    report_path: Option<String>,
    total_duration_ms: u128,
    stats: serde_json::Value,
    context: serde_json::Value,
    perf: PerfReport,
}

impl RoomComputeCliReport {
    fn for_full_compute(
        timer: &mut PerfTimer,
        stats: aios_database::fast_model::RoomBuildStats,
        duration: &std::time::Duration,
        room_keywords: Option<&[String]>,
        db_nums: Option<&[u32]>,
        refno_root: Option<RefnoEnum>,
        gen_panels_mesh: bool,
        report_json: &Option<PathBuf>,
    ) -> Self {
        let perf = timer.generate_report(serde_json::json!({
            "command": "room compute",
            "report_requested": report_json.is_some(),
        }));
        Self {
            schema_version: 1,
            command: "room compute".to_string(),
            report_path: report_json.as_ref().map(|p| p.display().to_string()),
            total_duration_ms: duration.as_millis(),
            stats: serde_json::json!({
                "total_rooms": stats.total_rooms,
                "total_panels": stats.total_panels,
                "total_components": stats.total_components,
                "build_time_ms": stats.build_time_ms,
                "cache_hit_rate": stats.cache_hit_rate,
                "memory_usage_mb": stats.memory_usage_mb,
                "failed_panels": stats.failed_panels,
                "missing_candidates": stats.missing_candidates,
            }),
            context: serde_json::json!({
                "keywords": room_keywords,
                "db_nums": db_nums,
                "refno_root": refno_root.map(|r| r.to_string()),
                "gen_panels_mesh": gen_panels_mesh,
            }),
            perf,
        }
    }

    fn for_single_panel(
        timer: &mut PerfTimer,
        panel_refno: RefnoEnum,
        result: &[RefnoEnum],
        expect_refnos: Option<&[String]>,
        rebuild_spatial_index: bool,
        duration: &std::time::Duration,
        report_json: &Option<PathBuf>,
    ) -> Self {
        let perf = timer.generate_report(serde_json::json!({
            "command": "room compute-panel",
            "report_requested": report_json.is_some(),
        }));
        Self {
            schema_version: 1,
            command: "room compute-panel".to_string(),
            report_path: report_json.as_ref().map(|p| p.display().to_string()),
            total_duration_ms: duration.as_millis(),
            stats: serde_json::json!({
                "panel_refno": panel_refno.to_string(),
                "hit_refno_count": result.len(),
                "within_refnos": result.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
            }),
            context: serde_json::json!({
                "expect_refnos": expect_refnos,
                "rebuild_spatial_index": rebuild_spatial_index,
            }),
            perf,
        }
    }
}

fn print_room_compute_stage_summary(report: &RoomComputeCliReport) {
    println!("\n⏱️ 阶段耗时:");
    for stage in &report.perf.stages {
        println!(
            "   - {}: {:.1}ms ({:.1}%)",
            stage.name, stage.duration_ms, stage.percentage
        );
    }
    println!("   - total: {}ms", report.total_duration_ms);
}

fn maybe_write_room_compute_report(
    report_json: &Option<PathBuf>,
    report: &RoomComputeCliReport,
) -> Result<()> {
    let Some(path) = report_json else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create room report dir: {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(report)?;
    std::fs::write(path, text)
        .with_context(|| format!("write room report json: {}", path.display()))?;
    println!("💾 已写入房间计算报告: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_room_compute_panel_calc_options, build_room_compute_panel_gen_option,
        build_room_compute_panel_gen_refnos, build_room_compute_panel_spatial_index_roots,
        resolve_room_compute_generation_target,
    };
    use aios_core::RefnoEnum;
    use std::str::FromStr;

    fn refno(s: &str) -> RefnoEnum {
        RefnoEnum::from_str(s).unwrap()
    }

    #[test]
    fn test_resolve_room_compute_generation_target_prefers_bran_owner() {
        let target = resolve_room_compute_generation_target(
            refno("24381/145019"),
            "ELBO",
            Some((refno("24381/145018"), "BRAN")),
        );

        assert_eq!(target, refno("24381/145018"));
    }

    #[test]
    fn test_build_room_compute_panel_gen_refnos_sorts_and_dedups() {
        let refnos = build_room_compute_panel_gen_refnos(
            refno("24381/35798"),
            vec![
                refno("24381/145018"),
                refno("24381/145018"),
                refno("24381/35798"),
            ],
        );

        assert_eq!(refnos, vec![refno("24381/35798"), refno("24381/145018")]);
    }

    #[test]
    fn test_build_room_compute_panel_gen_option_enables_instance_export() {
        let db_option_ext =
            aios_database::options::get_db_option_ext_from_path("db_options/DbOption-mac").unwrap();

        let gen_opt =
            build_room_compute_panel_gen_option(&db_option_ext, Some(vec![7997, 8000, 7997]));

        assert!(!gen_opt.export_instances);
        assert_eq!(gen_opt.inner.replace_mesh, Some(true));
        assert!(gen_opt.inner.gen_model);
        assert!(gen_opt.inner.gen_mesh);
        assert_eq!(gen_opt.inner.manual_db_nums, Some(vec![7997, 8000]));
    }

    #[test]
    fn test_build_room_compute_panel_spatial_index_roots_prefers_non_panel_targets() {
        let roots = build_room_compute_panel_spatial_index_roots(
            refno("24381/35798"),
            &[
                refno("24381/35798"),
                refno("24381/145018"),
                refno("24381/145018"),
            ],
        );

        assert_eq!(roots, vec![refno("24381/145018")]);
    }

    #[test]
    fn test_build_room_compute_panel_calc_options_preserves_prebuilt_spatial_index() {
        let options = build_room_compute_panel_calc_options();

        assert!(!options.refresh_spatial_index_enabled());
        assert!(!options.query_from_cache_enabled());
    }
}
