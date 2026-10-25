#![cfg(unix)]

use std::os::unix::net::UnixListener;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn proxy_exits_when_its_host_exits() {
    let home = tempfile::tempdir().unwrap();
    let _daemon_socket = UnixListener::bind(home.path().join("serve.sock")).unwrap();
    let script = format!(
        "'{}' serve </dev/null >/dev/null 2>&1 & echo $!; sleep 2",
        env!("CARGO_BIN_EXE_agent-brain")
    );
    let host = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("AGENT_BRAIN_HOME", home.path())
        .output()
        .unwrap();
    let pid: libc::pid_t = String::from_utf8_lossy(&host.stdout)
        .trim()
        .parse()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < deadline, "proxy {pid} outlived its host");
        std::thread::sleep(Duration::from_millis(100));
    }
}
