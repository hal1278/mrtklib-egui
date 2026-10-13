//! Command-line mock of `mrtk run` with only the RPC endpoint, for manual GUI development.
//!
//! It takes the token from `MRTK_RPC_TOKEN` and prints the ready line on stderr like the
//! real server. Unlike the real server it does not generate a token and does not remove
//! the variable from its environment (it starts no child processes).

use std::process::ExitCode;
use std::time::Duration;

use mrtklib_egui_mock::Config;

const USAGE: &str = "\
usage: mrtklib-egui-mock [options]
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
        Ok(config) => run(config),
        Err(message) => {
            eprintln!("mrtklib-egui-mock: {message}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(config: Config) -> ExitCode {
    match mrtklib_egui_mock::start(config) {
        Ok(running) => {
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

fn parse(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
    let mut config = Config::default();
    let mut no_auth = false;
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
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
    Ok(config)
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("not a number: {text}"))
}
