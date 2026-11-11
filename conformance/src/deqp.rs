//! The deqp-vk suite: cases of Khronos' Vulkan CTS run on KosmicKrisp.
//!
//! Which cases run is `deqp-vk.toml` ([`groups`]), a set of patterns over
//! the vk-default mustpass of the pinned VK-GL-CTS. [`run`] runs the cases
//! of the selected groups with `deqp-vk` from `bin/build-vk-gl-cts.sh`, in
//! batches of one process each, and reads each process's output
//! ([`log`]). The result of each group is compared with its committed
//! expected results, or recorded into them ([`expected`]), with the notes
//! of its [`triage`] file.
//!
//! The suite shares the result vocabulary of the Wine suites: its
//! [`Verdict`](crate::diff::Verdict) and its
//! [`Conclusion`](crate::runner::Conclusion), so the exit codes and the
//! last line mean what they mean there.

pub mod expected;
pub mod groups;
pub mod log;
pub mod probe;
pub mod run;
pub mod triage;
