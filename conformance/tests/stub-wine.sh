#!/bin/sh
# Stands in for the bundle's `wine` in the tests of the runner's process
# handling (tests/exec.rs): `stub-wine.sh <exe> <unit>` prints what a test
# executable built with the selection blocks prints, chosen by WT_LIST and
# WT_ONLY, in the formats of include/wine/test.h.
unit="$2"

summary() {
    echo "0024:$unit: $1 tests executed (0 marked as todo, 0 as flaky, $2 failures), 0 skipped."
}

if [ -n "$WT_LIST" ]; then
    echo "0024:wt-list:test_pass"
    echo "0024:wt-list:test_fail"
    summary 0 0
    exit 0
fi

case "$WT_ONLY" in
    test_pass)
        echo "0024:wt-run:test_pass"
        echo "stub.c:10: a trace on stderr" >&2
        echo "stub.c:11: a trace on stdout"
        summary 3 0
        exit 0
        ;;
    test_fail)
        echo "0024:wt-run:test_fail"
        echo "stub.c:12: Test failed: expected 1, got 2" >&2
        summary 3 1
        exit 1
        ;;
    test_crash)
        echo "0024:wt-run:test_crash"
        exit 5
        ;;
    test_silent)
        echo "0024:wt-run:test_silent"
        exec sleep 30
        ;;
    test_chatty)
        echo "0024:wt-run:test_chatty"
        while :; do
            echo "stub.c:13: still busy"
            sleep 0.2
        done
        ;;
    test_child)
        # A child in the test's process group keeps the pipe open; the
        # group kill after the root's exit ends it.
        echo "0024:wt-run:test_child"
        sleep 30 &
        summary 1 0
        exit 0
        ;;
    test_holder)
        # A child in a process group of its own keeps the pipe open, as
        # explorer.exe does; only the bounded drain ends the wait. It
        # writes its pid to $STUB_READY once it left the group, and the
        # stub ends only then, so the group kill cannot reach it first.
        echo "0024:wt-run:test_holder"
        perl -e 'setpgrp(0, 0); open(my $f, ">", "$ARGV[0].new") or die;
                 print $f "$$\n"; close $f; rename("$ARGV[0].new", $ARGV[0]);
                 sleep 10' "$STUB_READY" &
        while [ ! -f "$STUB_READY" ]; do
            sleep 0.05
        done
        summary 1 0
        exit 0
        ;;
    test_burst)
        # A large burst right before the exit: all of it is in the pipe
        # when the root ends.
        echo "0024:wt-run:test_burst"
        perl -e 'print "stub.c:14: burst line $_\n" for 1 .. 200000'
        summary 1 0
        exit 0
        ;;
    *)
        summary 0 0
        exit 0
        ;;
esac
