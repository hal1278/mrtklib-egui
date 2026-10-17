//! Pure display mapping. Child lines have already been masked by the supervisor.

use mrtklib_egui_supervisor::{State, Stream, mask::mask};

#[derive(Default)]
pub(crate) struct View {
    pub message: String,
    pub lines: Vec<String>,
    pub dropped: u64,
}

pub(crate) fn build(process: Option<&State>, error: Option<&str>) -> View {
    let mut view = View::default();
    if let Some(process) = process {
        view.lines = process
            .lines
            .iter()
            .filter(|line| line.stream == Stream::Stderr)
            .map(|line| line.text.clone())
            .collect();
        view.dropped = process.dropped_lines;
        view.message = view.lines.last().cloned().unwrap_or_default();
        if let Some(status) = process.exit_status {
            if !view.message.is_empty() {
                view.message.push_str(" | ");
            }
            let exit = match status.code() {
                Some(255) => "exit 255 (-1)".into(),
                Some(code) => format!("exit {code}"),
                None => status.to_string(),
            };
            view.message.push_str(&exit);
        }
        for error in &process.errors {
            view.lines.push(format!("Supervisor: {error}"));
        }
        if let Some(error) = process.errors.back() {
            if !view.message.is_empty() {
                view.message.push_str(" | ");
            }
            view.message.push_str(error);
        }
    }
    if let Some(error) = error {
        view.message = mask(error, &[]);
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrtklib_egui_supervisor::{OutputLine, Readiness};
    use std::collections::VecDeque;

    #[test]
    fn last_stderr_exit_status_and_supervisor_errors() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        #[cfg(unix)]
        let status = std::process::ExitStatus::from_raw(255 << 8);
        #[cfg(windows)]
        let status = std::process::ExitStatus::from_raw(255);
        let mut state = State {
            lines: VecDeque::from([
                OutputLine {
                    stream: Stream::Stderr,
                    text: "first".into(),
                },
                OutputLine {
                    stream: Stream::Stderr,
                    text: "last".into(),
                },
                OutputLine {
                    stream: Stream::Stdout,
                    text: "stdout is not stderr".into(),
                },
            ]),
            dropped_lines: 3,
            readiness: Readiness::NotRequired,
            exit_status: Some(status),
            errors: VecDeque::new(),
            dropped_errors: 0,
        };
        let view = build(Some(&state), None);
        assert_eq!(view.message, "last | exit 255 (-1)");
        assert_eq!(view.lines, ["first", "last"]);
        assert_eq!(view.dropped, 3);
        state.exit_status = Some(std::process::ExitStatus::from_raw(0));
        assert_eq!(build(Some(&state), None).message, "last | exit 0");
        state.errors.push_back("read error".into());
        assert!(build(Some(&state), None).message.ends_with("read error"));
        assert_eq!(
            build(Some(&state), Some("user:pass@host")).message,
            "***@host"
        );
        #[cfg(unix)]
        {
            state.exit_status = Some(std::process::ExitStatus::from_raw(9));
            assert!(build(Some(&state), None).message.contains("signal"));
        }
    }
}
