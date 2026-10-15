//! Results, events and state of protocol 0.1 (MRTKLIB `docs/rpc/` §5-§9, `openrpc.json`).
//!
//! Names are the contract's names with mechanical case conversion. Members that the client
//! does not know are ignored (§4), so these types never reject a later minor version's
//! additions. Members that the contract omits when they have no value are `Option`.

use serde::Deserialize;
use serde::de::{self, Deserializer};
use serde_json::Value;

/// Result of `getVersion`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// `MAJOR.MINOR` of the protocol (§4).
    pub protocol_version: String,
    /// MRTKLIB version of the server, for display.
    pub mrtklib_version: String,
    pub authentication: Authentication,
}

/// How the server authenticates clients. `None` only for a server started with
/// `--rpc-no-auth` (testing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Authentication {
    Token,
    None,
}

/// Lifecycle state of the positioning engine (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
}

/// Why the engine is in or came to `stopping` / `stopped` (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// A stop, restart or shutdown request from any interface, or a signal.
    Request,
    /// All inputs were files and were read to the end.
    InputEnd,
    /// The start failed; see [`Lifecycle::last_error`].
    Error,
}

/// The most recent start failure (§5).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LastError {
    /// Stable name from an open set (0.1: `startFailed`, `outputExists`). The caller treats a name it does
    /// not know like `startFailed`, so this stays a string.
    pub reason: String,
    /// English text for display. Not part of the contract.
    pub message: String,
    /// Distinct refused output paths, in server order. Absent means empty.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// The lifecycle object: `status.lifecycle` and the data of status events (§5).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lifecycle {
    pub state: State,
    pub stop_reason: Option<StopReason>,
    pub last_error: Option<LastError>,
}

/// Status of the server (§6). Outside `running` it holds only the lifecycle.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Status {
    pub lifecycle: Lifecycle,
    /// Positioning mode in its TOML form (for example `kinematic`), while running.
    pub mode: Option<String>,
    /// The last published solution event, while running and after the first solution.
    pub solution: Option<Solution>,
}

/// Result of `getStatus` and `sync` (§6, §7): the status and the `seq` of the last event
/// that it reflects.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub status: Status,
}

/// Result of `start`, `stop`, `restart` and `shutdown` (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Accepted {
    /// The last event published when the request took effect. The outcome follows as
    /// events with a greater `seq`.
    pub seq: u64,
}

/// A time in GPST (§8).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct GpsTime {
    /// GPS week, not truncated to 10 bits.
    pub week: u32,
    /// Time of week (s).
    pub tow: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SolutionStatus {
    Fix,
    Float,
    Sbas,
    Dgps,
    Single,
    Ppp,
    /// Dead reckoning.
    Dr,
    /// No solution; the solution then holds only time, leap seconds and status.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Ecef {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Geodetic position: degrees, degrees, ellipsoidal height in metres (WGS84).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Llh {
    pub lat: f64,
    pub lon: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Position {
    pub ecef: Ecef,
    pub llh: Llh,
}

/// Position covariance in ECEF (m²).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct CovarianceEcef {
    pub xx: f64,
    pub yy: f64,
    pub zz: f64,
    pub xy: f64,
    pub yz: f64,
    pub zx: f64,
}

/// Position covariance in the local east-north-up frame at the solution position (m²).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct CovarianceEnu {
    pub ee: f64,
    pub nn: f64,
    pub uu: f64,
    pub en: f64,
    pub nu: f64,
    pub ue: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Covariance {
    pub ecef: CovarianceEcef,
    pub enu: CovarianceEnu,
}

/// Geoid model and the geoid height above the ellipsoid (§8).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Geoid {
    /// The configuration value `output.geoid_model`, kept as text: the GUI only shows it.
    pub model: String,
    /// Metres. Absent when the model gives no value.
    pub separation: Option<f64>,
}

/// A solution event or the `solution` of a status (§8). A solution with status `none` has
/// only `time`, `leap_seconds` and `status`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Solution {
    pub time: GpsTime,
    /// GPST - UTC (s) that the server used for this epoch.
    pub leap_seconds: i32,
    pub status: SolutionStatus,
    pub position: Option<Position>,
    pub covariance: Option<Covariance>,
    pub geoid: Option<Geoid>,
    pub valid_satellites: Option<u32>,
    /// Age of differential corrections (s), as the engine reports it; 0 when the mode does
    /// not compute it.
    pub age: Option<f64>,
    /// Ambiguity validation ratio; 0 when no ambiguity resolution was done.
    pub ratio: Option<f64>,
    /// Base station position, when the mode uses one and it is known.
    pub base: Option<Position>,
}

/// A log message event (§9).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LogMessage {
    /// The lowest selected log level at which the message is delivered (1 or 2).
    pub level: u8,
    /// English text for display; not part of the contract.
    pub text: String,
}

/// An event with its sequence number (§7).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub data: EventData,
}

