#!/usr/bin/env python3
"""HTTP verification for workflow/sync optimistic concurrency protection.

Run this against a running web_server. It intentionally avoids Rust test
frameworks because this repo verifies web_server behavior through real POSTs.

Example:
  python scripts/verify_workflow_sync_cas.py --base-url http://127.0.0.1:3100
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from typing import Any


@dataclass
class HttpResult:
    status: int
    body: dict[str, Any]


def post_json(
    base_url: str,
    path: str,
    payload: dict[str, Any],
    *,
    bearer_token: str | None = None,
    timeout: float = 15.0,
) -> HttpResult:
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    headers = {"content-type": "application/json"}
    if bearer_token:
        headers["authorization"] = f"Bearer {bearer_token}"

    request = urllib.request.Request(
        f"{base_url.rstrip('/')}{path}",
        data=data,
        headers=headers,
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
            return HttpResult(response.status, json.loads(raw or "{}"))
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8")
        try:
            body = json.loads(raw or "{}")
        except json.JSONDecodeError:
            body = {"raw": raw}
        return HttpResult(error.code, body)


def delete_json(
    base_url: str,
    path: str,
    *,
    bearer_token: str,
    timeout: float = 15.0,
) -> HttpResult:
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}{path}",
        headers={"authorization": f"Bearer {bearer_token}"},
        method="DELETE",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
            return HttpResult(response.status, json.loads(raw or "{}"))
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8")
        try:
            body = json.loads(raw or "{}")
        except json.JSONDecodeError:
            body = {"raw": raw}
        return HttpResult(error.code, body)


def issue_token(base_url: str, user_id: str, role: str) -> str:
    result = post_json(
        base_url,
        "/api/auth/token",
        {
            "project_id": "workflow-cas-smoke",
            "user_id": user_id,
            "user_name": user_id,
            "role": role,
            "workflow_mode": "internal",
        },
    )
    if result.status != 200 or result.body.get("code") != 0:
        raise RuntimeError(f"token request failed for {user_id}: {result.status} {result.body}")
    token = (result.body.get("data") or {}).get("token")
    if not token:
        raise RuntimeError(f"token response missing token for {user_id}: {result.body}")
    return token


def create_review_task(base_url: str, token: str, form_id: str) -> str:
    result = post_json(
        base_url,
        "/api/review/tasks",
        {
            "title": f"workflow CAS smoke {form_id}",
            "description": "temporary task created by verify_workflow_sync_cas.py",
            "modelName": "workflow-cas-smoke-model",
            "formId": form_id,
            "reviewerId": "JH",
            "checkerId": "JH",
            "checkerName": "JH",
            "approverId": "SH",
            "approverName": "SH",
            "priority": "medium",
            "components": [],
        },
        bearer_token=token,
    )
    if result.status != 200 or not result.body.get("success"):
        raise RuntimeError(f"create task failed: {result.status} {result.body}")
    task = result.body.get("task") or {}
    task_id = task.get("id")
    if not task_id:
        raise RuntimeError(f"create task response missing task.id: {result.body}")
    return task_id


def sync_workflow(
    base_url: str,
    token: str,
    form_id: str,
    action: str,
    next_step: dict[str, str] | None,
    comments: str,
) -> HttpResult:
    payload: dict[str, Any] = {
        "form_id": form_id,
        "token": token,
        "action": action,
        "comments": comments,
    }
    if next_step:
        payload["next_step"] = next_step
    return post_json(base_url, "/api/review/workflow/sync", payload)


def run_concurrent_agree(
    base_url: str,
    token: str,
    form_id: str,
    worker_count: int,
) -> list[HttpResult]:
    barrier = threading.Barrier(worker_count)

    def submit(index: int) -> HttpResult:
        barrier.wait(timeout=10)
        return sync_workflow(
            base_url,
            token,
            form_id,
            "agree",
            {"assignee_id": "SH", "name": "SH", "roles": "sh"},
            f"CAS concurrent agree #{index}",
        )

    with ThreadPoolExecutor(max_workers=worker_count) as executor:
        return list(executor.map(submit, range(worker_count)))


def summarize_result(result: HttpResult) -> str:
    return (
        f"status={result.status}, "
        f"code={result.body.get('code')}, "
        f"error_code={result.body.get('error_code')}, "
        f"message={result.body.get('message')}"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:3100")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--attempts", type=int, default=3)
    parser.add_argument("--keep-task", action="store_true")
    args = parser.parse_args()

    if args.workers < 2:
        raise SystemExit("--workers must be >= 2")

    sj_token = issue_token(args.base_url, "SJ", "sj")
    jh_token = issue_token(args.base_url, "JH", "jd")

    observed_state_changed = False
    for attempt in range(1, args.attempts + 1):
        form_id = f"CAS-{int(time.time())}-{uuid.uuid4().hex[:8]}"
        task_id = create_review_task(args.base_url, sj_token, form_id)
        print(f"[attempt {attempt}] created task={task_id} form_id={form_id}")

        try:
            active = sync_workflow(
                args.base_url,
                sj_token,
                form_id,
                "active",
                {"assignee_id": "JH", "name": "JH", "roles": "jd"},
                "CAS smoke active",
            )
            if active.status != 200 or active.body.get("code") != 200:
                raise RuntimeError(f"active failed: {summarize_result(active)}")

            results = run_concurrent_agree(args.base_url, jh_token, form_id, args.workers)
            success_count = sum(1 for result in results if result.status == 200 and result.body.get("code") == 200)
            state_changed_count = sum(
                1
                for result in results
                if result.status == 409 and result.body.get("error_code") == "WORKFLOW_STATE_CHANGED"
            )
            observed_state_changed = observed_state_changed or state_changed_count > 0

            print(f"[attempt {attempt}] concurrent agree results:")
            for index, result in enumerate(results, start=1):
                print(f"  #{index}: {summarize_result(result)}")

            if success_count != 1:
                print(
                    f"[FAIL] expected exactly one successful agree, got {success_count}",
                    file=sys.stderr,
                )
                return 1
        finally:
            if not args.keep_task:
                cleanup = delete_json(args.base_url, f"/api/review/tasks/{task_id}", bearer_token=sj_token)
                print(f"[attempt {attempt}] cleanup: {summarize_result(cleanup)}")

    if observed_state_changed:
        print("[PASS] CAS conflict observed and duplicate workflow success prevented.")
    else:
        print(
            "[PASS] duplicate workflow success prevented. "
            "No 409 CAS conflict was observed in this run; increase --attempts/--workers to stress harder."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
