# AvevaPlantSample 站点部署 + DESI 解析端到端测试与修复（2026-04-28）

> 任务来源：MCP 通道 `best-mcp-9` 直接派发
> 任务一句话：测试 `plant-model-gen` 的「站点部署 + DESI db 解析 + 启动站点 + 打开 plant3d-web」全流程，监控部署是否生成正确，浏览器点按钮启动站点。
> 关联：
> - `runtime/admin_sites/avevaplantsample-18330/DbOption.toml`
> - `runtime/admin_sites/avevaplantsample-18330/DbOption-parse.toml`
> - `src/web_server/managed_project_sites.rs`
> - `D:/work/plant-code/rs-core/src/lib.rs`
> - `D:/work/plant-code/rs-core/src/options.rs`

---

## 0. 背景

| 项 | 值 |
|---|---|
| 主管理 web_server 端口 | 3100（admin UI `/admin/#/registry`） |
| 子站点 AvevaMarineSample | 已 Running，监听 3120（与主进程绑定） |
| 子站点 AvevaPlantSample | **Failed**，配置端口 web=18330 / surreal=18320 |
| 目标 db 文件 | `D:\AVEVA\Projects\E3D2.1\AvevaPlantSample\aps000\aps7009`（DESI，382976 字节，已存在） |
| 站点 surreal 数据目录 | `runtime/admin_sites/avevaplantsample-18330/data/surreal.db`（RocksDB） |
| Admin 登录凭据 | `admin / admin`（已通过 `/api/admin/auth/login` 验证） |

`runtime/admin_sites/avevaplantsample-18330/logs/parse.log` 显示最近 3 次解析任务全部失败，统一在
`rs-core/src/rs_surreal/index.rs:55` panic：

```
called `Result::unwrap()` on an `Err` value: Error { code: -32002,
  message: "Anonymous access not allowed: Not enough permissions to perform this action",
  details: NotAllowed(..) }
```

直接原因：parse 子进程 `init_surreal()` 实际连接的是 `ws://127.0.0.1:8020`（默认值），
但 SurrealDB 服务监听在 `ws://127.0.0.1:18320`，导致跨站点连到了主站点的 surreal，
凭据 `apsadmin` 在主站点不存在 → 鉴权失败 → 后续查询走 anonymous → panic。

---

## 1. 根因定位

### 1.1 关键证据

`parse.log` line 25 / 36 / 47：

```
🌐 后端: WebSocket 远程
🌐 连接服务器: ws://127.0.0.1:8020
👤 用户名: apsadmin
❌ 连接尝试 1 失败: 数据库初始化失败: There was a problem with authentication
```

`parse.log` line 260（同一进程内的 sdb_cfg 调试输出）：

```
🐛 [DEBUG sdb_cfg] mode=Ws ip=127.0.0.1 port=8020 user=apsadmin password=Aps2026Admin@
  path=Some("runtime/admin_sites/avevaplantsample-18330/data/surreal.db")
  | top.surreal_port=18320 top.surreal_user=apsadmin top.surreal_password=Aps2026Admin@
```

也就是：

- `[surrealdb]` 子表里**除了 port 全部正确反序列化**：mode=ws、ip=127.0.0.1、user=apsadmin、password=Aps2026Admin@、path=...都对。
- 唯独 `port` 不是 toml 里写的 `18320`，而是 `default_surrealdb_port()` 默认值 `8020`。
- 顶层 `surreal_port = 18320` 的字段反而被正确读到。

### 1.2 配置文件已确认正确

`runtime/admin_sites/avevaplantsample-18330/DbOption-parse.toml` 末尾：

```toml
[surrealdb]
ip = "127.0.0.1"
mode = "ws"
password = "Aps2026Admin@"
path = "runtime/admin_sites/avevaplantsample-18330/data/surreal.db"
port = 18320
user = "apsadmin"
```

`Test-Path` + `Get-Content` 验证文件可读、`port = 18320` 存在。

### 1.3 候选根因

| 编号 | 假设 | 评估 |
|---|---|---|
| H1 | `config` crate 解析 nested table 时 u16 字段从 String → number 转换静默回落到 `#[serde(default)]` | **最可能**：crate 默认把所有 leaf 当 String，`SurrealDbConfig.port: u16` 的 `default = "default_surrealdb_port"` 会在反序列化失败时静默触发 |
| H2 | `OnceCell` 在 `set_var("DB_OPTION_FILE")` 之前已被触发，读到了主进程的 DbOption | 已排除：`mode/ip/user/password/path` 均与 parse.toml 一致，证明 OnceCell 加载的就是 parse.toml |
| H3 | toml 里 `port = 18320`（裸整数）被 config crate 当字符串 `"18320"` 处理，导致字段类型不匹配 | 与 H1 是同一个根因的不同描述 |

