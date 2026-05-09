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
    # GitHub repo 全名，用于拼 workflow_dispatch URL / gh CLI -R 参数
    github_repo: str = ""
    # GitHub Actions deploy workflow 文件名（gh workflow run 用）
    deploy_workflow_file: str = ""
    # workflow_dispatch 输入参数默认值（gh workflow run -f key=value）
    dispatch_inputs: dict[str, str] = field(default_factory=dict)
    # 远程被替换的产物路径（用 mtime 比对部署前后是否更新，作为 gh watch 的兜底）
    remote_artifacts: list[str] = field(default_factory=list)
    # 健康检查 URL（HEAD/GET 200 即认为 OK）；部署后访问
    health_urls: list[str] = field(default_factory=list)

    @property
    def workflow_dispatch_url(self) -> str:
        if not self.github_repo or not self.deploy_workflow_file:
            return ""
        return f"https://github.com/{self.github_repo}/actions/workflows/{self.deploy_workflow_file}"


REPOS: dict[str, RepoSpec] = {
    "plant-model-gen": RepoSpec(
        name="plant-model-gen",
        local_path=WORKSPACE_ROOT / "plant-model-gen",
        expected_branch="feat/collab-api-consolidation",
        github_repo="happyrust/plant-model-gen",
        deploy_workflow_file="deploy-web-server-ubuntu.yml",
        dispatch_inputs={"db_option_file": "db_options/DbOption-mac.toml"},
        remote_artifacts=["/root/web_server"],
        health_urls=[],  # 后端健康检查走 sync(query)，单独跑
    ),
    "plant3d-web": RepoSpec(
        name="plant3d-web",
        local_path=WORKSPACE_ROOT / "plant3d-web",
        expected_branch="feat/rus-244-design-a-ui-empty-state",
        github_repo="happyrust/plant3d-web",
        deploy_workflow_file="deploy-ubuntu.yml",
        dispatch_inputs={},
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


# ============================================================================
# gh CLI integration
# ============================================================================


def gh_available() -> bool:
    cp = subprocess.run(["gh", "--version"], capture_output=True, text=True)
    return cp.returncode == 0


def gh_authenticated() -> bool:
    cp = subprocess.run(["gh", "auth", "status"], capture_output=True, text=True)
    return cp.returncode == 0


def gh_workflow_run(spec: RepoSpec, git_ref: str, extra_inputs: Optional[dict[str, str]] = None) -> bool:
    """通过 gh workflow run 触发 workflow_dispatch。"""
    inputs = dict(spec.dispatch_inputs)
    inputs["git_ref"] = git_ref
    if extra_inputs:
        inputs.update(extra_inputs)
    cmd = [
        "gh", "workflow", "run", spec.deploy_workflow_file,
        "-R", spec.github_repo,
        "--ref", git_ref,
    ]
    for key, val in inputs.items():
        cmd.extend(["-f", f"{key}={val}"])
    log("STEP", f"[{spec.name}] $ {' '.join(shlex.quote(c) for c in cmd)}")
    cp = subprocess.run(cmd, capture_output=True, text=True)
    if cp.returncode != 0:
        log("FAIL", f"[{spec.name}] gh workflow run 失败：{(cp.stderr or cp.stdout).strip()}")
        return False
    log("OK", f"[{spec.name}] workflow 已触发")
    return True


def gh_latest_run_id(spec: RepoSpec, git_ref: str, max_age_seconds: int = 300) -> Optional[str]:
    """拿到刚触发的最新 run id（按 createdAt 倒序，且分支匹配）。"""
    cmd = [
        "gh", "run", "list",
        "-R", spec.github_repo,
        "--workflow", spec.deploy_workflow_file,
        "--branch", git_ref,
        "--limit", "5",
        "--json", "databaseId,status,createdAt,conclusion,headBranch",
    ]
    deadline = time.time() + 30
    while time.time() < deadline:
        cp = subprocess.run(cmd, capture_output=True, text=True)
        if cp.returncode != 0:
            log("WARN", f"[{spec.name}] gh run list 失败：{cp.stderr.strip()}")
            return None
        try:
            runs = json.loads(cp.stdout)
        except json.JSONDecodeError:
            return None
        # 按 createdAt 排序找最近一条且 status != completed
        for r in runs:
            try:
                created_ts = time.mktime(time.strptime(r["createdAt"], "%Y-%m-%dT%H:%M:%SZ"))
            except ValueError:
                continue
            age = time.time() - created_ts - time.timezone  # createdAt is UTC
            if age <= max_age_seconds:
                log("OK", f"[{spec.name}] 命中 run id={r['databaseId']} status={r['status']} branch={r['headBranch']}")
                return str(r["databaseId"])
        log("INFO", f"[{spec.name}] 等待 GitHub Actions 创建 run 记录...")
        time.sleep(3)
    log("WARN", f"[{spec.name}] 未在 30s 内拿到新 run id（可能 webhook 延迟），将退回 mtime 监听")
    return None


def gh_watch_run(spec: RepoSpec, run_id: str, timeout: int) -> Optional[str]:
    """gh run watch 阻塞到 run 完成；返回 conclusion (success/failure/cancelled/...)"""
    log("STEP", f"[{spec.name}] $ gh run watch {run_id} -R {spec.github_repo} --exit-status")
    proc = subprocess.Popen(
        ["gh", "run", "watch", run_id, "-R", spec.github_repo, "--exit-status", "--interval", "10"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
    )
    try:
        deadline = time.time() + timeout
        for line in proc.stdout or []:
            line = line.rstrip()
            if line:
                log("INFO", f"[{spec.name}][gh] {line[:200]}")
            if time.time() > deadline:
                proc.kill()
                log("FAIL", f"[{spec.name}] gh run watch 超时 ({timeout}s)")
                return None
    finally:
        proc.wait(timeout=10)
    # 重新查询确认最终 conclusion
    cp = subprocess.run(
        ["gh", "run", "view", run_id, "-R", spec.github_repo, "--json", "conclusion,status"],
        capture_output=True, text=True,
    )
    try:
        data = json.loads(cp.stdout)
    except json.JSONDecodeError:
        return None
    conclusion = data.get("conclusion") or data.get("status")
    log("OK" if conclusion == "success" else "FAIL",
        f"[{spec.name}] run {run_id} conclusion={conclusion}")
    return conclusion


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


def get_current_branch(spec: RepoSpec) -> Optional[str]:
    cp = run_local(["git", "branch", "--show-current"], cwd=spec.local_path, check=False)
    if cp.returncode != 0:
        return None
    branch = cp.stdout.strip()
    return branch or None


def check_branch(spec: RepoSpec, strict: bool) -> tuple[bool, Optional[str]]:
    current = get_current_branch(spec)
    if not current:
        log("FAIL", f"[{spec.name}] 无法获取当前分支（detached HEAD？）")
        return False, None
    if current == spec.expected_branch:
        log("OK", f"[{spec.name}] 当前分支 {current}（匹配建议分支）")
        return True, current
    if strict:
        log("FAIL", f"[{spec.name}] 当前分支 {current} ≠ 建议 {spec.expected_branch}，--strict-branch 模式跳过")
        return False, current
    log("WARN", f"[{spec.name}] 当前分支 {current} ≠ 建议 {spec.expected_branch}，但仍以当前分支为部署目标")
    log("INFO", f"[{spec.name}] GitHub Actions Run workflow 的 git_ref 请填: {current}")
    return True, current


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


def push_branch(spec: RepoSpec, branch: str, dry_run: bool = False) -> bool:
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
    log("STEP", f"[{spec.name}] git push origin {branch}（ahead {ahead}）")
    cp = run_local(["git", "push", "origin", branch], cwd=spec.local_path, check=False)
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

    branch_ok, current_branch = check_branch(spec, strict=args.strict_branch)
    if not branch_ok:
        summary["steps"]["branch"] = False
        return summary
    summary["steps"]["branch"] = True
    summary["steps"]["current_branch"] = current_branch
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

    pushed = push_branch(spec, current_branch or spec.expected_branch, dry_run=args.no_push)
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

    # deploy workflow 是 workflow_dispatch（push 不会自动启动 CI）。
    #   优先：gh CLI 自动触发 + watch run 完成
    #   兜底：用户手动去 GitHub UI 点 Run workflow，脚本轮询远端 mtime
    use_gh = (not args.no_gh) and gh_available() and gh_authenticated()
    git_ref = current_branch or spec.expected_branch

    if use_gh:
        triggered = gh_workflow_run(spec, git_ref)
        summary["steps"]["workflow_triggered"] = triggered
        if not triggered:
            log("WARN", f"[{spec.name}] gh 触发失败，回退到手动模式")
            use_gh = False

    if use_gh:
        # 等 GitHub 把 run 创建出来 → watch 直到 conclusion
        run_id = gh_latest_run_id(spec, git_ref)
        summary["steps"]["run_id"] = run_id
        if run_id:
            conclusion = gh_watch_run(spec, run_id, args.timeout)
            summary["steps"]["run_conclusion"] = conclusion
            if conclusion != "success":
                log("FAIL", f"[{spec.name}] workflow run {conclusion}，不再做远端验证")
                summary["steps"]["run_url"] = f"https://github.com/{spec.github_repo}/actions/runs/{run_id}"
                return summary
        else:
            # gh 拿不到 run_id（可能 webhook 延迟），fallback 到 mtime
            use_gh = False

    if not use_gh and spec.workflow_dispatch_url:
        bar = "=" * 70
        log("STEP", bar)
        log("STEP", f"[{spec.name}] 请去 GitHub Actions 手动触发 Deploy workflow:")
        log("STEP", f"  ↳ {spec.workflow_dispatch_url}")
        log("STEP", f"  → 点击右上方 [Run workflow]")
        log("STEP", f"  → git_ref 填: {git_ref}（或 commit SHA）")
        log("STEP", bar)
        log("STEP", f"[{spec.name}] 触发后脚本会自动等远端产物 mtime 更新（最长 {args.timeout}s）")

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
    parser.add_argument("--strict-branch", action="store_true", help="严格要求当前分支等于 expected_branch，否则跳过")
    parser.add_argument("--no-gh", action="store_true", help="不使用 gh CLI 自动触发 workflow_dispatch（回退到手动 + mtime 监听）")
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
