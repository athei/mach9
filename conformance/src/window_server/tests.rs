use super::*;

/// `top -l 2 -s 1 -stats pid,ports -pid 89973 -pid 1`, cut down: the second
/// sample marks the counts that changed.
const TWO_SAMPLES: &str = "Processes: 915 total, 4 running, 911 sleeping, 4411 threads
2026/11/25 03:16:17
Load Avg: 4.63, 3.51, 3.85
MemRegions: 275327 total, 8500M resident, 1001M private, 4617M shared.

PID    #PORTS
89973  14217
1      5594
Processes: 916 total, 5 running, 911 sleeping, 4420 threads
2026/11/25 03:16:18
MemRegions: 79 total, 5760K resident, 1001M private, 4617M shared.

PID    #PORTS
89973  14225+
1      5590-
";

#[test]
fn top_gives_the_pid_and_count_of_each_process_line() {
    assert_eq!(
        parse_top(TWO_SAMPLES),
        [(89973, 14217), (1, 5594), (89973, 14225), (1, 5590)]
    );
}

#[test]
fn only_the_lines_under_a_ports_header_count() {
    let report = "Processes: 915 total\n123 456 not a table\n\nPID    #TH\n89973  30\n";
    assert_eq!(parse_top(report), []);
    let report = "PID    #PORTS\n89973  14217K\nabc    12\n1      5594\n";
    assert_eq!(parse_top(report), [(1, 5594)]);
}

#[test]
fn window_server_is_found_by_its_file_name() {
    let table = "    1 /sbin/launchd\n89973 \
                 /System/Library/PrivateFrameworks/SkyLight.framework/Resources/WindowServer\n  \
                 812 /tmp/WindowServer2\n  813 WindowServer\n  814 /tmp/NotWindowServer\n";
    assert_eq!(pids(table), [89973, 813]);
    assert_eq!(pids("    1 /sbin/launchd\n"), []);
}

#[test]
fn a_count_above_the_threshold_stops_the_run() {
    assert_eq!(over_threshold(3871), None);
    assert_eq!(over_threshold(PORT_THRESHOLD), None);
    let message = over_threshold(PORT_THRESHOLD + 1).unwrap_or_default();
    assert!(message.contains("250001 Mach ports"), "{message}");
    assert!(message.contains("log out and in"), "{message}");
    assert!(over_threshold(PORT_LIMIT).is_some());
}

#[test]
fn the_count_is_the_largest_of_the_window_servers() {
    let counts = [(1, 5594), (89973, 14217), (90000, 3871)];
    assert_eq!(count_of(&[89973], &counts), Ok(14217));
    assert_eq!(count_of(&[90000, 89973], &counts), Ok(14217));
}

#[test]
fn a_window_server_without_a_count_or_none_at_all_is_an_error() {
    let counts = [(1, 5594), (89973, 14217)];
    let error = count_of(&[89973, 812], &counts).unwrap_err();
    assert!(error.contains("WindowServer 812"), "{error}");
    let error = count_of(&[], &counts).unwrap_err();
    assert!(error.contains("no WindowServer runs"), "{error}");
}

#[test]
fn between_units_only_a_count_above_the_threshold_stops_the_run() {
    assert_eq!(between_units(Ok(14217)), Ok(None));
    assert_eq!(between_units(Ok(PORT_THRESHOLD)), Ok(None));
    let stop = between_units(Ok(PORT_THRESHOLD + 1)).unwrap_err();
    assert!(
        stop.contains("log out and in, or restart the Mac"),
        "{stop}"
    );
    let warning = between_units(Err(String::from("top ended with exit status: 1")))
        .unwrap()
        .unwrap_or_default();
    assert!(warning.contains("the run goes on"), "{warning}");
    assert!(
        warning.contains("top ended with exit status: 1"),
        "{warning}"
    );
}

#[test]
fn the_summary_gives_the_change_since_the_start() {
    assert_eq!(
        summary(14066, Ok(15012)),
        "conformance: WindowServer holds 15012 Mach ports, +946 since the start"
    );
    assert_eq!(
        summary(14066, Ok(14000)),
        "conformance: WindowServer holds 14000 Mach ports, -66 since the start"
    );
    assert_eq!(
        summary(14066, Err(String::from("no WindowServer runs"))),
        "conformance: cannot read WindowServer's Mach ports: no WindowServer runs"
    );
}