H1/H3 是同一个 bug：`config` crate 把所有 toml leaf 都当作 `Value::String` 暂存，然后通过 `serde::de::Visitor` 转目标类型；当目标是 `u16` 时按理可以用 `visit_string` + parse，但如果实现走了 `visit_str` 后 fallback，`#[serde(default)]` 会无声触发。

### 1.4 反向佐证

- `surreal_port = 18320`（顶层，类型同样是 `u16`，无 nested table）能读到 → 证明问题只发生在 nested `[surrealdb]` 子表的整数字段。
- `mqtt_port = 1883`（顶层，整数）能读到。
- `[surrealkv]` 子表里只有 bool / string，没有 numeric 字段，没有报错。

可以确定是 `config` crate + `[surrealdb]` 子表 + `u16` 三者交叉的 bug，与 plant-model-gen 业务无关。

---

## 2. 方案选择

### 2.1 候选方案对比

| 方案 | 改动面 | 风险 | 选择 |
|---|---|---|---|
| A. 修复 `config` crate 上游 | 第三方 crate | 大、慢、不可控 | ❌ |
| B. rs-core `effective_surrealdb()` 在 `[surrealdb].port` 缺失/默认时回落到 `top.surreal_port` | rs-core 1 行 + 1 个测试 | 小、确定 | ✅ |
| C. plant-model-gen 写 parse.toml 时把 `[surrealdb] port` 写成与 `top.surreal_port` 相同的字符串 `"18320"` | plant-model-gen managed_project_sites.rs | 治标不治本，仍可能踩到其他 numeric 字段 | ❌ |
| D. 改成 file 模式绕过 ws | rs-core / managed_project_sites（spawn_db_process 要 skip） | 改动多、影响主管理面 | ❌ |
| E. 调整 surreal 服务监听端口为 8020（与 main 站点冲突） | 运行时操作 | 严重破坏 main web_server | ❌ |

**结论：选 B**。理由：

1. 只改 rs-core 一个函数，不动业务逻辑；
2. 与既有「顶层字段为兼容 fallback」的设计一致（rs-core 已经在很多地方用顶层字段）；
3. 单元测试可覆盖；
4. 不会重新引入主管理面 surreal 端口冲突。

### 2.2 修复要点（rs-core/src/options.rs）

```rust
// 原实现
pub fn effective_surrealdb(&self) -> SurrealDbConfig {
    self.surrealdb.clone()
}

// 新实现：当 [surrealdb] 子表的 port 为默认 8020 但顶层 surreal_port 不是默认值时,
// 优先采用顶层值。绕过 config crate 在 nested u16 字段上的 silent default fallback。
pub fn effective_surrealdb(&self) -> SurrealDbConfig {
    let mut cfg = self.surrealdb.clone();
    if cfg.port == default_surrealdb_port()
        && self.surreal_port != 0
        && self.surreal_port != default_surrealdb_port()
    {
        cfg.port = self.surreal_port;
    }
    cfg
}
```

**对应可加的单元测试**（rs-core/src/options.rs `mod tests`）：

```rust
#[test]
fn effective_surrealdb_falls_back_to_top_surreal_port() {
    let mut opt = DbOption::default();
    opt.surreal_port = 18320;
    // 模拟 [surrealdb].port 没读到，保持 default 8020
    assert_eq!(opt.surrealdb.port, 8020);
    let eff = opt.effective_surrealdb();
    assert_eq!(eff.port, 18320, "应当回落到 top.surreal_port");
}
```

---

## 3. 执行计划

### Step 1 — 制定计划文件 *(本文件)*
- 输出：`docs/plans/2026-04-28-aveva-plant-sample-deployment-test-plan.md`
- 验收：本文件存在并完整描述上述各节
- 状态：✅

### Step 2 — 实施 rs-core 最小修复
- 改动：`D:/work/plant-code/rs-core/src/options.rs`
- 内容：见 §2.2
- 验收：
  - `cargo check -p plant-model-gen --features web_server` 通过
  - rs-core 自身 `cargo test --lib` 中 `effective_surrealdb_*` 全部 PASS

### Step 3 — 重新编译 web_server 并热重启
- 命令：
  ```powershell
  cd D:\work\plant-code\plant-model-gen
  cargo build --bin web_server --features web_server
  Stop-Process -Id 296788 -Force   # 当前监听 3100 的 web_server.exe
  # 后台启动新的 web_server，使用同样的 -c db_options/DbOption
  Start-Process -FilePath .\target\debug\web_server.exe `
                -ArgumentList "--config", "db_options/DbOption" `
                -RedirectStandardOutput web_server_stdout.log `
                -RedirectStandardError  web_server_stderr.log
  ```
- 环境：保留 `ADMIN_USER=admin / ADMIN_PASS=admin`（如果是父 shell 设的，需要在新 shell 里重新设）
- 验收：
  - `curl http://127.0.0.1:3100/api/site/identity` 返回 200
  - `curl -X POST .../api/admin/auth/login` 用 `admin/admin` 拿到 token

