//! Reads the output of one `deqp-vk` process: the result of each case it
//! finished, and the case it was in when it ended.
//!
//! deqp-vk prints, for each case, a start line and then a result line
//! (`tcuTestSessionExecutor.cpp`):
//!
//! ```text
//! Test case 'dEQP-VK.draw.renderpass.simple_draw.simple_draw_triangle_list'..
//!   Pass (Rendering succeeded)
//! ```
//!
//! Its stdout and stderr arrive on one pipe, so the driver's and the
//! loader's lines can come between the two; a line counts as the result
//! only when it starts with two spaces and a result name deqp knows,
//! followed by ` (`.

use crate::deqp::expected::Status;

/// What one process's output says.
#[derive(PartialEq, Eq, Debug)]
pub struct Output {
    /// Each finished case with its status, in output order.
    pub results: Vec<(String, Status)>,
    /// The case that started last and printed no result: the one the
    /// process was in when it ended.
    pub unfinished: Option<String>,
}

/// Read the output `text` of a deqp-vk process.
#[must_use]
pub fn parse(text: &str) -> Output {
    let mut results = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(case) = line
            .strip_prefix("Test case '")
            .and_then(|rest| rest.strip_suffix("'.."))
        {
            current = Some(case.to_owned());
            continue;
        }
        let Some(status) = result_line(line) else {
            continue;
        };
        if let Some(case) = current.take() {
            results.push((case, status));
        }
    }
    Output {
        results,
        unfinished: current,
    }
}

/// The status of a result line, `  <Result> (<description>)`.
fn result_line(line: &str) -> Option<Status> {
    let rest = line.strip_prefix("  ")?;
    let (name, _) = rest.split_once(" (")?;
    Status::from_deqp(name)
}

#[cfg(test)]
mod tests;
