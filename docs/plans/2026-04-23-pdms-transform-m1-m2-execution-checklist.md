# 执行清单 · PDMS Transform 修复落盘 & 路由聚合重构（M1 + M2）

> 父计划：`docs/plans/2026-04-23-pdms-transform-followup-hardening-plan.md`
>
> 分支：`feat/collab-api-consolidation`（已切，9e3fd6f 已 push）
>
> 估时：M1 ≈ 2h · M2 ≈ 4h · 合计 ≈ 6h · 产出前两个里程碑（修复上线 + 装配机制硬化）

---

## 关键前置事实（动手前必读）

| 维度 | 当前状态 | 本清单目标 |
| --- | --- | --- |
| 修复 commit | `9e3fd6f` 本地有，origin 已接收 | 合入主干并上 123.57.182.243 |
| 路由聚合函数 | ❌ 不存在 | ✅ `assemble_all_web_api_routes` 成为唯一装配入口 |
| 漏挂载审计 | 本次是唯一漏挂载（19 定义 vs 18 挂载） | 聚合后不再可能 silently miss |
| 有状态路由 | 6 个（collision / e3d_tree / noun_hierarchy / spatial_query / search / upload） | 保留在 `web_server/mod.rs`，不进聚合函数 |
| 无状态路由 | 13 个（pdms_attr / pdms_transform / pdms_model_query / ptset / room_tree / scene_tree / mbd_pipe / review_integration / platform_api / jwt_auth / review_api / pipeline_annotation(nest) / version(nest)） | 全部由聚合函数托管 |

**原则**：

1. **保持 merge 顺序**：旧装配顺序不要扰动，避免 Axum path-priority 意外
2. **逐步编译**：每小步都跑 `cargo check --bin web_server --features web_server`
3. **运行期验证不依赖 cargo test**：遵循 `AGENTS.md`，用 HTTP 打真后端
4. **回滚随时可行**：任何一步失败都能回到当前 `9e3fd6f`

---

## M1.1 · 合并策略确认（10 min）

- [ ] 确认 `feat/collab-api-consolidation` 的最终并入目标是 `main` 还是 `only-csg`（CI 触发分支集）
- [ ] 若是 PR 流程：
  - [ ] 访问 `https://github.com/happyrust/plant-model-gen/pull/new/feat/collab-api-consolidation`
  - [ ] PR 标题：`fix(web_server): register pdms_transform_routes so q pos/ori no longer 404`
  - [ ] PR 描述：直接引用 `docs/plans/2026-04-23-pdms-transform-route-missing-registration-fix.md`
- [ ] 若是直推：
  - [ ] `git checkout main && git pull && git merge --no-ff feat/collab-api-consolidation && git push`

完成判定：CI 或人工 review 通过，远端带有 9e3fd6f。

---

## M1.2 · 部署到 123.57.182.243（45 min）

### 选一条路径执行

**方案 A（推荐 · bash 环境）**

- [ ] 在 Git Bash / WSL / mac terminal 跑：
  ```bash
  cd <repo>
  REMOTE_PASS='Happytest123_' ./shells/deploy/deploy_web_server_bundle.sh
  ```
- [ ] 观察脚本输出，等到 `systemctl restart web-server` 成功
- [ ] 远端 `systemctl status web-server` 显示 active (running)

**方案 B（纯 SSH 手动 · 耗时但透明）**

- [ ] `ssh root@123.57.182.243`（`Happytest123_`）
- [ ] `cd /opt/plant-model-gen`（以实际部署目录为准）
- [ ] `git fetch origin feat/collab-api-consolidation`
- [ ] `git checkout feat/collab-api-consolidation && git pull --ff-only`
- [ ] `cargo build --release --bin web_server --features "ws,gen_model,manifold,project_hd,surreal-save,sqlite-index,web_server,parquet-export"`
- [ ] `systemctl restart web-server`
- [ ] `systemctl status web-server | head -20`

