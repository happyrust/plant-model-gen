#!/usr/bin/env python3
"""读取 JSON fixture，逐条 POST web_server 空间计算接口并校验响应。"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


class ValidationError(Exception):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="验证 web_server 空间计算接口 JSON 基线")
    parser.add_argument(
        "--input",
        default="verification/space/compute/web_server_validation.json",
        help="fixture JSON 路径",
    )
    parser.add_argument(
        "--base-url",
        default=os.environ.get("BASE_URL", "http://127.0.0.1:3185"),
        help="web_server 根地址，默认读取 BASE_URL 或 http://127.0.0.1:3185",
    )
    return parser.parse_args()


def resolve_path(data: Any, path: str) -> Any:
    current = data
    for part in path.split("."):
        if isinstance(current, list):
            try:
                index = int(part)
            except ValueError as exc:
                raise ValidationError(f"路径 {path} 在数组位置要求数字索引，实际为 {part}") from exc
            try:
                current = current[index]
            except IndexError as exc:
                raise ValidationError(f"路径 {path} 数组索引越界: {index}") from exc
        elif isinstance(current, dict):
            if part not in current:
                raise ValidationError(f"响应里缺少字段: {path}")
            current = current[part]
        else:
            raise ValidationError(f"路径 {path} 无法继续解析，停在 {current!r}")
    return current


def post_json(url: str, payload: dict[str, Any]) -> tuple[int, Any]:
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            return response.status, json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode("utf-8", errors="replace")
        raise ValidationError(f"HTTP {exc.code}: {raw}") from exc
    except urllib.error.URLError as exc:
        raise ValidationError(f"请求失败: {exc}") from exc


def assert_equals(data: Any, equals: dict[str, Any]) -> None:
    for path, expected in equals.items():
        actual = resolve_path(data, path)
        if actual != expected:
            raise ValidationError(f"字段 {path} 不符合预期，实际={actual!r}，预期={expected!r}")


def assert_approx(data: Any, approx: dict[str, dict[str, float]]) -> None:
    for path, config in approx.items():
        actual = resolve_path(data, path)
        expected = config["value"]
        tolerance = config["tolerance"]
        if not isinstance(actual, (int, float)):
            raise ValidationError(f"字段 {path} 不是数字，无法做近似比较，实际={actual!r}")
        if math.isnan(actual) or abs(float(actual) - float(expected)) > float(tolerance):
            raise ValidationError(
                f"字段 {path} 超出容差，实际={actual!r}，预期={expected!r}，容差={tolerance!r}"
            )


def assert_gt(data: Any, gt: dict[str, float]) -> None:
    for path, lower_bound in gt.items():
        actual = resolve_path(data, path)
        if not isinstance(actual, (int, float)):
            raise ValidationError(f"字段 {path} 不是数字，无法做大小比较，实际={actual!r}")
        if not float(actual) > float(lower_bound):
            raise ValidationError(f"字段 {path} 应大于 {lower_bound!r}，实际={actual!r}")


def validate_case(case: dict[str, Any], base_url: str) -> Any:
    case_id = case["case_id"]
    endpoint = case["endpoint"]
    payload = case["request"]
    expect = case["expect"]
    status_code, response = post_json(f"{base_url.rstrip('/')}{endpoint}", payload)
    if status_code != 200:
        raise ValidationError(f"HTTP 状态码不是 200，实际={status_code}")

    if "status" in expect:
        actual_status = response.get("status")
        if actual_status != expect["status"]:
            raise ValidationError(
                f"响应 status 不符合预期，实际={actual_status!r}，预期={expect['status']!r}"
            )

    if "message" in expect:
        actual_message = response.get("message")
        if actual_message != expect["message"]:
            raise ValidationError(
                f"响应 message 不符合预期，实际={actual_message!r}，预期={expect['message']!r}"
            )

    if "data_null" in expect:
        actual_is_null = response.get("data") is None
        if actual_is_null != bool(expect["data_null"]):
            raise ValidationError(
                f"响应 data 空值状态不符合预期，实际={'null' if actual_is_null else 'non-null'}"
            )

    if response.get("data") is None:
        if any(key in expect for key in ("equals", "approx", "gt")):
            raise ValidationError("响应 data 为 null，无法继续校验子字段")
        return

    if "equals" in expect:
        assert_equals(response, expect["equals"])
    if "approx" in expect:
        assert_approx(response, expect["approx"])
    if "gt" in expect:
        assert_gt(response, expect["gt"])

    print(json.dumps(response, ensure_ascii=False, indent=2))
    return response


def main() -> int:
    args = parse_args()
    fixture_path = Path(args.input)
    if not fixture_path.is_absolute():
        fixture_path = Path.cwd() / fixture_path
    if not fixture_path.exists():
        print(f"未找到 fixture: {fixture_path}", file=sys.stderr)
        return 1

    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    test_cases = fixture.get("test_cases") or []
    if not test_cases:
        print(f"fixture 没有 test_cases: {fixture_path}", file=sys.stderr)
        return 1

    print(f"🧪 {fixture.get('description', '空间计算接口验证')}")
    print(f"BASE_URL : {args.base_url.rstrip('/')}")
    print(f"FIXTURE  : {fixture_path}")

    failures: list[str] = []
    known_gaps: list[str] = []
    passed = 0
    for index, case in enumerate(test_cases, start=1):
        print("\n" + "=" * 72)
        print(f"[{index}/{len(test_cases)}] {case['case_id']} | {case.get('issue', 'N/A')}")
        print(case.get("description", ""))
        print(f"POST {case['endpoint']}")
        print(json.dumps(case["request"], ensure_ascii=False, indent=2))
        expected_failure = bool(case.get("expected_failure"))
        try:
            validate_case(case, args.base_url)
            if expected_failure:
                failures.append(
                    f"{case['case_id']}: 该 case 被标记为 expected_failure，但当前已通过，请更新 fixture"
                )
                print("❌ 失败: expected_failure case 已经通过，请更新 fixture")
            else:
                passed += 1
                print("✅ 通过")
        except ValidationError as exc:
            if expected_failure:
                known_gaps.append(f"{case['case_id']}: {exc}")
                reason = case.get("expected_failure_reason")
                if reason:
                    print(f"⚠️ 已命中已知差距: {reason}")
                else:
                    print(f"⚠️ 已命中已知差距: {exc}")
            else:
                failures.append(f"{case['case_id']}: {exc}")
                print(f"❌ 失败: {exc}")

    print("\n" + "=" * 72)
    if known_gaps:
        print("已命中的已知差距：")
        for gap in known_gaps:
            print(f"- {gap}")
    if failures:
        print("以下 case 未通过：", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(f"通过 {passed} 条，已记录已知差距 {len(known_gaps)} 条")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
