"""一键部署到 123.57.182.243（生产）。

流程：
  1. 检查工作区是否 clean，确认 commit 都已落地
  2. push 指定 branch 到 origin（GitHub）
  3. 触发 GitHub Actions Deploy workflow（自动）
  4. 监控远程 /root/web_server 与 /var/www/plant3d-web/index.html 的 mtime
     直到时间戳更新，说明 CI 已部署完成
  5. 健康检查：调 backend /api/review/workflow/sync action=query 看响应
  6. 输出部署 summary

用法（PowerShell）：
    python scripts/deploy/deploy_to_production.py
    python scripts/deploy/deploy_to_production.py --repo plant3d-web
    python scripts/deploy/deploy_to_production.py --no-push --skip-verify

环境变量（可选覆盖）：
    DEPLOY_HOST          目标服务器 IP，默认 123.57.182.243
    DEPLOY_USER          SSH 用户，默认 root
    DEPLOY_PASSWORD      SSH 密码，默认从 setup-deploy-server.sh 一致
    DEPLOY_TIMEOUT       等待 CI 完成的最大秒数，默认 1800（30 分钟）
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

try:
    import paramiko
except ImportError:
    print("[deploy] paramiko 未安装，请先 `pip install paramiko`", file=sys.stderr)
    sys.exit(1)


# ============================================================================
# Configuration
# ============================================================================

REPO_ROOT = Path(__file__).resolve().parents[2]  # plant-model-gen 根
WORKSPACE_ROOT = REPO_ROOT.parent  # work/plant-code/

HOST = os.environ.get("DEPLOY_HOST", "123.57.182.243")
USER = os.environ.get("DEPLOY_USER", "root")
PASSWORD = os.environ.get("DEPLOY_PASSWORD", "Happytest123_")
TIMEOUT = int(os.environ.get("DEPLOY_TIMEOUT", "1800"))


@dataclass
class RepoSpec:
    """单个仓库的部署规约。"""

    name: str
    local_path: Path
    expected_branch: str
    # 远程被替换的产物路径（用 mtime 比对部署前后是否更新）
    remote_artifacts: list[str] = field(default_factory=list)
    # 健康检查 URL（HEAD/GET 200 即认为 OK）；部署后访问
    health_urls: list[str] = field(default_factory=list)


REPOS: dict[str, RepoSpec] = {
    "plant-model-gen": RepoSpec(
        name="plant-model-gen",
        local_path=WORKSPACE_ROOT / "plant-model-gen",
        expected_branch="feat/collab-api-consolidation",
        remote_artifacts=["/root/web_server"],
        health_urls=[],  # 后端健康检查走 sync(query)，单独跑
    ),
    "plant3d-web": RepoSpec(
        name="plant3d-web",
        local_path=WORKSPACE_ROOT / "plant3d-web",
        expected_branch="feat/rus-244-design-a-ui-empty-state",
        remote_artifacts=["/var/www/plant3d-web/index.html", "/var/www/plant3d-web/version.json"],
        health_urls=["http://123.57.182.243/version.json", "http://123.57.182.243/index.html"],
    ),
}


# ============================================================================
# Helpers
# ============================================================================


def log(level: str, msg: str) -> None:
    ts = time.strftime("%H:%M:%S")
    color = {"INFO": "\033[36m", "OK": "\033[32m", "WARN": "\033[33m", "FAIL": "\033[31m", "STEP": "\033[1;35m"}.get(level, "")
    reset = "\033[0m" if color else ""
    print(f"[{ts}] [{color}{level}{reset}] {msg}", flush=True)


def run_local(cmd: list[str], cwd: Path, capture: bool = True, check: bool = True) -> subprocess.CompletedProcess:
    log("INFO", f"$ ({cwd.name}) {' '.join(shlex.quote(c) for c in cmd)}")
    return subprocess.run(cmd, cwd=cwd, capture_output=capture, text=True, encoding="utf-8", check=check)


def ssh_connect() -> paramiko.SSHClient:
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, username=USER, password=PASSWORD, timeout=20)
    return client


def ssh_run(client: paramiko.SSHClient, cmd: str, timeout: int = 60) -> tuple[str, str, int]:
    _, stdout, stderr = client.exec_command(cmd, timeout=timeout)
    out = stdout.read().decode("utf-8", errors="replace")
    err = stderr.read().decode("utf-8", errors="replace")
    rc = stdout.channel.recv_exit_status()
    return out, err, rc


def remote_mtime(client: paramiko.SSHClient, path: str) -> Optional[int]:
    out, _, rc = ssh_run(client, f"stat -c %Y {shlex.quote(path)} 2>/dev/null", timeout=10)
    if rc != 0:
        return None
    out = out.strip()
    return int(out) if out.isdigit() else None


# ============================================================================
# Steps
# ============================================================================


def check_clean(spec: RepoSpec) -> bool:
    cp = run_local(["git", "status", "--porcelain", "--", spec.local_path.as_posix()], cwd=spec.local_path, check=False)
    if cp.returncode != 0:
        log("WARN", f"[{spec.name}] git status 失败：{cp.stderr.strip()}")
        return False
    # 只关心 staged 修改（已 commit 进当前 branch 的不在这里），不严格 fail；仅警告
    if cp.stdout.strip():
        log("WARN", f"[{spec.name}] 工作区有未提交改动，将仅 push 当前分支已 commit 的内容：")
        for line in cp.stdout.strip().splitlines()[:10]:
            log("WARN", f"  {line}")
    return True


def check_branch(spec: RepoSpec) -> bool:
    cp = run_local(["git", "branch", "--show-current"], cwd=spec.local_path)
    current = cp.stdout.strip()
    if current != spec.expected_branch:
        log("FAIL", f"[{spec.name}] 当前分支 {current} ≠ 预期 {spec.expected_branch}，跳过")
        return False
    log("OK", f"[{spec.name}] 当前分支 {current}")
    return True


def check_ahead(spec: RepoSpec) -> tuple[int, int]:
    """返回 (behind, ahead)。"""
    cp = run_local(
        ["git", "rev-list", "--left-right", "--count", "@{u}...HEAD"],
        cwd=spec.local_path,
        check=False,
    )
    if cp.returncode != 0:
        log("WARN", f"[{spec.name}] 无法判断 ahead/behind：{cp.stderr.strip()}")
        return 0, 0
    parts = cp.stdout.strip().split()
    if len(parts) != 2:
        return 0, 0
    return int(parts[0]), int(parts[1])


def push_branch(spec: RepoSpec, dry_run: bool = False) -> bool:
    behind, ahead = check_ahead(spec)
    if behind > 0:
        log("FAIL", f"[{spec.name}] 本地 behind upstream {behind} commit；请先 git pull --rebase")
        return False
    if ahead == 0:
        log("OK", f"[{spec.name}] 已与 upstream 同步，无需 push")
        return True
    if dry_run:
        log("INFO", f"[{spec.name}] 跳过 push（--no-push）；本地 ahead {ahead} commit 待发布")
        return True
    log("STEP", f"[{spec.name}] git push origin {spec.expected_branch}（ahead {ahead}）")
    cp = run_local(["git", "push", "origin", spec.expected_branch], cwd=spec.local_path, check=False)
    if cp.returncode != 0:
        log("FAIL", f"[{spec.name}] push 失败：{cp.stderr.strip()}")
        return False
    log("OK", f"[{spec.name}] push 成功")
    return True


def wait_artifact_updated(
    client: paramiko.SSHClient,
    spec: RepoSpec,
    initial_mtimes: dict[str, Optional[int]],
    timeout: int,
    poll_interval: int = 30,
) -> dict[str, bool]:
    """轮询远端产物 mtime，直到都更新或超时。"""
    deadline = time.time() + timeout
    pending = list(spec.remote_artifacts)
    updated: dict[str, bool] = {p: False for p in spec.remote_artifacts}
    log("STEP", f"[{spec.name}] 监听远端产物 mtime（最长 {timeout}s）：{', '.join(pending)}")
    while pending and time.time() < deadline:
        for path in list(pending):
            current = remote_mtime(client, path)
            initial = initial_mtimes.get(path)
            if current is not None and (initial is None or current > initial):
                updated[path] = True
                pending.remove(path)
                ts = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(current))
                log("OK", f"[{spec.name}] {path} mtime 已更新 → {ts}")
        if not pending:
            break
        remaining = int(deadline - time.time())
        log("INFO", f"[{spec.name}] 仍在等待 {len(pending)} 项（剩余 {remaining}s）：{', '.join(pending)}")
        time.sleep(poll_interval)
    if pending:
        log("FAIL", f"[{spec.name}] 超时未更新：{', '.join(pending)}")
    return updated


def health_check(client: paramiko.SSHClient, spec: RepoSpec) -> bool:
    """Backend 健康检查走远端 curl 调 sync(query)；前端健康走 HEAD 200。"""
    all_ok = True
    if spec.name == "plant-model-gen":
        # 后端 health：调 sync(query) 看 service 是否响应
        cmd = (
            'curl -sS -o /dev/null -w "%{http_code}" -X POST '
            'http://localhost:3100/api/review/workflow/sync '
            '-H "Content-Type: application/json" '
            '--data \'{"form_id":"DEPLOY-HEALTH-CHECK","token":"deploy-probe","action":"query","actor":{"id":"deploy","name":"deploy","roles":"sj"}}\''
        )
        out, err, rc = ssh_run(client, cmd, timeout=20)
        code = out.strip()
        if code in {"200", "401", "403"}:  # 401/403 也算服务正常（鉴权拒绝），只要不是 5xx/连接失败
            log("OK", f"[{spec.name}] backend sync(query) HTTP {code}")
        else:
            log("FAIL", f"[{spec.name}] backend 健康检查失败 HTTP {code} stderr={err.strip()[:200]}")
            all_ok = False
        # 看 systemd unit 状态
        out, _, _ = ssh_run(client, "systemctl is-active web-server.service", timeout=10)
        active = out.strip()
        if active == "active":
            log("OK", f"[{spec.name}] web-server.service active")
        else:
            log("FAIL", f"[{spec.name}] web-server.service 状态={active}")
            all_ok = False
    for url in spec.health_urls:
        cmd = f'curl -sS -o /dev/null -w "%{{http_code}}" -L {shlex.quote(url)}'
        out, _, _ = ssh_run(client, cmd, timeout=20)
        code = out.strip()
        if code == "200":
            log("OK", f"[{spec.name}] {url} HTTP 200")
        else:
            log("FAIL", f"[{spec.name}] {url} HTTP {code}")
            all_ok = False
    return all_ok


# ============================================================================
# Orchestration
# ============================================================================


def deploy(spec: RepoSpec, args) -> dict:
    log("STEP", f"==== [{spec.name}] 开始 ====")
    summary = {"name": spec.name, "ok": False, "steps": {}}

    if not check_branch(spec):
        summary["steps"]["branch"] = False
        return summary
    summary["steps"]["branch"] = True
    summary["steps"]["clean"] = check_clean(spec)

    initial_mtimes: dict[str, Optional[int]] = {}
    if not args.skip_verify:
        client = ssh_connect()
        try:
            for path in spec.remote_artifacts:
                initial_mtimes[path] = remote_mtime(client, path)
            for path, t in initial_mtimes.items():
                ts = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(t)) if t else "(missing)"
                log("INFO", f"[{spec.name}] 部署前 {path} mtime={ts}")
        finally:
            client.close()

    pushed = push_branch(spec, dry_run=args.no_push)
    summary["steps"]["push"] = pushed
    if not pushed:
        return summary

    if args.skip_verify:
        log("INFO", f"[{spec.name}] --skip-verify 跳过部署后验证")
        summary["ok"] = True
        return summary

    if args.no_push:
        log("INFO", f"[{spec.name}] --no-push 模式不监听远端，仅打印初始 mtime")
        summary["ok"] = True
        return summary

    log("STEP", f"[{spec.name}] 等待 GitHub Actions Deploy workflow 完成（CI 触发后产物会被替换）")
    log("INFO", f"[{spec.name}] 进度可在 https://github.com/happyrust/{spec.name}/actions 查看")
    client = ssh_connect()
    try:
        updated = wait_artifact_updated(client, spec, initial_mtimes, args.timeout)
        summary["steps"]["artifacts_updated"] = updated
        all_updated = all(updated.values())
        if all_updated:
            log("OK", f"[{spec.name}] 所有产物 mtime 已更新")
            time.sleep(8)
            summary["steps"]["health"] = health_check(client, spec)
            summary["ok"] = summary["steps"]["health"]
        else:
            log("FAIL", f"[{spec.name}] 部分产物超时未更新")
    finally:
        client.close()

    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description="一键部署 plant-model-gen / plant3d-web 到生产服务器")
    parser.add_argument("--repo", choices=["all"] + list(REPOS.keys()), default="all", help="部署哪个仓库（默认 all）")
    parser.add_argument("--no-push", action="store_true", help="不 push，只检查状态与远端时间戳")
    parser.add_argument("--skip-verify", action="store_true", help="push 后不等待远端产物更新（不验证 CI 完成）")
    parser.add_argument("--timeout", type=int, default=TIMEOUT, help=f"等待 CI 完成的最大秒数（默认 {TIMEOUT}）")
    args = parser.parse_args()

    targets = list(REPOS.values()) if args.repo == "all" else [REPOS[args.repo]]
    log("STEP", f"目标：{', '.join(t.name for t in targets)}（host={HOST}）")

    results = [deploy(spec, args) for spec in targets]

    log("STEP", "==== 部署结果汇总 ====")
    print(json.dumps(results, ensure_ascii=False, indent=2))
    all_ok = all(r["ok"] for r in results)
    if all_ok:
        log("OK", "全部仓库部署成功 ✅")
        return 0
    log("FAIL", "部分仓库部署失败 ❌")
    return 1


if __name__ == "__main__":
    sys.exit(main())
