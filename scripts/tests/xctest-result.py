#!/usr/bin/env python3
"""simctl launch can exit zero when XCTest fails; require its final suite result."""
import json, pathlib, re, sys

text = pathlib.Path(sys.argv[1]).read_text(errors='replace')
matches = re.findall(r"Test Suite 'All tests' (passed|failed)[^\n]*\n\s*Executed (\d+) tests?, with (\d+) failures?", text)
if not matches:
    print(json.dumps({'status':'failed','reason':'No complete XCTest All tests summary'}))
    raise SystemExit(1)
status, total, failed = matches[-1]
total, failed = int(total), int(failed)
skipped = len(re.findall(r"Test Case [^\n]+ skipped", text))
ok = status == 'passed' and total > 0 and failed == 0 and skipped == 0
print(json.dumps({'status':'passed' if ok else 'failed','total':total,
                  'passed':total-failed-skipped,'failed':failed,'skipped':skipped,
                  'evidenceLevel':'iOS simulator XCTest'}))
raise SystemExit(0 if ok else 1)
