# Admin 站点安全收口实施计划（2026-04-24）

> 承接 `docs/plans/2026-04-24-next-sprint-development-plan.md` 的 P2，基于
> 实际 codebase 的 2026-04-24 基线对原始方案做"是否已落地"的复核，然后只动
> 仍有 gap 的部分。

## 一、基线复核：原方案 vs 现状

### 原 P2 方案（来自 `2026-04-21-next-iteration-plan` P0）

| 条目 | 原方案 | 2026-04-24 实测状态 |
|------|--------|-------------------|
| 默认绑定 `127.0.0.1` | 默认从 `0.0.0.0` 收窄 | ✅ 已落地：`managed_project_sites.rs::normalize_host` 在 host 为空时默认 `127.0.0.1` |
| 创建/更新拒绝 `root/root` 弱凭据 | 需要新增 | ❌ 未落地：`require_db_user` / `require_db_password` 只校验非空，显式传 `root/root` 会直接通过 |
| `SiteDrawer.vue` 去默认凭据 | 需要改前端 | ✅ 已落地：`form.value.db_user/db_password` 初值为 `''`，表单提示"不再自动写入默认 root/root" |
| `parse`: Running/Starting/Stopping 或 parse_status=Running 时拒绝 | 需要加守卫 | ✅ 已落地：`parse_site` line 3089–3120 |
| `start`: 禁止重复启动 | 需要加守卫 | ✅ 已落地：`start_site` line 3035–3087 |
| `stop`: 允许在有活动进程时执行 | 需要改 | ✅ 已落地：`stop_site` 的 `can_stop` 条件包含 `site_has_active_processes(&site)` |
| `delete`: 任一进程活跃时拒绝 | 需要加守卫 | ✅ 已落地：`delete_site` line 3267–3280 |
| 前端按钮联动 | 需要加 can_* 判断 | ✅ 已落地：`ui/admin/src/components/sites/site-status.ts` 已有 `canStartSite` / `canStopSite` / `canParseSite` / `canDeleteSite` / `canEditSite` |

### 真正的 gap（本计划要落地的）

1. **后端拒绝显式 `root/root` 弱凭据**
   - 位置：`managed_project_sites.rs::require_db_user` / `require_db_password`（以及 `update_site` 里 `db_user/db_password` 空值判断）
   - 现状：只要非空就通过，`root/root` 可创建成功
   - 问题：`admin_registry.rs` / `create_site` 在 E2E 脚本或误触场景下会把明文 `root/root` 写进站点的 SurrealDB config
   - 逃生通道：显式 env `AIOS_ALLOW_WEAK_DB_CREDS=1` 时跳过拒绝（开发/测试兼容）

2. **后端拒绝 / 警告显式 `0.0.0.0` bind**
   - 位置：`managed_project_sites.rs::normalize_host`
   - 现状：用户显式传 `0.0.0.0` 可以写入 DB，`web_server` 起来后对公网开放
   - 处置：
     - 默认：显式传 `0.0.0.0` 时返回结构化错误（含 env 逃生说明）
     - 逃生通道：`AIOS_ALLOW_PUBLIC_BIND=1` 时放行（保留现网段部署能力）

3. **前端在 `SiteDrawer.vue` 加本地弱凭据软提示**（渐进式 UX，不阻断提交）
   - 目的：在未触发后端之前就告诉用户 `root/root` / `admin/admin` 类组合不推荐
   - 实现：`computed` 一个 `weakCredentialsWarning`，在用户名/密码任一组合命中弱凭据黑名单时显示黄色提示

### 不做的事

- 不新增 Rust `#[cfg(test)]`（遵守 `AGENTS.md`：只用运行起来的 web_server + POST 验证）
- 不扩大到改 admin 响应结构（那是 P4 的范围）
- 不动 `site_registry.rs`（心跳/注册链路与本轮正交）

---

## 二、实现步骤

### Task 1：后端弱凭据拦截（`managed_project_sites.rs`）

**Files:**
- Modify: `src/web_server/managed_project_sites.rs`

**Step 1：新增凭据合法性检查**

在 `require_db_password` 之后插入一组工具函数：

```rust
/// 弱凭据黑名单（小写比较）。如需扩展，加在这里并补 README 说明。
const WEAK_CREDENTIAL_PAIRS: &[(&str, &str)] = &[
    ("root", "root"),
    ("admin", "admin"),
    ("admin", "123456"),
    ("root", "123456"),
    ("test", "test"),
];

fn env_allow_weak_db_creds() -> bool {
    std::env::var("AIOS_ALLOW_WEAK_DB_CREDS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// 拒绝常见弱凭据；允许通过 `AIOS_ALLOW_WEAK_DB_CREDS=1` 逃生（开发/测试）。
fn assert_db_credentials_strong(user: &str, password: &str) -> Result<()> {
    if env_allow_weak_db_creds() {
        return Ok(());
    }
    let u = user.trim().to_ascii_lowercase();
    let p = password.trim().to_ascii_lowercase();
    for (weak_u, weak_p) in WEAK_CREDENTIAL_PAIRS {
        if u == *weak_u && p == *weak_p {
            bail!(
                "数据库凭据过于简单（{}/{}）。请使用更复杂的用户名/密码；如仅用于本地开发，可设置 AIOS_ALLOW_WEAK_DB_CREDS=1 临时放行。",
                user,
                password,
            );
        }
    }
    Ok(())
}
```

