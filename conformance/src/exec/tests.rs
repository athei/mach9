use std::io::Write as _;

use super::*;

/// What the root wrote before it exited is all in the pipe, and the drain
/// reads all of it while a holder keeps the pipe open, waiting only for
/// the end, not for a quiet gap before the data.
#[test]
fn the_drain_reads_everything_in_the_pipe_while_a_holder_keeps_it_open() {
    let (mut reader, mut writer) = io::pipe().unwrap();
    let data: Vec<u8> = (0..60_000u32)
        .map(|i| b'a' + u8::try_from(i % 26).unwrap())
        .collect();
    writer.write_all(&data).unwrap();
    let start = Instant::now();
    let mut output = Vec::new();
    assert!(!drain(&mut reader, &mut output));
    assert_eq!(output, data);
    assert!(start.elapsed() < DRAIN_QUIET + Duration::from_millis(300));
    drop(writer);
}

#[test]
fn the_drain_sees_the_end_of_the_pipe() {
    let (mut reader, mut writer) = io::pipe().unwrap();
    writer.write_all(b"last line\n").unwrap();
    drop(writer);
    let mut output = Vec::new();
    assert!(drain(&mut reader, &mut output));
    assert_eq!(output, b"last line\n");
}

/// A member of the killed group that closes the pipe a moment later is
/// still seen as the end of the pipe.
#[test]
fn the_drain_waits_briefly_for_the_end() {
    let (mut reader, writer) = io::pipe().unwrap();
    let closer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        drop(writer);
    });
    let mut output = Vec::new();
    assert!(drain(&mut reader, &mut output));
    closer.join().unwrap();
}

/// A holder that keeps writing is cut off at the cap.
#[test]
fn the_drain_ends_at_the_cap_for_a_writing_holder() {
    let (mut reader, mut writer) = io::pipe().unwrap();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stopped = std::sync::Arc::clone(&stop);
    let chatter = thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            if writer.write_all(b"chatter\n").is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
    let start = Instant::now();
    let mut output = Vec::new();
    assert!(!drain(&mut reader, &mut output));
    let elapsed = start.elapsed();
    assert!(
        elapsed >= DRAIN_CAP && elapsed < DRAIN_CAP + Duration::from_millis(500),
        "{elapsed:?}, {} bytes",
        output.len()
    );
    stop.store(true, Ordering::SeqCst);
    discard_until_end(reader);
    chatter.join().unwrap();
}
