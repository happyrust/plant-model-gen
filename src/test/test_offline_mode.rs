/// `--offline` 模式相关测试：
/// - `find_pids_on_port` 查找占用端口的进程
/// - `kill_process_on_port` 端到端测试（启动临时 TCP listener → kill → 验证端口释放）
///
/// 注意：`kill_process_on_port` 和 `find_pids_on_port` 定义在 binary crate (main.rs → cli_modes.rs) 中，
/// 而测试模块属于 lib crate，无法直接引用。因此这里以自包含方式复现同样的逻辑进行测试。

#[cfg(test)]
mod tests {

    // ========== 内联的被测函数（与 cli_modes.rs 中的实现保持一致） ==========

    /// 查找占用指定端口的进程 PID 列表（Unix 版本）。
    #[cfg(unix)]
    fn find_pids_on_port(port: u16) -> Vec<String> {
        let output = match std::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
        {
            Ok(o) => o,
            Err(_) => return vec![],
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// 查找占用指定端口的进程 PID 列表（Windows 版本）。
    #[cfg(windows)]
    fn find_pids_on_port(port: u16) -> Vec<String> {
        let output = match std::process::Command::new("netstat")
            .args(["-ano"])
            .output()
        {
            Ok(o) => o,
            Err(_) => return vec![],
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

    /// 强制终止指定 PID 的进程，返回是否成功（Unix）。
    #[cfg(unix)]
    fn kill_pid(pid: &str) -> bool {
        match std::process::Command::new("kill")
            .args(["-9", pid])
            .output()
        {
            Ok(r) => r.status.success(),
            Err(_) => false,
        }
    }

    /// 强制终止指定 PID 的进程，返回是否成功（Windows）。
    #[cfg(windows)]
    fn kill_pid(pid: &str) -> bool {
        match std::process::Command::new("taskkill")
            .args(["/F", "/PID", pid])
            .output()
        {
            Ok(r) => r.status.success(),
            Err(_) => false,
        }
    }

    /// 关闭占用指定端口的进程。
    fn kill_process_on_port(port: u16) {
        let pids = find_pids_on_port(port);
        if pids.is_empty() {
            return;
        }
        for pid in &pids {
            kill_pid(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // ========== 测试辅助 ==========

    /// 检查指定端口是否有进程正在监听。
    fn is_port_in_use(port: u16) -> bool {
        std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
    }

    /// 找一个当前未被占用的临时端口。
    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    // ========== 测试用例 ==========

    /// 对一个没有任何进程监听的端口调用 kill，应该不报错、不 panic。
    #[test]
    fn kill_on_unused_port_is_noop() {
        let port = find_free_port();
        kill_process_on_port(port);
        // 能走到这里就算通过
    }

    /// 启动一个子进程监听端口，然后用 kill_process_on_port 杀掉它，验证端口被释放。
    #[test]
    fn kill_releases_occupied_port() {
        let port = find_free_port();

        // 启动一个子进程占用该端口
        let mut child = std::process::Command::new("python3")
            .args([
                "-c",
                &format!(
                    "import socket,time; s=socket.socket(); \
                     s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); \
                     s.bind(('127.0.0.1',{})); s.listen(1); time.sleep(60)",
                    port
                ),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("需要 python3 来运行此测试");

        // 等待子进程开始监听
        let mut listening = false;
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if is_port_in_use(port) {
                listening = true;
                break;
            }
        }
        assert!(listening, "子进程未能在端口 {} 上开始监听", port);

        // 调用 kill_process_on_port
        kill_process_on_port(port);

        // 验证端口已释放
        let released = !is_port_in_use(port);
        if !released {
            let _ = child.kill();
            let _ = child.wait();
            panic!("kill_process_on_port 未能释放端口 {}", port);
        }

        let _ = child.wait();
    }

    /// 验证 find_pids_on_port 对空闲端口返回空列表。
    #[test]
    fn find_pids_returns_empty_for_free_port() {
        let port = find_free_port();
        let pids = find_pids_on_port(port);
        assert!(pids.is_empty(), "空闲端口不应有 PID，但找到: {:?}", pids);
    }

    /// 验证 find_pids_on_port 对被占用端口能找到 PID。
    #[test]
    fn find_pids_returns_pid_for_listening_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let pids = find_pids_on_port(port);
        assert!(!pids.is_empty(), "占用端口 {} 应能找到 PID", port);

        // 验证返回的 PID 包含当前进程
        let my_pid = std::process::id().to_string();
        assert!(
            pids.contains(&my_pid),
            "找到的 PID {:?} 中应包含当前进程 {}",
            pids,
            my_pid
        );

        drop(listener);
    }
}
