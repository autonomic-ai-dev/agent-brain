use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

const CALLS_PER_CLIENT: usize = 20;

fn clients() -> usize {
    std::env::var("LOAD_CLIENTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "load benchmark: run with --release --ignored"]
async fn hundred_clients_route_through_the_daemon_socket() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = if std::env::var_os("AGENT_BRAIN_HOME").is_some() {
        agent_brain::config::Config::load().unwrap()
    } else {
        agent_brain::config::Config::isolated(dir.path().to_path_buf())
    };
    let engine = Arc::new(agent_brain::engine::Engine::new(config).unwrap());
    let socket = dir.path().join("brain.sock");
    let cwd = std::env::var("LOAD_CWD").unwrap_or_else(|_| "/tmp".into());

    let daemon_socket = socket.clone();
    tokio::spawn(async move {
        agent_brain::mcp::run_daemon(engine, &daemon_socket, None)
            .await
            .unwrap();
    });
    while !socket.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let started = Instant::now();
    let tasks: Vec<_> = (0..clients())
        .map(|client| {
            let socket = socket.clone();
            let cwd = cwd.clone();
            tokio::spawn(async move { session(&socket, client, &cwd).await })
        })
        .collect();
    let mut latencies = Vec::new();
    for task in tasks {
        latencies.extend(task.await.unwrap());
    }
    let wall = started.elapsed();

    latencies.sort();
    let at = |q: f64| latencies[((latencies.len() - 1) as f64 * q) as usize];
    eprintln!(
        "LOAD clients={} calls={} wall_ms={} rps={:.0} p50_ms={} p95_ms={} p99_ms={}",
        clients(),
        latencies.len(),
        wall.as_millis(),
        latencies.len() as f64 / wall.as_secs_f64(),
        at(0.50).as_millis(),
        at(0.95).as_millis(),
        at(0.99).as_millis(),
    );
    assert_eq!(latencies.len(), clients() * CALLS_PER_CLIENT);
}

async fn session(socket: &Path, client: usize, cwd: &str) -> Vec<Duration> {
    let stream = UnixStream::connect(socket).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    send(
        &mut write,
        json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "load", "version": "1"}
            }
        }),
    )
    .await;
    await_reply(&mut lines, 0).await;
    send(
        &mut write,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;

    let mut latencies = Vec::with_capacity(CALLS_PER_CLIENT);
    for call in 1..=CALLS_PER_CLIENT {
        let message = format!("run unit tests for client {client} call {call}");
        let start = Instant::now();
        send(
            &mut write,
            json!({
                "jsonrpc": "2.0", "id": call, "method": "tools/call",
                "params": {
                    "name": "route_task",
                    "arguments": {
                        "user_message": message,
                        "current_working_directory": cwd,
                        "open_files": []
                    }
                }
            }),
        )
        .await;
        let reply = await_reply(&mut lines, call).await;
        assert!(reply.get("result").is_some(), "route_task failed: {reply}");
        latencies.push(start.elapsed());
    }
    latencies
}

async fn send(write: &mut OwnedWriteHalf, message: Value) {
    let mut line = message.to_string();
    line.push('\n');
    write.write_all(line.as_bytes()).await.unwrap();
}

async fn await_reply(lines: &mut Lines<BufReader<OwnedReadHalf>>, id: usize) -> Value {
    loop {
        let line = lines.next_line().await.unwrap().expect("daemon closed the session");
        let value: Value = serde_json::from_str(&line).unwrap();
        if value.get("id") == Some(&json!(id)) {
            return value;
        }
    }
}
