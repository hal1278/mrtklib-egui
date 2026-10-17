//! State-to-display mapping, independent of egui. All display text leaves through mask.

use mrtklib_egui_rpc::{
    Command, Connection, Model, RequestError, SolutionStatus, State, StopReason,
};
use mrtklib_egui_supervisor::{Readiness, State as ProcessState, mask::mask};

use crate::time;

pub(crate) type Color = [u8; 3];
pub(crate) const GRAY: Color = [128, 128, 128];
const BLACK: Color = [0, 0, 0];
const WHITE: Color = [255, 255, 255];

pub(crate) struct View {
    pub times: [String; 2],
    pub time_color: Color,
    pub solution_label: String,
    pub solution: String,
    pub solution_color: Color,
    pub indicator: Color,
    pub rows: [(String, String); 4],
    pub quality: String,
    pub action: Command,
    pub action_label: String,
    pub action_enabled: bool,
    pub options_label: String,
    pub options_hint: String,
    pub exit_label: String,
    pub status: String,
}

pub(crate) fn build(
    model: &Model,
    process: Option<&ProcessState>,
    preview: &str,
    launch_error: Option<&str>,
    closing: bool,
    token: &str,
) -> View {
    let state = model.lifecycle.as_ref().map(|l| l.state);
    let connected = matches!(model.connection, Connection::Connected { .. })
        && process.is_none_or(|p| {
            p.exit_status.is_none() && matches!(p.readiness, Readiness::Ready { .. })
        });
    let running = connected && state == Some(State::Running) && !closing;
    let status = model
        .solution
        .as_ref()
        .map_or(SolutionStatus::None, |s| s.status);
    let (solution, color) = solution_style(status);
    let action = if state.is_none() || state == Some(State::Stopped) {
        Command::Start
    } else {
        Command::Stop
    };
    let mut view = View {
        times: model.last_valid_solution.as_ref().map_or_else(
            || ["GPST ---- ----.---".into(), "UTC ----".into()],
            |s| time::format(s.time, s.leap_seconds),
        ),
        time_color: if running { BLACK } else { GRAY },
        solution_label: "Solution:".into(),
        solution: solution.into(),
        solution_color: if running { color } else { GRAY },
        indicator: if running && status != SolutionStatus::None {
            color
        } else {
            WHITE
        },
        rows: [
            ("Lat:".into(), "----".into()),
            ("Lon:".into(), "----".into()),
            ("He:".into(), "---- m".into()),
            ("σ".into(), "E:---- N:---- U:---- m".into()),
        ],
        quality: "Age:---- s Ratio:---- #Sat:--".into(),
        action,
        action_label: if action == Command::Start {
            "Start"
        } else {
            "Stop"
        }
        .into(),
        action_enabled: connected
            && !closing
            && matches!(state, Some(State::Stopped | State::Running)),
        options_label: "Options...".into(),
        options_hint: "Configuration is not available over RPC yet".into(),
        exit_label: "Exit".into(),
        status: status_line(model, process, preview, launch_error, closing),
    };
    if let Some(s) = &model.last_valid_solution {
        if let Some(position) = s.position {
            let llh = position.llh;
            view.rows[0].1 = format!("{:.8}°", llh.lat);
            view.rows[1].1 = format!("{:.8}°", llh.lon);
            view.rows[2].1 = format!("{:.3} m", llh.height);
        }
        if let Some(covariance) = s.covariance {
            let enu = covariance.enu;
            view.rows[3].1 = format!(
                "E:{:6.3} N:{:6.3} U:{:6.3} m",
                enu.ee.max(0.0).sqrt(),
                enu.nn.max(0.0).sqrt(),
                enu.uu.max(0.0).sqrt()
            );
        }
        view.quality = format!(
            "Age:{} s Ratio:{} #Sat:{}",
            s.age.map_or_else(|| "----".into(), |v| format!("{v:4.1}")),
            s.ratio
                .map_or_else(|| "----".into(), |v| format!("{v:4.1}")),
            s.valid_satellites
                .map_or_else(|| "--".into(), |v| format!("{v:2}")),
        );
    }
    view.mask(token);
    view
}

fn solution_style(status: SolutionStatus) -> (&'static str, Color) {
    let q = match status {
        SolutionStatus::None => 0,
        SolutionStatus::Fix => 1,
        SolutionStatus::Float => 2,
        SolutionStatus::Sbas => 3,
        SolutionStatus::Dgps => 4,
        SolutionStatus::Single => 5,
        SolutionStatus::Ppp => 6,
        SolutionStatus::Dr => 7,
    };
    mrtklib_egui_plotkit::solution_style(q)
}

