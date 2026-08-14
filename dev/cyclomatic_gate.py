#!/usr/bin/env python3
# Copyright 2025 yu-iskw
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""Function-level cyclomatic gate over a Debtmap JSON report.

Exit codes:
  0 — parsed function metrics and none exceeded the threshold
  1 — one or more functions exceeded the threshold
  2 — report missing, unsupported, or unusable (do not treat as a pass)
"""

from __future__ import annotations

import json
import sys
from typing import Any, NoReturn


def fail(message: str) -> NoReturn:
    print(f"ERROR: {message}", file=sys.stderr)
    sys.exit(2)


def as_dict(value: Any, key: str) -> dict[str, Any]:
    nested = value.get(key) if isinstance(value, dict) else None
    return nested if isinstance(nested, dict) else {}


def load_report(path: str) -> dict[str, Any]:
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except OSError as exc:
        fail(f"failed to read {path}: {exc}")
    except json.JSONDecodeError as exc:
        fail(f"failed to parse {path}: {exc}")
    if not isinstance(data, dict):
        fail(f"{path} is not a JSON object")
    version = str(data.get("format_version") or "")
    if version != "3.0":
        fail(f"unsupported Debtmap format_version {version!r}; expected '3.0'")
    return data


def function_rows(data: dict[str, Any]) -> list[tuple[float, str, str]]:
    items = data.get("items")
    if not isinstance(items, list):
        fail("Debtmap JSON does not contain an items array")
    rows: list[tuple[float, str, str]] = []
    for item in items:
        if not isinstance(item, dict) or item.get("type") != "Function":
            continue
        location = as_dict(item, "location")
        function = str(location.get("function") or "unknown")
        if function == "[file-scope]":
            continue
        metrics = as_dict(item, "metrics")
        cyclo = metrics.get("cyclomatic_complexity")
        if not isinstance(cyclo, (int, float)):
            continue
        file = str(location.get("file") or "")
        line = location.get("line") or ""
        location_s = f"{file}:{line}" if file else ""
        rows.append((float(cyclo), function, location_s))
    return rows


def expected_function_count(data: dict[str, Any]) -> int:
    by_type = as_dict(as_dict(data, "summary"), "by_type")
    count = by_type.get("Function", 0)
    return int(count) if isinstance(count, (int, float)) else 0


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        fail("usage: cyclomatic_gate.py <debtmap.json> <threshold>")
    path, threshold_s = argv[1], argv[2]
    try:
        threshold = float(threshold_s)
    except ValueError:
        fail(f"threshold must be a number, got {threshold_s!r}")

    data = load_report(path)
    summary = as_dict(data, "summary")
    total_loc = summary.get("total_loc", 0)
    if not isinstance(total_loc, (int, float)) or total_loc <= 0:
        fail("Debtmap report has no analyzed LOC; refusing to pass an empty analysis")

    rows = function_rows(data)
    expected = expected_function_count(data)
    if expected > 0 and not rows:
        fail(
            f"Debtmap summary reports {expected} Function items but none had "
            "parseable function-level cyclomatic_complexity"
        )

    rows.sort(key=lambda row: row[0], reverse=True)
    print("==> Highest-complexity functions (top 20):")
    if not rows:
        print("  (no function-level metrics)")
    for cyclo, name, location_s in rows[:20]:
        suffix = f"  {location_s}" if location_s else ""
        print(f"  {name}  cyclomatic={cyclo:g}{suffix}")

    violations = [row for row in rows if row[0] > threshold]
    if not violations:
        print(f"    No functions exceeded cyclomatic complexity {threshold:g}.")
        return 0

    print("")
    print(f"ERROR: {len(violations)} function(s) exceed cyclomatic complexity {threshold:g}.")
    print("  Refactor the flagged functions. Do not suppress this gate to make CI green.")
    for cyclo, name, location_s in violations:
        suffix = f"  {location_s}" if location_s else ""
        print(f"  VIOLATION: {name}  cyclomatic={cyclo:g} > {threshold:g}{suffix}")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
