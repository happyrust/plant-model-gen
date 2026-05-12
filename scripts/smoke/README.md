# scripts/smoke

固化的"重复执行 smoke"集合。脚本设计为本地或 CI 都能跑：缺少必要参数时降级为最小可用子集，不会强依赖外部环境。

## 当前脚本

### `external_workflow_smoke.ps1`

External workflow mode 回归 smoke。两段式：

| Stage | 内容 | 是否强依赖 |
|---|---|---|
| 1 | `cargo test --lib` 跑 `platform_api::workflow_sync::tests` 下 external/internal 相关单元测试 | 必跑（除非 `-SkipCargo`） |
| 2 | 对一个已经启动的 `web_server` 发 `workflow/verify` 探测，确认 external 模式不被 owner 校验阻断 | 仅当传入 `-DebugToken` 时执行 |

```powershell
# 最小用法（仅 cargo 单测）
pwsh -File scripts/smoke/external_workflow_smoke.ps1

# 全量（叠加 HTTP probe）
pwsh -File scripts/smoke/external_workflow_smoke.ps1 `
    -BaseUrl "http://127.0.0.1:3100" `
    -DebugToken "your-debug-token" `
    -FormId "FORM-EXTERNAL-SMOKE"

# 只跑 HTTP probe
pwsh -File scripts/smoke/external_workflow_smoke.ps1 -SkipCargo `
    -BaseUrl "http://127.0.0.1:3100" -DebugToken "your-debug-token"
```

退出码：

- `0`：全部通过
- `1`：任何一个阶段失败（cargo test 非 0，或 external 模式被 owner 相关 block_code 阻断）

## 设计原则

1. **可重复执行**：脚本不写库，不修改文件，幂等。
2. **降级友好**：缺参数时跳过对应阶段，仍然输出有用信号。
3. **CI 可选运行**：在 CI 里默认只跑 Stage 1（不依赖外部服务）。
