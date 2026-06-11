# 增量代码审核 — `feat/multi-project-site` (2026-06-10)

> 接续 `docs/code-review-2026-06-08/` 两份文档：审核上轮基线 `c7b9900` 之后的 7 个新提交（HEAD = `b6d23d2`），并复查上轮 9 条发现的最新状态。

| 项目 | 内容 |
|------|------|
| 审核范围 | `c7b9900..b6d23d2`（7 提交，剔除构建产物后 75 文件 / +2,309 / −595） |
| 新发现 | **1 Critical + 3 Warning**（NEW-1 ~ NEW-4） |
| 上轮复查 | W-2 大部分修复 ✅、W-3 实质缓解 ✅；W-1 仍未修 🔴、C-1 继续恶化 🔴（13,455 行） |
| 一句话结论 | **W-2 生命周期收口是高质量的"治本"修复，值得肯定；但任务单飞检查引入了"崩溃后站点永久死锁"的新 Critical，且 W-1 空校验函数两轮未修、上帝模块又长 1,353 行** |

---

## 一、本轮新提交清单

| 提交 | 主题 | 审核结论 |
|------|------|---------|
| `4edd003` | 站点 web bind_host 受控默认 0.0.0.0 | ✅ 安全默认保留（仅 `AIOS_ALLOW_PUBLIC_BIND=1` 时放开，与 `assert_bind_host_safe` 同开关一致） |
| `34641f5` | 离线静态资产本地打包 | ✅ 方向正确 |
| `43eb161` | 任务操作加固（单飞 + 启动前置校验） | ⚠️ 引入 NEW-1 死锁；收敛 create/retry 入口值得肯定 |
| `14d1fe7` | 真实 IP 访问地址 | ⚠️ 引入 NEW-2 代码克隆、NEW-3 回退不一致 |
| `9298f3c` | viewer 离线加载修复 | ✅ 多 host 端口探测修复正确；`choose_nginx_viewer_port` 克隆再生（归入 C-1） |
| `b6d23d2` | 进程生命周期收口（spec 001） | ✅ 核心修复到位；⚠️ NEW-4 提交纪律（混入无关改动） |

---

## 二、新发现

### NEW-1（Critical）admin 崩溃后站点任务永久死锁，无任何解锁路径

- **Symptom**：`43eb161` 给 `create_and_dispatch_site_task` 加了同站点单飞检查（`load_inflight_site_task` 查 SQLite `status IN ('Pending','Running')`，命中即拒绝新任务）。
- **Source**（`src/web_server/admin_task_handlers.rs`）：
  - 任务状态持久化在 SQLite；admin 进程崩溃（OOM / kill -9 / 断电）时 Running 任务**永久滞留**；
  - 启动时无 stale 任务恢复逻辑（无 interrupted / recover 标记）；
  - `cancel_task` 是空壳：对一切任务返回"当前 admin 任务暂不支持取消"（L222-228）；
  - `delete_admin_task` 明确拒绝删除 Pending/Running 任务；
  - `retry_task` 只接受 Failed。
- **Consequence**：四条路全堵死 → 崩溃一次，该站点从此**永远无法提交任何任务**，UI 无解，只能手改 SQLite。这是单飞检查引入前不存在的回归。
- **Remedy**：admin 启动时执行一次启动代恢复：`UPDATE ... SET status='Failed', error='admin 重启中断' WHERE status IN ('Pending','Running')`；或给任务记录加 boot-id/心跳戳，inflight 检查忽略上一代任务。两者任一均可在 ~20 行内闭环。

### NEW-2（Warning）`get_local_ip_via_udp` 整体复制到 platform_api

- `14d1fe7`/`b6d23d2` 把 `get_local_ip_via_udp` + `is_loopback_or_unspecified_host`（含内部 helper、env key 列表）从 `web_server/mod.rs` **整份复制**到 `src/web_api/platform_api/config.rs:68-160`。
- 两份实现必然漂移（env 优先级、回退语义改一处忘一处）。`mod.rs` 版本已是 `pub fn`；若是模块依赖方向约束，应下沉到共享 util，而不是克隆。

### NEW-3（Warning）本机 IP 推断失败时的回退策略四分五裂

同一个 `get_local_ip_via_udp()` 失败（典型于无默认路由的纯离线机——恰是本批提交的目标场景），不同调用点行为不一致：

| 调用点 | 回退行为 | 后果 |
|--------|---------|------|
| `handlers.rs:2463/2773` 等 | `unwrap_or_default()` → 空串 | 拼出 `http://:5173` 损坏 URL |
| `managed_project_sites.rs:505` | `"0.0.0.0"` | 拼出 `http://0.0.0.0:port` 不可访问地址展示给用户 |
| `site_config_handlers.rs:73` 等 | 503 SERVICE_UNAVAILABLE | 行为合理但与上两者不一致 |
| `bin/web_server.rs:95` | 回退 bind_ip | 合理 |

建议统一为单一语义（失败 → 明确报错并提示设置 `AIOS_PUBLIC_HOST`，已有现成错误文案），消灭空串与 `0.0.0.0` 两种"静默坏 URL"。

### NEW-4（Suggestion）提交纪律：生命周期提交混入无关改动

