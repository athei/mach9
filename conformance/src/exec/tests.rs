use std::io::Write as _;

use super::*;

/// A limit of [`drain_within`] that a test does not test: long enough that
/// no slow thread of a loaded machine or under the background policy
/// reaches it.
const LONG: Duration = Duration::from_secs(30);

/// A limit of [`drain_within`] that a test does test.
const SHORT: Duration = Duration::from_millis(200);

/// What the root wrote before it exited is all in the pipe, and the drain
/// reads all of it while a holder keeps the pipe open, waiting only for
/// the end, not for a quiet gap before the data: it ends at the quiet
/// limit, not at the cap.
#[test]
fn the_drain_reads_everything_in_the_pipe_while_a_holder_keeps_it_open() {
    let (mut reader, mut writer) = io::pipe().unwrap();
    let data: Vec<u8> = (0..60_000u32)
        .map(|i| b'a' + u8::try_from(i % 26).unwrap())
        .collect();
    writer.write_all(&data).unwrap();
    let start = Instant::now();
    let mut output = Vec::new();
    assert!(!drain_within(&mut reader, &mut output, SHORT, LONG));
    assert_eq!(output, data);
    let elapsed = start.elapsed();
    assert!(elapsed >= SHORT && elapsed < LONG, "{elapsed:?}");
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

/// Whether the pipe of `watch`, a second handle of its read end, holds
/// nothing to read: the drain has read all of it.
fn read_out(watch: &PipeReader) -> bool {
    let mut fds = [PollFd::new(watch.as_fd(), PollFlags::POLLIN)];
    poll(&mut fds, PollTimeout::ZERO) == Ok(0)
}

/// A member of the killed group that closes the pipe a while after the
/// drain has read what was in it is still seen as the end of the pipe:
/// the drain waits up to its quiet limit for the end.
///
/// The closer waits until the drain has emptied the pipe and then 50 ms
/// more, so the end comes while the drain waits, not at its first look.
/// The quiet limit is [`LONG`], so a sleep stretched by a loaded machine or
/// the background policy cannot outlast it.
#[test]
fn the_drain_waits_for_the_end_up_to_its_quiet_limit() {
    let (mut reader, mut writer) = io::pipe().unwrap();
    writer.write_all(b"last line\n").unwrap();
    let watch = reader.try_clone().unwrap();
    let closer = thread::spawn(move || {
        while !read_out(&watch) {
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(50));
        drop(writer);
    });
    let start = Instant::now();
    let mut output = Vec::new();
    assert!(drain_within(&mut reader, &mut output, LONG, LONG));
    assert!(start.elapsed() < LONG);
    assert_eq!(output, b"last line\n");
    closer.join().unwrap();
}

/// A holder that keeps writing is cut off at the cap.
///
/// The holder writes its next line once the drain has read the last one,
/// which keeps the output small. The quiet limit is [`LONG`], so the cap
/// ends the drain however long the holder's thread waits between lines.
#[test]
fn the_drain_ends_at_the_cap_for_a_writing_holder() {
    let (mut reader, mut writer) = io::pipe().unwrap();
    writer.write_all(b"chatter\n").unwrap();
    let watch = reader.try_clone().unwrap();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stopped = std::sync::Arc::clone(&stop);
    let chatter = thread::spawn(move || {
        loop {
            while !stopped.load(Ordering::SeqCst) && !read_out(&watch) {
                thread::yield_now();
            }
            if stopped.load(Ordering::SeqCst) || writer.write_all(b"chatter\n").is_err() {
                return;
            }
        }
    });
    let start = Instant::now();
    let mut output = Vec::new();
    assert!(!drain_within(&mut reader, &mut output, LONG, SHORT));
    let elapsed = start.elapsed();
    assert!(
        elapsed >= SHORT && elapsed < LONG,
        "{elapsed:?}, {} bytes",
        output.len()
    );
    assert!(output.starts_with(b"chatter\n"), "{} bytes", output.len());
    stop.store(true, Ordering::SeqCst);
    discard_until_end(reader);
    chatter.join().unwrap();
}