fn status_line(
    model: &Model,
    process: Option<&ProcessState>,
    preview: &str,
    launch_error: Option<&str>,
    closing: bool,
) -> String {
    let backend = if let Some(error) = launch_error {
        format!("Backend error: {error}")
    } else if let Some(p) = process
        && let Readiness::Failed(error) = &p.readiness
    {
        let output = p.lines.back().map_or("", |line| line.text.as_str());
        format!("Backend error: {error}; last output: {output}")
    } else if let Some(p) = process
        && let Some(exit) = p.exit_status
    {
        format!("Backend server exited: {exit}")
    } else {
        match &model.connection {
            Connection::Connecting => format!("Backend starting: {preview}"),
            Connection::Connected { version } => format!(
                "Backend connected: {preview}; MRTKLIB {}; protocol {}",
                version.mrtklib_version, version.protocol_version
            ),
            Connection::ServerExited => "Backend server exited".into(),
            Connection::Disconnected { error } => format!("Backend error: {error}"),
        }
    };
    let mut parts = vec![backend];
    if let Some(p) = process
        && let Some(error) = p.errors.back()
    {
        parts.push(format!("Process: {error}"));
    }
    if let Some(lifecycle) = &model.lifecycle {
        let state = match lifecycle.state {
            State::Stopped => "stopped",
            State::Starting => "starting",
            State::Running => "running",
            State::Stopping => "stopping",
        };
        parts.push(format!("Lifecycle: {state}"));
        if let Some(reason) = lifecycle.stop_reason {
            parts.push(format!(
                "stopReason: {}",
                match reason {
                    StopReason::Request => "request",
                    StopReason::InputEnd => "inputEnd",
                    StopReason::Error => "error",
                }
            ));
        }
        if let Some(error) = &lifecycle.last_error {
            parts.push(format!("lastError: {}: {}", error.reason, error.message));
            if !error.paths.is_empty() {
                parts.push(format!("Paths: {}", error.paths.join(", ")));
            }
        }
    }
    if let Some(outcome) = &model.last_request
        && let Err(error) = &outcome.result
    {
        let error = match error {
            RequestError::Rpc(error) => format!(
                "{}: {}",
                error.reason.as_deref().unwrap_or("unspecified reason"),
                error.message
            ),
            RequestError::Connection(error) => format!("connection: {error}"),
        };
        parts.push(format!("Last request {:?}: {error}", outcome.command));
    }
    if let Some(mode) = &model.mode {
        parts.push(format!("Mode: {mode}"));
    }
    if model.lost_events != 0 {
        parts.push(format!("Lost events: {}", model.lost_events));
    }
    if closing {
        parts.push("Closing: waiting for backend shutdown".into());
    }
    parts.join(" | ")
}