// Solutions are almost all of the events, so boxing the largest variant would only add an
// allocation per event.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum EventData {
    Status(Lifecycle),
    Solution(Solution),
    LogMessage(LogMessage),
    /// A kind that this client does not know. The caller skips it (§4).
    Unknown {
        kind: String,
    },
}

/// Events are decoded in two steps, so that an unknown kind is kept as `Unknown` and does
/// not fail the whole page.
impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            seq: u64,
            kind: String,
            data: Value,
        }
        fn decode<T: de::DeserializeOwned, E: de::Error>(
            seq: u64,
            kind: &str,
            data: Value,
        ) -> Result<T, E> {
            serde_json::from_value(data)
                .map_err(|e| E::custom(format!("event {seq} ({kind}): {e}")))
        }
        let Raw { seq, kind, data } = Raw::deserialize(deserializer)?;
        let data = match kind.as_str() {
            "status" => EventData::Status(decode(seq, &kind, data)?),
            "solution" => EventData::Solution(decode(seq, &kind, data)?),
            "logMessage" => EventData::LogMessage(decode(seq, &kind, data)?),
            _ => EventData::Unknown { kind },
        };
        Ok(Event { seq, data })
    }
}

/// Result of `getEvents` (§7).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPage {
    pub events: Vec<Event>,
    /// Cursor for the next call: the `seq` of the last returned event, or `after`.
    pub next: u64,
    /// More events are ready now: call again without waiting.
    pub more: bool,
    /// Events after `after` are no longer held; `events` is empty. Call `sync` again.
    pub lost: bool,
    /// With `lost`: the oldest `seq` still held.
    pub oldest_seq: Option<u64>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(value: Value) -> Event {
        serde_json::from_value(value).expect("event decodes")
    }

    #[test]
    fn an_unknown_kind_is_kept_not_rejected() {
        let e = event(json!({ "seq": 7, "kind": "newThing", "data": { "x": [1, 2] } }));
        assert_eq!(
            e,
            Event {
                seq: 7,
                data: EventData::Unknown {
                    kind: "newThing".into()
                }
            }
        );
    }

    #[test]
    fn a_known_kind_with_bad_data_is_an_error() {
        let result: Result<Event, _> =
            serde_json::from_value(json!({ "seq": 1, "kind": "status", "data": { "state": "x" } }));
        let error = result.unwrap_err().to_string();
        assert!(error.contains("event 1 (status)"), "{error}");
    }

    #[test]
    fn status_and_log_events_decode() {
        let status = event(json!({
            "seq": 2, "kind": "status",
            "data": { "state": "stopped", "stopReason": "inputEnd" }
        }));
        let EventData::Status(lifecycle) = status.data else {
            panic!("not a status event");
        };
        assert_eq!(lifecycle.state, State::Stopped);
        assert_eq!(lifecycle.stop_reason, Some(StopReason::InputEnd));
        assert_eq!(lifecycle.last_error, None);

        let log =
            event(json!({ "seq": 3, "kind": "logMessage", "data": { "level": 2, "text": "t" } }));
        assert_eq!(
            log.data,
            EventData::LogMessage(LogMessage {
                level: 2,
                text: "t".into()
            })
        );
    }

    #[test]
    fn a_none_solution_has_only_time_leap_seconds_and_status() {
        let e = event(json!({
            "seq": 4, "kind": "solution",
            "data": {
                "time": { "week": 2440, "tow": 345639.0 },
                "leapSeconds": 18,
                "status": "none"
            }
        }));
        let EventData::Solution(solution) = e.data else {
            panic!("not a solution event");
        };
        assert_eq!(solution.status, SolutionStatus::None);
        assert_eq!(
            solution.time,
            GpsTime {
                week: 2440,
                tow: 345639.0
            }
        );
        assert_eq!(solution.leap_seconds, 18);
        assert!(solution.position.is_none() && solution.covariance.is_none());
        assert!(solution.geoid.is_none() && solution.base.is_none());
        assert!(solution.valid_satellites.is_none());
        assert!(solution.age.is_none() && solution.ratio.is_none());
    }

    #[test]
    fn unknown_members_are_ignored() {
        let page: EventPage = serde_json::from_value(json!({
            "events": [], "next": 5, "more": false, "lost": true, "oldestSeq": 9,
            "addedLater": 1
        }))
        .unwrap();
        assert_eq!(page.oldest_seq, Some(9));
        assert!(page.lost && page.events.is_empty());
    }
    #[test]
    fn last_error_paths_are_preserved_or_default_to_empty() {
        let error: LastError = serde_json::from_value(json!({"reason": "outputExists", "message": "exists", "paths": ["./a.pos", "b file.pos"]})).unwrap();
        assert_eq!(error.paths, ["./a.pos", "b file.pos"]);
        let error: LastError =
            serde_json::from_value(json!({"reason": "startFailed", "message": "failed"})).unwrap();
        assert!(error.paths.is_empty());
    }
}
