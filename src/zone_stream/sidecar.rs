//! ZoneStream 的 kv-mem sidecar（ADR-0016 D3）。
//!
//! 每次运行拉起**一个** loopback-only 的外部 surreal 进程，内含三个逻辑数据库：
//! `deps`（本 dbnum 的共享依赖并集，只读）、`slot-a` / `slot-b`（ZONE 工作区，双缓冲）。
//!
//! 几个刻意的选择：
//!
//! - **动态端口 + run-scoped namespace**：两次运行、以及与站点自身的 SurrealDB 之间都不会
//!   撞端口或撞命名空间。站点管理里的 `stop_site_ws_db_for_exclusivity` 是按端口杀进程的，
//!   固定端口迟早会被它误伤。
//! - **只绑 127.0.0.1**：内存工作库不对外，端口是动态的也不该被外部发现。
//! - **崩溃不恢复**：kv-mem 里的东西丢了就是丢了，依据源清单与目标 RocksDB 的 ZONE 检查点
//!   重建（ADR-0016 D3），因此这里不做任何持久化或重连续跑。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::process::{Child, Command};

/// 共享依赖库：一个 dbnum 的全部 ZONE 依赖并集，装载后冻结（ADR-0016 D5）。
pub const DEPS_DB: &str = "deps";
/// ZONE 工作区 A。
pub const SLOT_A_DB: &str = "slot-a";
/// ZONE 工作区 B。
pub const SLOT_B_DB: &str = "slot-b";

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// sidecar 的连接信息。复合读 session 的 route map 以此为准（ADR-0016 D4）。
#[derive(Debug, Clone)]
pub struct SidecarEndpoint {
    pub port: u16,
    /// run-scoped namespace，形如 `zs_<run_id>`。
    pub namespace: String,
    pub user: String,
    pub password: String,
}

impl SidecarEndpoint {
    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }
}

/// 运行中的 kv-mem sidecar。`Drop` 时兜底杀进程，但正常路径应显式 [`Self::shutdown`]。
pub struct ZoneStreamSidecar {
    endpoint: SidecarEndpoint,
    child: Option<Child>,
    pid: u32,
    /// `<runtime_dir>/zone-stream/<run_id>/`：日志与 pid 文件都落在这里，
    /// 与生成子进程的磁盘产物同一个私有目录（ADR-0016 D7）。
    private_dir: PathBuf,
}

impl ZoneStreamSidecar {
    pub fn endpoint(&self) -> &SidecarEndpoint {
        &self.endpoint
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn private_dir(&self) -> &Path {
        &self.private_dir
    }

    /// 拉起 sidecar 并等待端口可连。
    ///
    /// `surreal_bin` 由调用方解析（站点侧已有 `managed_surreal_bin_string()` 的口径），
    /// 这里不重复探测，避免两处对「用哪个 surreal」给出不同答案。
    pub async fn start(surreal_bin: &str, run_id: &str, private_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(private_dir)
            .with_context(|| format!("创建 ZoneStream 私有目录失败: {}", private_dir.display()))?;

        let port = allocate_loopback_port()?;
        let endpoint = SidecarEndpoint {
            port,
            namespace: run_namespace(run_id),
            user: "root".to_string(),
            // 每次运行一套随机口令：sidecar 只在本机 loopback 上活一次运行的时间，
            // 固定口令没有收益，只会让别的进程有机会连上来写内存库。
            password: uuid::Uuid::new_v4().simple().to_string(),
        };

        let log_path = private_dir.join("sidecar.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("打开 sidecar 日志失败: {}", log_path.display()))?;
        let log_err = log
            .try_clone()
            .context("复制 sidecar 日志句柄失败")?;

        let mut command = Command::new(surreal_bin);
        command
            .arg("start")
            .arg("--log")
            .arg("info")
            .arg("--user")
            .arg(&endpoint.user)
            .arg("--pass")
            .arg(&endpoint.password)
            .arg("--bind")
            .arg(format!("127.0.0.1:{port}"))
            .arg("memory")
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        isolate_process_group(&mut command);

        let child = command
            .spawn()
            .with_context(|| format!("启动 ZoneStream kv-mem sidecar 失败: {surreal_bin}"))?;
        let pid = child.id().unwrap_or_default();

        // pid 文件让既有的孤儿 sidecar 清理脚本能在进程异常退出后找到残留。
        let _ = std::fs::write(private_dir.join("sidecar.pid"), pid.to_string());

        let sidecar = Self {
            endpoint,
            child: Some(child),
            pid,
            private_dir: private_dir.to_path_buf(),
        };

        sidecar.wait_until_ready().await?;
        Ok(sidecar)
    }

    async fn wait_until_ready(&self) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        let addr = format!("127.0.0.1:{}", self.endpoint.port);
        loop {
            if tokio::net::TcpStream::connect(&addr).await.is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "ZoneStream sidecar 在 {:?} 内未就绪（pid={}, {}）；详见 {}",
                    READY_TIMEOUT,
                    self.pid,
                    addr,
                    self.private_dir.join("sidecar.log").display()
                );
            }
            tokio::time::sleep(READY_POLL_INTERVAL).await;
        }
    }

    /// 显式停止。内存内容随进程一起消失，这是预期行为（ADR-0016 D3）。
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        let _ = std::fs::remove_file(self.private_dir.join("sidecar.pid"));
        Ok(())
    }
}

impl Drop for ZoneStreamSidecar {
    fn drop(&mut self) {
        // 正常路径走 shutdown()；这里只兜底，避免 orchestrator 早退时留下孤儿进程。
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

/// run-scoped namespace。`run_id` 里的连字符会被去掉，避免 SurrealQL 标识符需要转义。
fn run_namespace(run_id: &str) -> String {
    let sanitized: String = run_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("zs_{sanitized}")
}

/// 让内核分配一个空闲的 loopback 端口。
///
/// 这里有个无法消除的竞态：拿到端口号到 surreal 真正绑上去之间，别的进程可能抢走。
/// 端口是动态的、窗口是毫秒级，抢占概率极低；真抢到了 surreal 会启动失败，
/// 由 [`ZoneStreamSidecar::start`] 的就绪等待超时暴露出来，而不是静默继续。
fn allocate_loopback_port() -> Result<u16> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("为 ZoneStream sidecar 分配空闲端口失败")?;
    let port = listener
        .local_addr()
        .context("读取 ZoneStream sidecar 端口失败")?
        .port();
    drop(listener);
    Ok(port)
}

#[cfg(windows)]
fn isolate_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP：避免父进程收到的 Ctrl-C 直接把 sidecar 也带走，
    // 停止时机由 orchestrator 在批次边界决定（ADR-0016 D9 的 Stop 语义）。
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
fn isolate_process_group(command: &mut Command) {
    command.process_group(0);
}
