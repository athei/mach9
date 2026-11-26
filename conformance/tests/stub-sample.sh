#!/bin/sh
# Stands in for macOS's `sample` in tests/exec.rs: writes its arguments as
# the report to the file after `-file` and prints a message, as `sample`
# does. Without `-file` it fails, since `sample` then also keeps a copy of
# the report in /tmp.
file=
prev=
for arg in "$@"; do
    if [ "$prev" = -file ]; then
        file=$arg
    fi
    prev=$arg
done
if [ -z "$file" ]; then
    echo "stub sample: no -file" >&2
    exit 1
fi
echo "stub sample of $*" > "$file"
echo "stub sample written to file $file"
