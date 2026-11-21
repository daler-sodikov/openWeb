//! A real `node --inspect-brk` session. Skipped when `node` is not installed.

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ferrule_cdp_client::protocol::{
    DebuggerEnable, Event, Resume, RunIfWaitingForDebugger, RuntimeEnable,
};
use ferrule_cdp_client::{CdpClient, CdpEvent, CdpEvents, discover};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Kills the child when the test ends, however it ends.
struct Node(Child);

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Binds port 0, reads the port and releases it (the race the design accepts).
fn free_port() -> Result<u16, std::io::Error> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

async fn next(events: &mut CdpEvents) -> Result<CdpEvent, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(15), events.recv())
        .await?
        .ok_or_else(|| "event stream ended".into())
}

#[tokio::test]
async fn debug_a_two_line_script_on_real_node() -> TestResult {
    if !node_available() {
        eprintln!("skipped: node is not installed");
        return Ok(());
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cdp-real-node");
    std::fs::create_dir_all(&dir)?;
    let script = dir.join("two-lines.js");
    std::fs::write(
        &script,
        "const answer = 40 + 2;\nglobalThis.done = answer;\n",
    )?;

    let port = free_port()?;
    let child = Command::new("node")
        .arg(format!("--inspect-brk=127.0.0.1:{port}"))
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut node = Node(child);

    let cancel = CancellationToken::new();
    let url = discover(port, Duration::from_secs(15), &cancel).await?;
    let (client, mut events) = CdpClient::connect(&url, cancel).await?;

    client.send(RuntimeEnable).await?;
    let enabled = client.send(DebuggerEnable::default()).await?;
    assert!(!enabled.debugger_id.is_empty());
    client.send(RunIfWaitingForDebugger).await?;

    // Node stops on the first line of the script.
    let paused = loop {
        match next(&mut events).await?.typed()? {
            Some(Event::Paused(paused)) => break paused,
            _ => continue,
        }
    };
    assert_eq!(paused.reason, "Break on start");
    assert!(!paused.call_frames.is_empty());

    client.send(Resume::default()).await?;
    // The script runs to its end; Node then waits for the debugger to
    // disconnect, so the connection is closed from this side.
    client.close().await;
    loop {
        if let CdpEvent::Closed { .. } = next(&mut events).await? {
            break;
        }
    }
    assert!(events.recv().await.is_none());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if node.0.try_wait()?.is_some() {
            break;
        }
        if Instant::now() > deadline {
            return Err("node did not exit after the debugger disconnected".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}