impl View {
    /// The single UI text boundary, including labels and hover text. Supervisor
    /// snapshots are already masked; applying the same mask again is harmless.
    fn mask(&mut self, token: &str) {
        for text in self
            .times
            .iter_mut()
            .chain([
                &mut self.solution_label,
                &mut self.solution,
                &mut self.quality,
                &mut self.action_label,
                &mut self.options_label,
                &mut self.options_hint,
                &mut self.exit_label,
                &mut self.status,
            ])
            .chain(
                self.rows
                    .iter_mut()
                    .flat_map(|(label, value)| [label, value]),
            )
        {
            *text = mask(text, &[token]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrtklib_egui_rpc::{
        Authentication, Covariance, CovarianceEcef, CovarianceEnu, Ecef, GpsTime, LastError,
        Lifecycle, Llh, Position, RequestOutcome, RpcError, Solution, Version,
    };
    use std::collections::VecDeque;

    fn model(state: State) -> Model {
        let mut model = Model {
            connection: Connection::Connected {
                version: Version {
                    protocol_version: "0.1".into(),
                    mrtklib_version: "mock 1".into(),
                    authentication: Authentication::Token,
                },
            },
            lifecycle: Some(Lifecycle {
                state,
                stop_reason: None,
                last_error: None,
            }),
            solution: Some(Solution {
                time: GpsTime {
                    week: 0,
                    tow: 0.123,
                },
                leap_seconds: 0,
                status: SolutionStatus::Fix,
                position: Some(Position {
                    ecef: Ecef {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    llh: Llh {
                        lat: -35.123456789,
                        lon: -139.123456789,
                        height: 12.3456,
                    },
                }),
                covariance: Some(Covariance {
                    ecef: CovarianceEcef {
                        xx: 0.0,
                        yy: 0.0,
                        zz: 0.0,
                        xy: 0.0,
                        yz: 0.0,
                        zx: 0.0,
                    },
                    enu: CovarianceEnu {
                        ee: 4.0,
                        nn: -1.0,
                        uu: 0.0004,
                        en: 0.0,
                        nu: 0.0,
                        ue: 0.0,
                    },
                }),
                geoid: None,
                valid_satellites: Some(9),
                age: Some(1.24),
                ratio: Some(3.56),
                base: None,
            }),
            ..Model::default()
        };
        model.last_valid_solution = model.solution.clone();
        model
    }

    fn show(model: &Model) -> View {
        build(model, None, "/suite/mrtk run", None, false, "test-secret")
    }

    #[test]
    fn exact_position_uncertainty_and_quality_formats() {
        let view = show(&model(State::Running));
        assert_eq!(view.rows[0], ("Lat:".into(), "-35.12345679°".into()));
        assert_eq!(view.rows[1], ("Lon:".into(), "-139.12345679°".into()));
        assert_eq!(view.rows[2], ("He:".into(), "12.346 m".into()));
        assert_eq!(view.rows[3].1, "E: 2.000 N: 0.000 U: 0.020 m");
        assert_eq!(view.quality, "Age: 1.2 s Ratio: 3.6 #Sat: 9");
        let mut north_east = model(State::Running);
        north_east
            .last_valid_solution
            .as_mut()
            .unwrap()
            .position
            .as_mut()
            .unwrap()
            .llh = Llh {
            lat: 35.0,
            lon: 139.0,
            height: -1.0,
        };
        assert_eq!(
            show(&north_east).rows[0],
            ("Lat:".into(), "35.00000000°".into())
        );
        assert_eq!(
            show(&north_east).rows[1],
            ("Lon:".into(), "139.00000000°".into())
        );
    }

    #[test]
    fn all_status_labels_and_colors_and_stopped_display() {
        for (status, label, color) in [
            (SolutionStatus::None, "----", [192, 192, 192]),
            (SolutionStatus::Fix, "FIX", [0, 128, 0]),
            (SolutionStatus::Float, "FLOAT", [255, 170, 0]),
            (SolutionStatus::Sbas, "SBAS", [255, 0, 255]),
            (SolutionStatus::Dgps, "DGPS", [0, 0, 255]),
            (SolutionStatus::Single, "SINGLE", [255, 0, 0]),
            (SolutionStatus::Ppp, "PPP", [0, 128, 128]),
            (SolutionStatus::Dr, "DR", GRAY),
        ] {
            let mut model = model(State::Running);
            model.solution.as_mut().unwrap().status = status;
            let running = show(&model);
            assert_eq!(running.solution, label);
            assert_eq!(running.solution_color, color);
            assert_eq!(running.time_color, BLACK);
            assert_eq!(
                running.indicator,
                if status == SolutionStatus::None {
                    WHITE
                } else {
                    color
                }
            );
            model.lifecycle.as_mut().unwrap().state = State::Stopped;
            let stopped = show(&model);
            assert_eq!(stopped.solution, label);
            assert_eq!(stopped.solution_color, GRAY);
            assert_eq!(stopped.time_color, GRAY);
            assert_eq!(stopped.indicator, WHITE);
            assert_eq!(stopped.rows, running.rows);
            assert_eq!(stopped.times, running.times);
        }
    }

    #[test]
    fn none_keeps_the_last_valid_display_but_uses_the_current_status() {
        let mut model = model(State::Running);
        let fix = show(&model);
        let latest = model.solution.as_mut().unwrap();
        latest.status = SolutionStatus::None;
        latest.time.tow = 10.0;
        latest.position = None;
        latest.covariance = None;
        latest.age = None;
        latest.ratio = None;
        latest.valid_satellites = None;
        let none = show(&model);
        assert_eq!(none.times, fix.times);
        assert_eq!(none.rows, fix.rows);
        assert_eq!(none.quality, fix.quality);
        assert_eq!(none.solution, "----");
        assert_eq!(none.solution_color, [192, 192, 192]);
        assert_eq!(none.indicator, WHITE);
        assert_eq!(none.time_color, BLACK);

        model.lifecycle.as_mut().unwrap().state = State::Stopped;
        let stopped = show(&model);
        assert_eq!(stopped.times, fix.times);
        assert_eq!(stopped.rows, fix.rows);
        assert_eq!(stopped.quality, fix.quality);
        assert_eq!(stopped.solution, "----");
        assert_eq!(stopped.solution_color, GRAY);
        assert_eq!(stopped.time_color, GRAY);
        assert_eq!(stopped.indicator, WHITE);
    }

    #[test]
    fn none_before_a_valid_solution_shows_only_placeholders() {
        let mut model = model(State::Running);
        model.last_valid_solution = None;
        // Even a malformed NONE carrying coordinates and quality cannot show them.
        model.solution.as_mut().unwrap().status = SolutionStatus::None;
        let none = show(&model);
        assert_eq!(none.rows[0].1, "----");
        assert_eq!(none.rows[1].1, "----");
        assert_eq!(none.rows[2].1, "---- m");
        assert_eq!(none.rows[3].1, "E:---- N:---- U:---- m");
        assert_eq!(none.quality, "Age:---- s Ratio:---- #Sat:--");
        assert_eq!(none.times, ["GPST ---- ----.---", "UTC ----"]);
        model.solution = None;
        assert_eq!(show(&model).rows, none.rows);
        assert_eq!(show(&model).times[0], "GPST ---- ----.---");
    }

    #[test]
    fn starting_model_shows_disabled_stop_and_cleared_solution() {
        // Session clears both solutions and the previous run's mode on Starting.
        // Construct that model directly rather than racing a live event poll.
        let mut model = model(State::Starting);
        model.solution = None;
        model.last_valid_solution = None;
        model.mode = None;
        let view = show(&model);
        assert_eq!(view.action, Command::Stop);
        assert_eq!(view.action_label, "Stop");
        assert!(!view.action_enabled);
        assert_eq!(view.solution, "----");
        assert_eq!(view.times, ["GPST ---- ----.---", "UTC ----"]);
        assert_eq!(view.rows[0].1, "----");
        assert_eq!(view.rows[1].1, "----");
        assert_eq!(view.rows[2].1, "---- m");
        assert_eq!(view.rows[3].1, "E:---- N:---- U:---- m");
        assert_eq!(view.quality, "Age:---- s Ratio:---- #Sat:--");
        assert_eq!(view.time_color, GRAY);
        assert_eq!(view.solution_color, GRAY);
        assert_eq!(view.indicator, WHITE);
        assert!(view.status.contains("Lifecycle: starting"));
        assert!(!view.status.contains("Mode:"));
    }

    #[test]
    fn button_swapping_requires_connected_stable_lifecycle() {
        for (state, label, enabled) in [
            (State::Stopped, "Start", true),
            (State::Starting, "Stop", false),
            (State::Running, "Stop", true),
            (State::Stopping, "Stop", false),
        ] {
            let mut model = model(state);
            let view = show(&model);
            assert_eq!(view.action_label, label);
            assert_eq!(view.action_enabled, enabled);
            for connection in [
                Connection::Connecting,
                Connection::ServerExited,
                Connection::Disconnected {
                    error: "failed".into(),
                },
            ] {
                model.connection = connection;
                assert!(!show(&model).action_enabled);
            }
        }
        assert!(!show(&Model::default()).action_enabled);
        assert!(!build(&model(State::Running), None, "mrtk", None, true, "secret").action_enabled);
    }

    #[test]
    fn status_contains_path_versions_lifecycle_errors_mode_and_losses_and_masks() {
        let mut model = model(State::Stopped);
        model.mode = Some("kinematic test-secret".into());
        model.lost_events = 3;
        model.lifecycle.as_mut().unwrap().stop_reason = Some(StopReason::Error);
        model.lifecycle.as_mut().unwrap().last_error = Some(LastError {
            paths: vec![],
            reason: "startFailed".into(),
            message: "test-secret ntrip://user:pass@host/mount".into(),
        });
        model.last_request = Some(RequestOutcome {
            command: Command::Start,
            accepted_seq: None,
            start_outcome: None,
            result: Err(RequestError::Rpc(RpcError {
                code: 1001,
                reason: Some("busy".into()),
                message: "test-secret".into(),
                state: Some(State::Starting),
            })),
        });
        let status = show(&model).status;
        for text in [
            "/suite/mrtk run",
            "MRTKLIB mock 1",
            "protocol 0.1",
            "Lifecycle: stopped",
            "stopReason: error",
            "lastError: startFailed",
            "Last request Start: busy",
            "Mode: kinematic",
            "Lost events: 3",
            "ntrip://***@host/mount",
        ] {
            assert!(status.contains(text), "missing {text}: {status}");
        }
        assert!(!status.contains("test-secret"));
        assert!(!status.contains("user:pass"));
        let masked_label = build(&model, None, "mrtk", None, false, "Solution");
        assert_eq!(masked_label.solution_label, "***:");
    }

    #[test]
    fn startup_failure_includes_last_output_and_disables_buttons() {
        let process = ProcessState {
            lines: VecDeque::from([mrtklib_egui_supervisor::OutputLine {
                stream: mrtklib_egui_supervisor::Stream::Stderr,
                text: "listen failed test-secret".into(),
            }]),
            dropped_lines: 0,
            readiness: Readiness::Failed("startup failed".into()),
            exit_status: None,
            errors: VecDeque::new(),
            dropped_errors: 0,
        };
        let view = build(
            &model(State::Stopped),
            Some(&process),
            "mrtk",
            None,
            false,
            "test-secret",
        );
        assert!(view.status.contains("last output: listen failed ***"));
        assert!(!view.action_enabled);
        let view = build(
            &Model::default(),
            None,
            "",
            Some("cannot locate GUI"),
            false,
            "secret",
        );
        assert!(view.status.contains("Backend error: cannot locate GUI"));
    }
}
