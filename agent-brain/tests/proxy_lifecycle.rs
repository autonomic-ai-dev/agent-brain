#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
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

#[test]
fn proxy_answers_a_pending_request_when_the_daemon_drops_it() {
    let home = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(home.path().join("serve.sock")).unwrap();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        for (index, incoming) in listener.incoming().enumerate() {
            let Ok(stream) = incoming else { break };
            if index == 0 {
                let mut line = String::new();
                let _ = BufReader::new(&stream).read_line(&mut line);
                drop(stream);
            } else {
                kept.push(stream);
            }
        }
    });

    let mut proxy = Command::new(env!("CARGO_BIN_EXE_agent-brain"))
        .arg("serve")
        .env("AGENT_BRAIN_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = proxy.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"test","version":"1"}}}}}}"#
    )
    .unwrap();

    let (tx, rx) = mpsc::channel();
    let stdout = proxy.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let reply = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("proxy should answer the request the daemon dropped");
    let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(value["id"], 1);
    assert_eq!(value["error"]["code"], -32000);
    let _ = proxy.kill();
    let _ = proxy.wait();
}
