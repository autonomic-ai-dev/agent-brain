use std::io::Read;
use std::process::{Child, Command, ExitStatus, Output};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(super) fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<Output> {
    let mut child = cmd.spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = wait_with_deadline(&mut child, timeout)?;
    Ok(Output {
        status,
        stdout: join(stdout),
        stderr: join(stderr),
    })
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

fn join(handle: JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

fn wait_with_deadline(child: &mut Child, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out after {}s", timeout.as_secs());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::Stdio;
    use std::time::Instant;

    use super::*;

    fn shell(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    #[test]
    fn returns_stdout_of_a_command_that_finishes_in_time() {
        let output = output_with_timeout(&mut shell("echo done"), Duration::from_secs(10))
            .expect("command finishes");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"done\n");
    }

    #[test]
    fn kills_a_command_that_exceeds_the_timeout() {
        let started = Instant::now();
        let err = output_with_timeout(&mut shell("sleep 30"), Duration::from_millis(200))
            .expect_err("sleep outlives the timeout");
        assert!(err.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn large_output_is_drained_while_waiting() {
        let output = output_with_timeout(
            &mut shell("head -c 300000 /dev/zero"),
            Duration::from_secs(10),
        )
        .expect("output larger than a pipe buffer completes");
        assert_eq!(output.stdout.len(), 300_000);
    }
}
