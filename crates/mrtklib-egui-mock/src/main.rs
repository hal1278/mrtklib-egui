//! Command-line mock of `mrtk run` with only the RPC endpoint, for manual GUI development.
//!
//! It takes the token from `MRTK_RPC_TOKEN` and prints the ready line on stderr like the
//! real server. Unlike the real server it does not generate a token and does not remove
//! the variable from its environment (it starts no child processes).

use std::process::ExitCode;
use std::time::Duration;

use mrtklib_egui_mock::Config;

const USAGE: &str = "\
usage: mrtklib-egui-mock [run] [options]
  --no-console            accepted for mrtk run compatibility (no effect)
  --exit-on-stdin-eof      shut down normally when stdin closes (needs --no-console)
  --existing-output PATH   simulate an existing output file (repeatable)
  --config FILE           accepted for mrtk run compatibility (ignored)
  --rpc-port PORT          port to listen on (default 0: chosen by the system)
  --rpc-bind ADDR          address to listen on (default 127.0.0.1)
  --rpc-origin ORIGIN      accept browser connections from ORIGIN (repeatable)
  --rpc-no-auth            accept clients without a token
  --interval-ms N          interval between solutions (default 1000)
  --transition-ms N        duration of start and stop (default 300)
  --fail-start             every start fails
  --input-end-after N      stop with input end after N solutions of a run
The token is read from MRTK_RPC_TOKEN unless --rpc-no-auth is given.";

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)) {
        Ok(arguments) => run(arguments),
        Err(message) => {
            eprintln!("mrtklib-egui-mock: {message}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug)]
struct Arguments {
    config: Config,
    exit_on_stdin_eof: bool,
}

fn run(arguments: Arguments) -> ExitCode {
    match mrtklib_egui_mock::start(arguments.config) {
        Ok(running) => {
            if arguments.exit_on_stdin_eof {
                running.exit_on_stdin_eof();
            }
            eprintln!(
                "MRTK-RPC-READY {}",
                serde_json::json!({ "url": running.url() })
            );
            running.wait();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("mrtklib-egui-mock: cannot listen: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut config = Config::default();
    let mut no_auth = false;
    let mut no_console = false;
    let mut exit_on_stdin_eof = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        if first && arg == "run" {
            first = false;
            continue;
        }
        first = false;
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--no-console" => no_console = true,
            "--exit-on-stdin-eof" => exit_on_stdin_eof = true,
            "--existing-output" => config.existing_outputs.push(value(&arg)?),
            "--config" => {
                let _ = value(&arg)?;
            }
            "--rpc-port" => config.port = number(&value(&arg)?)?,
            "--rpc-bind" => {
                config.bind = value(&arg)?
                    .parse()
                    .map_err(|_| "invalid address".to_string())?
            }
            "--rpc-origin" => config.allowed_origins.push(value(&arg)?),
            "--rpc-no-auth" => no_auth = true,
            "--interval-ms" => {
                config.solution_interval = Duration::from_millis(number(&value(&arg)?)?)
            }
            "--transition-ms" => config.transition = Duration::from_millis(number(&value(&arg)?)?),
            "--fail-start" => config.fail_start = true,
            "--input-end-after" => config.input_end_after = Some(number(&value(&arg)?)?),
            "-h" | "--help" => return Err("help".to_string()),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if exit_on_stdin_eof && !no_console {
        return Err("--exit-on-stdin-eof requires --no-console".into());
    }
    if no_auth {
        if !config.bind.is_loopback() {
            return Err("--rpc-no-auth needs a loopback address".to_string());
        }
    } else {
        let token =
            std::env::var("MRTK_RPC_TOKEN").map_err(|_| "MRTK_RPC_TOKEN is not set".to_string())?;
        if token.is_empty()
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("MRTK_RPC_TOKEN may contain only base64url characters".to_string());
        }
        config.token = Some(token);
    }
    Ok(Arguments {
        config,
        exit_on_stdin_eof,
    })
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(args: &[&str]) -> impl Iterator<Item = String> {
        args.iter().map(|arg| (*arg).to_string())
    }

    #[test]
    fn accepts_real_run_arguments_and_ignores_configuration() {
        let config = parse(arguments(&[
            "run",
            "--rpc-port",
            "0",
            "--no-console",
            "--config",
            "a nonexistent config.toml",
            "--rpc-no-auth",
        ]))
        .unwrap()
        .config;
        assert_eq!(config.port, 0);
        assert!(config.token.is_none());
        assert_eq!(
            config.solution_interval,
            Config::default().solution_interval
        );
        assert!(parse(arguments(&["--no-console", "--rpc-no-auth"])).is_ok());
    }

    #[test]
    fn run_is_only_allowed_once_at_the_beginning() {
        for args in [&["run", "run"][..], &["--no-console", "run"][..]] {
            assert_eq!(parse(arguments(args)).unwrap_err(), "unknown option run");
        }
    }

    #[test]
    fn config_requires_a_file_argument() {
        assert_eq!(
            parse(arguments(&["run", "--config"])).unwrap_err(),
            "--config needs a value"
        );
    }
    #[test]
    fn stdin_eof_requires_no_console_and_outputs_are_repeatable() {
        assert_eq!(
            parse(arguments(&["--exit-on-stdin-eof", "--rpc-no-auth"])).unwrap_err(),
            "--exit-on-stdin-eof requires --no-console"
        );
        let arguments = parse(arguments(&[
            "--no-console",
            "--exit-on-stdin-eof",
            "--rpc-no-auth",
            "--existing-output",
            "b.pos",
            "--existing-output",
            "a.pos",
        ]))
        .unwrap();
        assert!(arguments.exit_on_stdin_eof);
        assert_eq!(arguments.config.existing_outputs, ["b.pos", "a.pos"]);
    }
}