**方案 C（GitHub Actions · 等合入 main / only-csg 后自动）**

- [ ] 合入主干后观察 `Deploy Applications` workflow
- [ ] Actions 结束后远端已自动重启

---

## M1.3 · 线上验证（15 min）

- [ ] 远端本机跑：
  ```bash
  curl -s http://127.0.0.1:3100/api/pdms/transform/24381_145018 | head -c 300
  ```
  预期：`{"success":true,"refno":"24381_145018","world_transform":[...16 numbers...],"owner":"24381_144975",...}`

- [ ] 外网对等 IP 跑（若 Nginx 反代已配）：
  ```bash
  curl -s https://<public-host>/api/pdms/transform/24381_145018 | head -c 300
  ```

- [ ] 生产 plant3d-web 控制台依次：
  ```
  = 24381/145018
  q pos
  q ori
  q pos wrt owner
  q ori wrt owner
  ```
  预期：4 条全部输出位置/方位数据，**不再出现** `Failed to query transform: Error: HTTP 404 Not Found`

- [ ] 采样另一个 refno（不同 noun 类型）重复一次，减少偶然性

完成判定：线上 curl 返回 200 + success=true；前端控制台四条命令全绿。

---

## M2.1 · 目标装配结构设计（30 min）

- [ ] 清点目前 `src/web_api/mod.rs` 里的 `create_*_routes()`：
  - [ ] 无状态（13）：`pdms_attr` `pdms_transform` `pdms_model_query` `ptset` `room_tree` `scene_tree` `mbd_pipe` `review_integration` `platform_api` `jwt_auth` `review_api` `pipeline_annotation` `version`
  - [ ] 有状态（6）：`collision` `e3d_tree` `noun_hierarchy` `spatial_query` `search` `upload`
- [ ] 确认两类路由在 `src/web_server/mod.rs` 的 merge 顺序，写成表格贴在聚合函数旁边的注释里
- [ ] 决定 `WebApiAssembly` 结构：

```rust
pub struct WebApiAssembly {
    pub stateless: Router,
    pub collision: Router,
    pub e3d_tree: Router,
    pub noun_hierarchy: Router,
    pub spatial_query: Router,
    pub search: Router,
    pub upload: Router,
    /// 供 M5 使用：所有已装配的 path 原样记录（不含 nest 前缀）
    pub known_paths: Vec<String>,
}
```

---

## M2.2 · 落地 `assemble_all_web_api_routes`（60 min）

- [ ] 在 `src/web_api/mod.rs` 末尾新增聚合函数（详见父计划 §2.2）
- [ ] `stateless` Router 构造顺序与现有 `web_server/mod.rs` 一致：
  ```rust
  let stateless = Router::new()
      .merge(create_pdms_attr_routes())
      .merge(create_pdms_transform_routes())
      .merge(create_pdms_model_query_routes())   // 原来次序
      .merge(create_ptset_routes())               // 原来次序
      .merge(create_room_tree_routes())           // 会和后面状态路由位置合拍
      .merge(create_scene_tree_routes())
      .merge(create_mbd_pipe_routes())
      .merge(create_review_integration_routes())
      .merge(create_platform_api_routes())
      .merge(create_jwt_auth_routes())
      .merge(create_review_api_routes())
      .nest("/api/pipeline", create_pipeline_annotation_routes())
      .nest("/api", create_version_routes());
  ```
- [ ] `cargo check --bin web_server --features web_server` 直到零错误

---

## M2.3 · 切换 `web_server/mod.rs` 到聚合函数（60 min）

