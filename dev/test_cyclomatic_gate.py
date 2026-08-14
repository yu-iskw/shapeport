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
"""Unit tests for cyclomatic_gate.py. Run: python3 -m unittest test_cyclomatic_gate.py"""

from __future__ import annotations

import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import cyclomatic_gate as gate


def report(
    *,
    items: list[dict],
    total_loc: int = 10,
    function_count: int | None = None,
    version: str = "3.0",
) -> dict:
    if function_count is None:
        function_count = sum(1 for item in items if item.get("type") == "Function")
    return {
        "format_version": version,
        "summary": {"total_loc": total_loc, "by_type": {"Function": function_count}},
        "items": items,
    }


def function_item(name: str, cyclo: float, file: str = "src/lib.rs", line: int = 1) -> dict:
    return {
        "type": "Function",
        "location": {"function": name, "file": file, "line": line},
        "metrics": {"cyclomatic_complexity": cyclo},
    }


class FunctionRowsTests(unittest.TestCase):
    def test_skips_file_scope_and_non_functions(self) -> None:
        data = report(
            items=[
                function_item("[file-scope]", 99, file="src/lib.rs"),
                {"type": "File", "metrics": {"cyclomatic_complexity": 50}},
                function_item("foo", 5, file="src/foo.rs", line=12),
            ],
            function_count=2,
        )
        self.assertEqual(gate.function_rows(data), [(5.0, "foo", "src/foo.rs:12")])

    def test_missing_items_array_fails(self) -> None:
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            gate.function_rows({"format_version": "3.0"})
        self.assertEqual(caught.exception.code, 2)


class LoadReportTests(unittest.TestCase):
    def test_rejects_wrong_format_version(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "debtmap.json"
            path.write_text(json.dumps(report(items=[], version="2.0")), encoding="utf-8")
            with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
                gate.load_report(str(path))
            self.assertEqual(caught.exception.code, 2)

    def test_rejects_missing_file(self) -> None:
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            gate.load_report("/tmp/shapeport-missing-debtmap.json")
        self.assertEqual(caught.exception.code, 2)


class MainGateTests(unittest.TestCase):
    def _write(self, payload: dict) -> str:
        handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8")
        json.dump(payload, handle)
        handle.close()
        self.addCleanup(lambda: os.unlink(handle.name))
        return handle.name

    def test_passes_when_all_functions_at_or_below_threshold(self) -> None:
        path = self._write(
            report(items=[function_item("ok", 20), function_item("also_ok", 1)])
        )
        with redirect_stdout(io.StringIO()):
            self.assertEqual(gate.main(["cyclomatic_gate.py", path, "20"]), 0)

    def test_fails_when_function_exceeds_threshold(self) -> None:
        path = self._write(report(items=[function_item("hot", 21, line=40)]))
        with redirect_stdout(io.StringIO()):
            self.assertEqual(gate.main(["cyclomatic_gate.py", path, "20"]), 1)

    def test_refuses_empty_analysis(self) -> None:
        path = self._write(report(items=[], total_loc=0, function_count=0))
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            gate.main(["cyclomatic_gate.py", path, "20"])
        self.assertEqual(caught.exception.code, 2)

    def test_refuses_when_summary_functions_have_no_metrics(self) -> None:
        path = self._write(
            report(
                items=[{"type": "Function", "location": {"function": "x"}}],
                total_loc=10,
                function_count=1,
            )
        )
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            gate.main(["cyclomatic_gate.py", path, "20"])
        self.assertEqual(caught.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