### Step 4 — 通过浏览器 admin UI 重新触发 AvevaPlantSample 解析
- 入口：`http://192.168.31.60:3100/admin/#/sites`（cursor-ide-browser 用 LAN IP，不能用 127.0.0.1）
- 操作序列：
  1. 用 `admin/admin` 登录
  2. 在站点表格找到 `avevaplantsample-18330`
  3. 点击该行的「解析」按钮
  4. 进入「详情」页观察实时日志 + 解析状态
- 验收：
  - 详情页 `parse_status` 从 `Failed` → `Parsing` → `Parsed`
  - `parse.log` 不再出现 `port=8020` 行；改为 `port=18320`
  - `parse.log` 不再出现 `Anonymous access not allowed` panic
  - aps7009 的解析最终落库 SurrealDB

### Step 5 — 通过浏览器点击「启动」按钮启动子站点
- 操作：
  1. 解析成功后，回到 `/admin/#/sites`
  2. 点击 `avevaplantsample-18330` 行的「启动」按钮
- 验收：
  - 该行状态从 `Parsed` → `Starting` → `Running`
  - `netstat -ano | findstr :18330` 出现 LISTENING（web 进程）
  - `netstat -ano | findstr :18320` 出现 LISTENING（surreal 进程）
  - `curl http://127.0.0.1:18330/api/site/identity` 返回 200，`site_id=avevaplantsample-18330`

### Step 6 — 在浏览器打开 plant3d-web 前端
- 选项 A（直连子站点 web_server 内置静态资源）：
  - URL：`http://192.168.31.60:18330/`
  - 验收：页面加载，能看到 plant3d-web 的入口（哪怕只是空 viewer）
- 选项 B（dev server）：
  - 仅当 A 失败再考虑 `D:\work\plant-code\plant3d-web` 跑 `pnpm dev` 反代到 18330
- 验收：
  - 页面网络请求中至少有一次成功命中 `/api/site/identity` 或 `/api/projects` 返回 200
  - 控制台无致命错误

### Step 7 — 汇报 + 进度落档
- `report_task(done)` 简洁汇报
- `save_progress` 标记完成；把「rs-core nested numeric 字段 + config crate silent default」这个 finding 沉淀到 plant-model-gen 的 `AGENTS.md` 或单独 ADR

---

## 4. 风险与回退

| 风险 | 触发条件 | 回退策略 |
|---|---|---|
| rs-core 修复影响其他依赖（gen_model-dev / aios-database-dev / cad） | 改 effective_surrealdb 语义 | git revert rs-core 该 commit；但因为是 fallback 而非覆盖，影响范围极小 |
| 重编 web_server 时间 > 5min | 增量编译失败 | 不要 cargo clean；如必要，仅重编 plant-model-gen 的 web_server bin |
| Stop-Process 后 surreal 子进程残留占用 8020 | 主 web_server graceful shutdown 失败 | `taskkill /F /IM surreal.exe` 兜底；`Get-NetTCPConnection -LocalPort 8020` 检查 |
| 解析依然失败但错误变了 | 仍然是配置/权限问题 | 重看 parse.log，对照本文件 §1.3 候选根因再分析 |
| 18330 web 起来但 plant3d-web 静态资源没打包到 web_server | 静态资源路径未链接 | 用选项 B（dev server）兜底；或者先 `cd plant3d-web && pnpm build` 复制 dist |

---

## 5. 不做（Out of Scope）

- 不去修 `config` crate 自身（H1）。
- 不去重构 `OnceCell → RwLock<Arc<DbOption>>`（rs-core 早有 backlog，不在本次 scope）。
- 不去触碰 AvevaMarineSample 主站点（保持 Running 不动）。
- 不部署到远端 `123.57.182.243`（本次只做本地端到端测试）。
- 不改 admin 鉴权 / 不引入 argon2（与本任务无关）。

---

## 6. 时间预算

| 步骤 | 预估耗时 |
|---|---|
| Step 1 写 plan | 10 分钟（已完成） |
| Step 2 改 rs-core 1 行 | 5 分钟 |
| Step 3 重编 + 重启 web_server | 5–10 分钟（debug 增量） |
| Step 4 浏览器触发解析 + 监控 aps7009 | 5–15 分钟（取决于 db 大小） |
| Step 5 启动 18330 站点 | 1–2 分钟 |
| Step 6 浏览器打开 plant3d-web 验证 | 3–5 分钟 |
| 合计 | **~30–45 分钟** |

---

## 7. 立即行动

下一步：执行 §3 Step 2，修改 `D:/work/plant-code/rs-core/src/options.rs` 的 `effective_surrealdb()` 函数。
