#!/usr/bin/env python3
"""Require the expected cohort and the final complete XCTest All tests result."""
import argparse
import json
import pathlib
import re


def positive_count(value):
    count = int(value)
    if count <= 0:
        raise argparse.ArgumentTypeError('expected count must be positive')
    return count


def evaluate(text, expected):
    result = {'status': 'failed', 'expected': expected,
              'evidenceLevel': 'iOS simulator XCTest'}
    events = list(re.finditer(
        r"^Test Suite 'All tests' (started|passed|failed)\b[^\r\n]*(?:\r?\n|$)",
        text, re.MULTILINE))
    if not events or events[-1].group(1) == 'started':
        return {**result, 'reason': 'No complete final XCTest All tests suite'}
    final = events[-1]
    summary = re.match(
        r"\s*Executed (\d+) tests?, with (?:(\d+) tests? skipped and )?(\d+) failures?\b",
        text[final.end():])
    if not summary:
        return {**result, 'reason': 'No complete final XCTest All tests summary'}
    total, skipped_summary, failure_count = (int(value or 0) for value in summary.groups())
    # Earlier complete runs are historical. An unfinished newer run must not
    # borrow their success, and failure/skip output after a summary invalidates it.
    starts = [event for event in events if event.group(1) == 'started']
    current = text[starts[-1].start() if starts else 0:]
    case_results = re.findall(r'^Test Case .+? (passed|failed|skipped)\b', current, re.MULTILINE)
    skipped = max(skipped_summary, case_results.count('skipped'))
    failed_cases = case_results.count('failed')
    conflicting = re.search(r'^Test Suite .+? failed\b', current, re.MULTILINE)
    trailing = re.search(r'^Test (?:Suite|Case) .+? (?:started|failed|skipped)\b',
                         text[final.end():], re.MULTILINE)
    ok = (final.group(1) == 'passed' and total == expected and failure_count == 0
          and skipped == 0 and failed_cases == 0 and not conflicting and not trailing)
    # XCTest's summary counts assertion failures, which need not equal failed
    # test methods. Only infer method counts from complete case output or a pass.
    complete_cases = len(case_results) == total
    passed = case_results.count('passed') if complete_cases else (total if ok else None)
    failed = failed_cases if complete_cases or failure_count == 0 else None
    return {**result, 'status': 'passed' if ok else 'failed', 'total': total,
            'passed': passed, 'failed': failed, 'failureCount': failure_count, 'skipped': skipped,
            **({} if ok else {'reason': 'Final suite must pass every expected test without failures, skips or trailing incomplete output'})}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log', type=pathlib.Path)
    parser.add_argument('--expected-count', type=positive_count, default=163)
    args = parser.parse_args()
    result = evaluate(args.log.read_text(errors='replace'), args.expected_count)
    print(json.dumps(result))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