`b6d23d2`（"tighten managed site process lifecycle"）混入：7 个 `resource/surreal/*.surql` 查询改动、`orchestrator.rs`/`room_model.rs`/`pe.rs`、以及 `parse_sidecar.rs::resolve_included_db_files` 的**行为变更**（manual dbnum 匹配范围 `primary_project_roots` → `project_roots`）。该行为变更无独立提交说明，回归排查时会被生命周期主题掩盖。

---

## 三、上轮 9 条发现复查（基线：2026-06-08 报告）

| 发现 | 上轮状态 | 本轮状态 | 证据 |
|------|---------|---------|------|
| **C-1** 上帝模块 | 🔴 12,434 行 | 🔴 **更严重：13,455 行**（再 +1,021） | `wc -l`；本轮 `choose_nginx_viewer_port` 与 `choose_viewer_port` 高度同构，克隆继续再生 |
| **W-1** dbnum 输入预检缺失 | 🔴 空函数 | 🔴 **仍是空函数（两轮未修）** | `managed_project_sites.rs:1060` 仍为 `let _ = projects; Ok(())`；更糟的是 `models.rs:943` 注释声称"保存时由 precheck_dbnum_conflicts 兜底"——**注释与事实相反** |
| **W-2** sidecar 孤儿/TOCTOU/无自关闭 | 🟡 部分缓解 | 🟢 **大部分修复**（`b6d23d2`，按 spec 001） | ① 句柄保留 pid+start_token 防 PID 复用误杀 ② `process_group(0)`+`killpg`/`taskkill /T` 进程树终止，TERM→1.5s→KILL 升级 ③ stop/delete 接线：取消活跃 job→`shutdown_site_sidecars`→守卫式 kill→孤儿扫描（按 `--site-key` 命令行匹配）④ CLI job 取消杀进程树。**残留两点见下** |
| **W-3** 免鉴权 quick-deploy-test | 🟡 仍成立 | 🟢 **实质缓解** | `quick_deploy_test_enabled()` 默认禁用，需 env/config 显式开启（`managed_project_sites.rs:284`）；路由结构仍分裂但风险大降 |
| **W-4** `kill_processes_on_port` 误杀 | 🟡 仍成立 | 🟡 **风险降低** | stop/delete 主路径已改守卫式 `kill_pid_guarded`；按端口杀仅剩 `handlers.rs:62`、`admin_handlers.rs:833` 两个显式入口 |
| **W-5** CLI job 无界轮询 | 🟡 仍成立 | 🟡 **仍成立** | `run_cli_job_with_status` 轮询 loop（`parse_sidecar_client.rs:225`）仍无总 deadline；缓解：sidecar 死亡时 HTTP 错误会退出循环，job sidecar 有自关闭 |
| **S-1/S-2/S-3** | 🟢 仍成立 | ➖ 本轮提交未触及 | 维持上轮结论 |

### W-2 残留（建议趁热收口）

1. **`ensure_sidecar` TOCTOU 仍在**（`parse_sidecar_client.rs:442-456`）：仍是"放锁检查→spawn→再拿锁插入"，并发同 key 请求会 spawn 两个 sidecar，后者覆盖前者句柄。site/job 类有孤儿扫描/自关闭兜底，但——
2. **`scan:`/`resolve:`/`preview:`/`db-index:` 类 sidecar 完全无回收路径**：无自关闭、不在孤儿扫描范围（扫描只匹配 `site:<id>`），且 `process_group(0)` 使其脱离 admin 进程组——**admin 重启后这批 sidecar 成为永久孤儿，新 admin 实例还会再 spawn 一批**。建议给非 job sidecar 加 idle 超时自关闭（复用现成 `schedule_shutdown_after_job` 机制即可）。

---

## 四、正面评价（应当保持的做法）

- `b6d23d2` 是**按 spec 写修复**的范本（`specs/001-managed-site-process-lifecycle/`，验收标准明确），且 pid+start_token 守卫、进程组隔离、TERM→KILL 升级的实现质量高；
- `43eb161` 将 create/retry 收敛到 `create_and_dispatch_site_task` 单一入口，消除了知识重复（呼应上轮主题 D）；
- `9298f3c` 的 `local_port_in_use_any_host` 修复了多接口绑定下端口探测漏判；`SURREAL_CONN_*` env 在 site web 进程显式注入、在 sidecar 显式移除，成对操作清晰；
- `4edd003` 在放开远程直连的同时保住了 127.0.0.1 安全默认。

---

## 五、优先级建议

| 序 | 动作 | 工作量 |
|---|------|--------|
| ① | **修 NEW-1**：启动代恢复 stale 任务（否则崩溃即死锁） | ~20 行 |
| ② | **W-2 残留收口**：非 job sidecar idle 自关闭 + `ensure_sidecar` 单飞 | 小 |
| ③ | **W-1**：补 `precheck_dbnum_conflicts` 实现，或至少删掉 `models.rs:943` 的虚假注释 | 小 |
| ④ | **NEW-3**：统一 IP 推断失败语义，消灭 `http://:port` / `http://0.0.0.0:port` | 小 |
| ⑤ | **NEW-2**：合并两份 `get_local_ip_via_udp` | 小 |
| ⑥ | **C-1**：上帝模块已 13,455 行，继续按上轮建议分批拆（`site_data_validation.rs` 为范本） | 大、分批 |

---

*审核日期：2026-06-10 · 基线 `c7b9900` → HEAD `b6d23d2` · 配套上轮文档见 `docs/code-review-2026-06-08/`*