**Step 2：在 `create_site` 落点**

紧接 `let db_password = require_db_password(req.db_password)?;` 之后：

```rust
assert_db_credentials_strong(&db_user, &db_password)?;
```

**Step 3：在 `update_site` 落点**

在 `let db_user = normalize_optional_db_user(req.db_user).unwrap_or(stored_db_user);` 两行之后：

```rust
assert_db_credentials_strong(&db_user, &db_password)?;
```

### Task 2：后端公网 bind 拦截（`managed_project_sites.rs`）

**Step 1：重写 `normalize_host` 为校验版**

把现有的 `normalize_host` 从"只做清洗"升级为"清洗 + 校验"：

```rust
fn env_allow_public_bind() -> bool {
    std::env::var("AIOS_ALLOW_PUBLIC_BIND")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn normalize_host(host: Option<String>) -> String {
    host.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// 在写入 DB 之前对 bind_host 做安全校验：
/// - `0.0.0.0` 默认拒绝（公网暴露）
/// - `AIOS_ALLOW_PUBLIC_BIND=1` 时放行，便于需要内网/跨机部署的场景
fn assert_bind_host_safe(host: &str) -> Result<()> {
    let trimmed = host.trim();
    if trimmed == "0.0.0.0" && !env_allow_public_bind() {
        bail!(
            "bind_host=0.0.0.0 会将站点暴露到所有网络接口。请改用 127.0.0.1 或指定内网地址；如确需公网绑定，请设置 AIOS_ALLOW_PUBLIC_BIND=1 并自行承担风险。"
        );
    }
    Ok(())
}
```

> 单独拆出 `assert_bind_host_safe` 是为了保留 `normalize_host` 对外部调用（例如测试脚本）的纯函数语义。

**Step 2：在 `create_site` 落点**

`let bind_host = normalize_host(req.bind_host);` 之后：

```rust
assert_bind_host_safe(&bind_host)?;
```

**Step 3：在 `update_site` 落点**

`if let Some(value) = req.bind_host.filter(|value| !value.trim().is_empty()) {` 块内：

```rust
let value = value.trim().to_string();
assert_bind_host_safe(&value)?;
site.bind_host = normalize_host(Some(value));
```

### Task 3：前端本地弱凭据软提示（`SiteDrawer.vue`）

**Files:**
- Modify: `ui/admin/src/components/sites/SiteDrawer.vue`

**Step 1：添加 computed**

在 script 段已有 `const form = ref(...)` 附近：

```ts
const WEAK_CREDENTIAL_SET = new Set([
  'root/root',
  'admin/admin',
  'admin/123456',
  'root/123456',
  'test/test',
])

const weakCredentialsWarning = computed<string | null>(() => {
  const user = (form.value.db_user || '').trim().toLowerCase()
  const password = (form.value.db_password || '').trim().toLowerCase()
  if (!user || !password) return null
  if (WEAK_CREDENTIAL_SET.has(`${user}/${password}`)) {
    return '检测到常见弱凭据（如 root/root、admin/admin）。后端将拒绝此组合；本地开发可设置 AIOS_ALLOW_WEAK_DB_CREDS=1 放行。'
  }
  return null
})
```

**Step 2：在模板中显示**

在凭据输入框下方、"不再自动写入默认 root/root" 提示之后加：

```html
<div
  v-if="weakCredentialsWarning"
  class="mt-1 rounded-md border border-amber-300 bg-amber-50 px-2 py-1 text-xs text-amber-800 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200"
>
  {{ weakCredentialsWarning }}
</div>
```

> 注意：不要把它变成"阻断提交"（弱凭据黑名单随时间会变化，后端 is source of truth；前端只做 UX 提示）

### Task 4：验证

**Files:**
- 验证文档：`docs/plans/2026-04-24-admin-site-security-hardening-plan.md`（本文件）

**Step 1：Rust 端编译检查**

`cargo check --bin web_server --features web_server`

Expected: 通过。

**Step 2：前端 type-check**

工作目录：`ui/admin/`

`npm run type-check`（若存在）或 `npx vue-tsc --noEmit -p tsconfig.app.json`

Expected: 通过。

**Step 3：HTTP 请求 smoke（人工执行）**

遵守 `AGENTS.md` 的 "运行真实 web_server 再用 POST 验证" 原则：

```powershell
# 1. 创建站点带 root/root（预期：400 含"弱凭据"）
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3100/api/admin/sites `
  -ContentType 'application/json' `
  -Body (@{ project_name='t'; project_path='D:\temp\t'; project_code=1; db_user='root'; db_password='root'; db_port=8001; web_port=3101 } | ConvertTo-Json)

# 2. 创建站点带 bind_host=0.0.0.0（预期：400 含"0.0.0.0 会将站点暴露"）
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3100/api/admin/sites `
  -ContentType 'application/json' `
  -Body (@{ project_name='t'; project_path='D:\temp\t'; project_code=1; db_user='good'; db_password='strongpw'; bind_host='0.0.0.0'; db_port=8001; web_port=3101 } | ConvertTo-Json)

# 3. env 放行后同一请求应成功
$env:AIOS_ALLOW_PUBLIC_BIND='1'
# 重启 web_server 后重试第 2 条请求，预期 201
```