- [ ] 移除旧的多个 `let xxx_routes = create_xxx_routes();`（无状态部分）
- [ ] 移除旧的多次 `.merge(xxx_routes)`（无状态部分）
- [ ] 在构造完 `app_state` 与 6 个有状态 state 之后：
  ```rust
  let api = assemble_all_web_api_routes(
      collision_state,
      e3d_tree_state,
      noun_hierarchy_state,
      spatial_query_state,
      search_state,
      upload_state,
  );

  app = app
      // 保持与旧顺序一致：先 admin_*，再 stateful，再 stateless
      .merge(api.spatial_query)
      .merge(api.noun_hierarchy)
      .merge(api.e3d_tree)
      .merge(api.stateless)         // ← 一次挂 13 条
      .merge(api.collision)
      .merge(api.search)
      .merge(api.upload);
  ```
- [ ] `cargo build --bin web_server --features web_server` 直到零错误
- [ ] diff 对比 `rg "\.merge\(|\.nest\(" src/web_server/mod.rs` 的结果，确认 merge 数量**减少到原来的 1/3 左右**

---

## M2.4 · 行为不变性验证（60 min）

**预置**：本地起 web_server（延续 M1 本地环境）

- [ ] 跑 `scripts/verify_fixing_position.ps1`，1 passed 0 failed
- [ ] 逐条 curl 抽样：
  - [ ] `GET /api/pdms/transform/24381_145018` → 200
  - [ ] `GET /api/pdms/ui-attr/24381_145018` → 200
  - [ ] `GET /api/ptset/<某 PSMX refno>` → 200
  - [ ] `GET /api/version` → 200 {version, commit, buildDate}
  - [ ] `GET /api/room-tree/...` → 200（抽一个已知房间）
  - [ ] `GET /api/e3d-tree/roots` → 200（有状态路由，走 state 分支）
  - [ ] `GET /api/collision/...` → 200/expected-404（有状态路由）
  - [ ] `POST /api/spatial-query/...` → 200
  - [ ] `GET /api/noun-hierarchy/...` → 200
- [ ] 抽样失败 → 对照 diff 定位丢失的 merge，补回

---

## M2.5 · 收尾 commit（15 min）

- [ ] `git status` 确认只有：
  - `src/web_api/mod.rs` (修改)
  - `src/web_server/mod.rs` (修改)
- [ ] 提交：
  ```
  refactor(web_server): introduce assemble_all_web_api_routes to prevent silent route miss
  ```
  Body：
  - 原因：本月 pdms_transform 被漏挂导致前端 q pos 全报 404（见 9e3fd6f）
  - 动作：将 13 个无状态路由统一交给 `web_api::assemble_all_web_api_routes` 装配；`web_server/mod.rs` 减肥
  - 回归：`verify_fixing_position.ps1` 通过，9 条抽样 curl 通过
- [ ] push 并合入主干（按 M1.1 的策略）

---

## 连带产出

完成本清单后，同一迭代内可追加：

- [ ] M3 · `fetchJson` 对 404 结构化提示（0.5d）
- [ ] M4 · `scripts/verify_pdms_console_api.ps1`（1d）
- [ ] M5 · 启动期路由清单打印（0.5d）

这些在父计划 `2026-04-23-pdms-transform-followup-hardening-plan.md` 里详细列出，完成 M2 后可拉独立 execution-checklist 继续。

---

## 风险 & 回滚

| 风险 | 触发条件 | 回滚 |
| --- | --- | --- |
| 聚合函数把某个路由漏在 stateless 之外 | `.merge()` 次数明显偏少 | `git revert` 到 `9e3fd6f`（仍有修复） |
| merge 顺序变化导致 path 冲突（比如两条 `/api/xxx/{id}` 撞车） | 任意 curl 抽样 404/500 异常 | 恢复 M2 之前的顺序映射表 |
| 有状态路由与无状态路由 nesting 差异被消平 | `pipeline_annotation` 失去 `/api/pipeline` 前缀 | 核对 `.nest(...)` 保留 |
| 部署机重启 web_server 后旧 surreal 端口残留 | `auto_start_surreal=true` 与外部 surreal 冲突 | `systemctl restart surrealdb` 前先 `pkill -9 surreal` |

**整体保底**：每一步都是可 `git revert` 的小 commit；任何一步崩坏，回退一个 commit 即可。
