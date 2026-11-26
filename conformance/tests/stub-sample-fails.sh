#!/bin/sh
# Stands in for macOS's `sample` in tests/exec.rs when it cannot examine the
# process: it writes no report and says why.
echo "stub sample cannot examine process $1" >&2
exit 255