Expected：
- 步骤 1：HTTP 400（或 500 + 结构化 error），错误信息包含"弱凭据"
- 步骤 2：HTTP 400，错误信息包含"0.0.0.0"
- 步骤 3：HTTP 201

**Step 4：UI 冒烟**

- 打开 admin 建站抽屉，输入 `root`/`root` → 黄色警示出现
- 改为 `admin`/`strongpw` → 警示消失
- 尝试以 `root/root` 提交 → 后端 400，前端 error toast（沿用现有 `handleSubmit` 的 catch 路径）

---

## 三、风险与缓解

| 风险 | 等级 | 缓解 |
|------|------|------|
| 既有站点已用 `root/root`，升级后没法再 `update` 它（除非改凭据） | 中 | 文档标注：升级后 update 带弱凭据会被拒，推荐改强凭据或设置 `AIOS_ALLOW_WEAK_DB_CREDS=1` |
| 已有自动化脚本依赖 `0.0.0.0` bind | 中 | 同上，env 逃生通道 `AIOS_ALLOW_PUBLIC_BIND=1` 保留 |
| 弱凭据黑名单不全面（例如 `dbadmin/123`） | 低 | 只做最常见 5 条；后续可按需扩展 `WEAK_CREDENTIAL_PAIRS` |
| 前端 warning 干扰用户正常操作 | 低 | 只做软提示，不阻断提交 |

## 四、完成定义

- `cargo check --bin web_server --features web_server` 通过
- `ui/admin` 的 vue-tsc 通过（或相应前端 type-check）
- HTTP smoke 三条请求按预期返回 400 / 400 / 201
- UI 冒烟：弱凭据警示按预期出现/消失
- 本文件第五节记录实际执行结果

## 五、执行记录（2026-04-24）

### 代码改动

- `src/web_server/managed_project_sites.rs`
  - 新增 `assert_bind_host_safe(&str) -> Result<()>`（0.0.0.0 拒绝 + `AIOS_ALLOW_PUBLIC_BIND=1` 逃生）
  - 新增 `env_allow_public_bind()`
  - 新增 `WEAK_CREDENTIAL_PAIRS`（5 条常见组合：root/root、admin/admin、admin/123456、root/123456、test/test）
  - 新增 `env_allow_weak_db_creds()`
  - 新增 `assert_db_credentials_strong(&str, &str) -> Result<()>`（`AIOS_ALLOW_WEAK_DB_CREDS=1` 逃生）
  - `create_site`：在 `normalize_host` 之后插入 `assert_bind_host_safe`；在 `require_db_password` 之后插入 `assert_db_credentials_strong`
  - `update_site`：`req.bind_host` 分支内插入 `assert_bind_host_safe`；在 `normalize_optional_db_password` 分支后插入 `assert_db_credentials_strong`
- `ui/admin/src/components/sites/SiteDrawer.vue`
  - 新增 `WEAK_CREDENTIAL_SET` 常量 + `weakCredentialsWarning` computed（黑名单和后端对齐）
  - 模板：凭据 fieldset 下方加 amber 软提示条（仅视觉提示，不阻断提交）

### 验证

- `cargo check --bin web_server --features web_server`
  - 结果：`Finished dev profile [unoptimized + debuginfo] target(s) in 1.35s`（零错误）
- `npx vue-tsc --noEmit -p tsconfig.app.json`（工作目录 `ui/admin`）
  - 结果：exit code 0，无错误
- HTTP smoke（待人工执行）：受限于本机 `web_server.exe` 被 PID 27112 持有，未能本轮 rebuild + 重启。HTTP 三条请求（root/root、0.0.0.0、env 放行）留作后续验证，预期：
  - `POST /api/admin/sites` with `db_user=root & db_password=root` → 500/400 含"数据库凭据过于简单"
  - `POST /api/admin/sites` with `bind_host=0.0.0.0` → 500/400 含"0.0.0.0 会将站点暴露"
  - `AIOS_ALLOW_PUBLIC_BIND=1` 重启后同一请求 → 201
- UI 冒烟（待人工执行）：刷新 admin 页面打开站点建站抽屉，输入 `root`/`root` 应触发 amber 警示；改强口令警示消失；弱凭据尝试提交后应收到后端 error。

### 剩余事项

- 需要用户允许停掉 PID 27112 的 web_server 后，按第四 Step 3 的 PowerShell 脚本跑一遍实际 HTTP smoke，核实错误文本符合预期
- 后续如需扩展黑名单，统一改 `WEAK_CREDENTIAL_PAIRS`（Rust）+ `WEAK_CREDENTIAL_SET`（TS），保证两端同步
