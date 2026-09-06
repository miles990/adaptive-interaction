#!/usr/bin/env python3
"""Deterministic XCTest log-gate regressions; no Xcode or simulator is launched."""
import pathlib
import subprocess
import sys
import tempfile
import unittest

PARSER = pathlib.Path(__file__).with_name('xctest-result.py')


def suite(total=163, status='passed', failures=0):
    return ("Test Suite 'All tests' started at 2026-09-06 13:00:00.000.\n"
            f"Test Suite 'All tests' {status} at 2026-09-06 13:00:01.000.\n"
            f"\t Executed {total} tests, with {failures} failures (0 unexpected) in 1.000 seconds\n")


class XCTestResultTests(unittest.TestCase):
    def check_result(self, text, passes, expected=None):
        with tempfile.TemporaryDirectory(prefix='aip-xctest-log-test-') as directory:
            log = pathlib.Path(directory)/'xctest.log'
            log.write_text(text)
            command = [sys.executable, str(PARSER), str(log)]
            if expected is not None:
                command += ['--expected-count', str(expected)]
            result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, passes, result.stdout + result.stderr)

    def test_expected_complete_suite_passes(self):
        self.check_result(suite(), True)

    def test_missing_tests_are_not_a_pass(self):
        self.check_result(suite(1), False)

    def test_explicit_count_allows_another_deliberate_cohort(self):
        self.check_result(suite(1), True, expected=1)

    def test_zero_expected_count_is_invalid(self):
        self.check_result(suite(), False, expected=0)

    def test_zero_executed_tests_are_not_a_pass(self):
        self.check_result(suite(0), False)

    def test_reported_failure_is_not_a_pass(self):
        self.check_result(suite(status='failed', failures=1), False)

    def test_missing_or_truncated_summary_is_not_a_pass(self):
        self.check_result('', False)
        self.check_result("Test Suite 'All tests' passed at 2026-09-06\n Executed 163 tests", False)

    def test_new_unfinished_suite_cannot_reuse_an_earlier_pass(self):
        self.check_result(suite() + "Test Suite 'All tests' started at 2026-09-06\n", False)

    def test_new_failed_suite_cannot_reuse_an_earlier_pass(self):
        self.check_result(suite() + suite(status='failed', failures=1), False)

    def test_skip_after_success_is_rejected(self):
        self.check_result(suite() + "Test Case '-[Example testSkipped]' skipped (0.000 seconds).\n", False)

    def test_failed_case_after_success_is_rejected(self):
        self.check_result(suite() + "Test Case '-[Example testFailed]' failed (0.000 seconds).\n", False)

    def test_latest_complete_suite_is_the_one_counted(self):
        self.check_result(suite(1, status='failed', failures=1) + suite(), True)

    def test_crlf_summary_is_accepted(self):
        self.check_result(suite().replace('\n', '\r\n'), True)

    def test_summary_with_a_skipped_test_is_rejected(self):
        self.check_result(suite().replace('with 0 failures', 'with 1 test skipped and 0 failures'), False)


if __name__ == '__main__':
    unittest.main(verbosity=2)
