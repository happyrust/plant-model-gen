# Init Project CLI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 新增 `init-project` Rust CLI 子命令，一次串联指定 dbnum 的 indextree 生成以及 pe_transform 刷新。

**Architecture:** 在库侧新增独立 `init_project` 模块，承接 dbnums 解析与初始化流程；CLI 侧只负责注册子命令并把 `DbOptionExt` 传入。流程复用现有 `generate_single_indextree`、`db_meta().try_load_default()` 与 `refresh_pe_transform_for_dbnums`，不再走 `total_sync`。

**Tech Stack:** Rust, clap, tokio, anyhow, 现有 aios_database/aios_core 模块

---

### Task 1: 为 init-project 提供可测试的参数与 dbnums 解析辅助

**Files:**
- Modify: `src/cli_args.rs`
- Create: `src/init_project.rs`
- Test: `src/test/test_init_project_cli.rs`

**Step 1: 写失败测试**
- 断言 `add_init_project_subcommand(Command::new("aios-database"))` 后存在 `init-project` 子命令与 `--dbnums` 参数。
- 断言 `resolve_target_dbnums(None, Some(vec![21909]))` 返回 `[21909]`；`resolve_target_dbnums(None, None)` 返回错误。

**Step 2: 运行测试确认失败**
Run: `cargo test test_init_project_cli -- --nocapture`
Expected: FAIL，提示缺少函数/模块或断言失败。

**Step 3: 写最小实现**
- 在 `src/cli_args.rs` 新增 `add_init_project_subcommand`。
- 在 `src/init_project.rs` 新增 `resolve_target_dbnums` 最小实现。

**Step 4: 再跑测试确认通过**
Run: `cargo test test_init_project_cli -- --nocapture`
Expected: PASS。

### Task 2: 实现 init-project 流程编排

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `src/init_project.rs`

**Step 1: 写/扩展失败测试**
- 为 `resolve_target_dbnums` 增加排序去重或空输入错误行为测试（按最终实现决定）。

**Step 2: 写最小实现**
- 在 `src/init_project.rs` 增加 `run_init_project_mode(db_option_ext, cli_dbnums)`。
- 对目标 dbnum 逐个 `generate_single_indextree(dbnum)?`。
- 调用 `init_surreal().await?`、`db_meta().try_load_default()?`、`refresh_pe_transform_for_dbnums(&dbnums).await?`。
- 在 `src/main.rs` 注册并分发 `init-project` 子命令。

**Step 3: 运行测试**
Run: `cargo test test_init_project_cli -- --nocapture`
Expected: PASS。

### Task 3: 编译验证 CLI 接线

**Files:**
- Verify only

**Step 1: 构建二进制**
Run: `cargo build --bin aios-database`
Expected: exit 0。

**Step 2: 检查帮助输出**
Run: `cargo run --bin aios-database -- init-project --help`
Expected: 能看到 `--dbnums` 帮助。

**Step 3: 如环境允许，给出实际运行命令**
Run: `cargo run --bin aios-database -- --config db_options/DbOption-zsy init-project --dbnums 21909`
Expected: 串联 indextree、pe_transform；若目标库/数据目录未就绪，则输出真实错误并据此收敛。
