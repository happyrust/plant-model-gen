# SurrealDB 3.1→3.2-nightly 生产切换 Runbook

> 适用对象:本机 dist 部署(`dist/package/Plant3D-AIOS-win-x64/release`,
> surreal@8020 + web_server@3100 + aios-database 站点进程)。
> 前置事实(2026-06-10/11 已实测验证,见 specs/003~005):
> - 客户端 3.2.0-nightly(#f01470af)与 3.1.0-alpha server **WS 协议不兼容**
>   (`Failed to decode value from fb value`),必须双端同步切换;
> - 旧 rocksdb 数据可被 3.2 server 直接打开(956 万条 pe 副本实测可查),
>   但打开即**单向升级**内部格式,必须先冷备份;
> - Windows **debug** 版 surreal.exe 主线程栈溢出,需 `/STACK:16777216` 重链接;
>   生产请用 **release** 构建(无此问题,不要把 debug 版投产)。

## 0. 准备(不停服,可提前任意时间做)

```powershell
# 0.1 release 版 surreal server(fork f01470af,与 Cargo.lock 锁定提交一致)
cd D:\work\plant-code\surrealdb-smoke-f01470af
cargo build --release --bin surreal          # 产物 target\release\surreal.exe

# 0.2 release 版本仓二进制(新客户端代码)
cd D:\work\plant-code\plant-model-gen
cargo build --release --bin web_server
cargo build --release --bin aios-database

# 0.3 预演(强烈建议):用已验证的迁移副本起 release server 再冒烟一次
#     数据副本已存在:D:\backup-dbs\ams-8021-migration-test.db
```

## 1. 停服(开始停机窗口)

```powershell
# 仅停 dist 相关进程;确认 PID 路径都在 dist\package 下再杀
Get-Process aios-database,web_server,surreal -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -like "*dist\package*" } |
  Select-Object Id,ProcessName,Path
# 人工核对上面输出后:
Get-Process aios-database,web_server,surreal -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -like "*dist\package*" } | Stop-Process -Force
```

## 2. 冷备份(必须在 surreal 停止后)

```powershell
$stamp = Get-Date -Format yyyyMMdd-HHmmss
robocopy "D:\backup-dbs\ams-8020.db" "D:\backup-dbs\ams-8020.db.bak-$stamp" /E /R:1 /W:1
# 退出码 <8 即成功;记录 $stamp 供回滚
```

## 3. 替换二进制

```powershell
$dist = "D:\work\plant-code\plant-model-gen\dist\package\Plant3D-AIOS-win-x64\release\bin"
Copy-Item "$dist\surreal\surreal.exe" "$dist\surreal\surreal.exe.old-31" -Force
Copy-Item "D:\work\plant-code\surrealdb-smoke-f01470af\target\release\surreal.exe" "$dist\surreal\surreal.exe" -Force
Copy-Item "$dist\web_server.exe" "$dist\web_server.exe.old" -Force
Copy-Item "D:\work\plant-code\plant-model-gen\target\release\web_server.exe" "$dist\web_server.exe" -Force
Copy-Item "$dist\aios-database.exe" "$dist\aios-database.exe.old" -Force
Copy-Item "D:\work\plant-code\plant-model-gen\target\release\aios-database.exe" "$dist\aios-database.exe" -Force
```

## 4. 启动与冒烟

按原有启动方式拉起 dist 服务(web_server 会按配置自启 surreal),然后:

```powershell
Invoke-RestMethod http://127.0.0.1:3100/api/version          # commit 应为新构建
# 数据完整性抽查(用新 CLI):
'SELECT count() FROM pe GROUP ALL;' | & "$dist\surreal\surreal.exe" sql `
  --endpoint http://127.0.0.1:8020 --username root --password root `
  --namespace 1516 --database AvevaMarineSample --hide-welcome
# 校审域冒烟:GET /api/review/tasks、/api/logs/types(参照 specs/003 T106 脚本)
```

## 5. 回滚预案

```powershell
# 停新服务 → 还原二进制(.old / .old-31 改回) → 删除被 3.2 打开过的数据目录
# → 用 bak-$stamp 副本整体还原 → 启动旧服务
```

> 注意:被 3.2 打开过的数据目录**不可**再交还给 3.1 server,必须用第 2 步备份还原。

## 6. 配套事项

- plant3d-web 若直连此站点无需改动;dev 调试代理 `VITE_BACKEND_PORT` 按需切换。
- 远端部署(123.57.182.243)同此流程,先在本机验证后再上。
- 一致性兜底:升级后首次解析/生成任务跑完,抽查 inst_relate 计数与升级前同口径对比。
