//! `conformance`: tooling for Wine's graphics conformance tests.
//!
//! Which executables and units exist is in `wine-tests.toml` ([`model`]).
//! `wine-macros` generates and checks the blocks in Wine's test sources that
//! let a single test function of a unit run alone ([`wine_src`]).
//!
//! A run lists the keys of each unit, runs each key in a process of its own
//! and reads its output ([`output`], [`compatdb_log`]) into an [`Outcome`]
//! per [`TestId`]. The outcomes of one executable are compared with its
//! committed [`baseline`] ([`diff`]) or recorded into it ([`record`]), with
//! the build they come from ([`header`]) and the notes of [`triage`];
//! [`resume`] keeps the finished outcomes of an interrupted run.
//!
//! [`runner`] is the `run` and `record` commands on top of that: it finds
//! the bundle and its state directory ([`layout`]), runs each process tree
//! under a watchdog ([`exec`]) with the scrubbed environment of
//! [`environment`] in one Wine session of a fresh prefix ([`session`],
//! [`procs`]), and stops cleanly on a signal ([`interrupt`]).
//!
//! [`Outcome`]: outcome::Outcome
//! [`TestId`]: outcome::TestId

pub mod baseline;
pub mod compatdb_log;
pub mod diff;
pub mod environment;
pub mod exec;
pub mod header;
pub mod interrupt;
pub mod layout;
pub mod model;
pub mod outcome;
pub mod output;
pub mod procs;
pub mod record;
pub mod resume;
pub mod runner;
pub mod session;
pub mod triage;
pub mod wine_src;
