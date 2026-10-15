//! Executable compatibility: a held stdin controls the same normal shutdown as RPC.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
            self.0.wait().unwrap();
        }
    }
}

fn wait(process: &mut Process) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(0));
            return;
        }
        assert!(Instant::now() < deadline, "mock did not shut down");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn stdin_eof_shuts_down_the_executable_without_rpc() {
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_mrtklib-egui-mock"))
            .args([
                "run",
                "--no-console",
                "--exit-on-stdin-eof",
                "--rpc-port",
                "0",
                "--rpc-no-auth",
                "--existing-output",
                "a.pos",
                "--existing-output",
                "b.pos",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stderr = process.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if line.unwrap().starts_with("MRTK-RPC-READY ") {
                sender.send(()).unwrap();
                return;
            }
        }
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("mock ready line");
    assert!(process.0.try_wait().unwrap().is_none());
    drop(process.0.stdin.take());
    wait(&mut process);
}
