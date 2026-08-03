---
status: proposed
---

# 用显式 CLI 子命令执行 refno scoped 快速抽查

## Context

开发者需要从一个 refno 出发，一次完成设计子树与 CATA 依赖闭包解析及模型生成。现有闭包、manifest 部分解析和 scoped 生成能力已经存在，但分散在多个 CLI 调用中；partial/closure 解析又按 ADR-0012 故意不发布全库 `pe_owner` Ready 证据。

## Decision

- 该能力交付为 `aios-database` 的显式 Rust 子命令；PowerShell 脚本可以调用它，但不是正式能力边界。
- 子命令复用现有闭包、解析和生成入口，不另建第二套解析或生成管线。
- partial/closure 解析不得伪造 `pe_owner_version_meta.bulk_state = ready`，普通 `Live` 读取也继续 fail-closed。
- 新子命令在本次解析范围验证成功后，向生成读取层传入仅本次运行有效的 scoped 层级覆盖凭据；凭据只放行其覆盖范围内的层级读取，不改变持久 Ready 状态，也不能泄漏到普通生成入口。
- 输入 refno 本身是生成根；生成范围为该根及其子树，不自动上卷到最小交付单元，也不回退到 dbnum 或全项目生成。
- 输入根的完整 owner 祖先链必须一直解析到 `WORL`，作为 world transform 与独立调试显示所需的结构上下文；祖先链不因此成为几何生成目标，CATA 闭包节点也仅作为生成读取依赖。
- scoped 显示树输出一条 `WORL → … → owner → 输入根` 的结构骨架，再接输入根子树；祖先节点不生成几何，也不展开其兄弟或其他子树。
- CATA 闭包固定使用现有 `CataClosureConfig::precise()`，保留生成期惰性小闭包补齐；v1 不暴露闭包调参。manifest 的 unresolved/missing 计入抽查报告，但不单独判失败；必需数据补齐后仍不可用时由生成失败关闭。
- 子命令自动启动并独占一个本机 `surreal start memory` 子进程，自动选择端口、等待就绪并派生本次运行配置；用户不需要预先启动或配置 endpoint。v1 不把解析与写入链改造成接收进程内 `Surreal::new::<Mem>` 句柄。
- 每次运行创建唯一的 `runtime/refno-inspect/<run-id>/`，派生配置把 manifest、mesh、scoped 显示树、报告和日志全部重定向到该目录；惰性 CATA 补齐也必须更新本次显式 manifest，禁止合并进共享默认输出。
- 命令结束默认停止其 memory server，但保留运行目录供复盘；显式 `--keep-server` 才保留服务并输出 endpoint、PID 与清理指引。失败运行遵循同一保留策略。
