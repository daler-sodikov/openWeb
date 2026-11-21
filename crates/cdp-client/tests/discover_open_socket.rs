//! Real Node answers `/json/list` as HTTP/1.0 and then keeps the socket open
//! (found in M5b-2): `discover` must not wait for EOF.

use std::time::Duration;

use ferrule_cdp_client::discover;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn discover_returns_without_waiting_for_the_peer_to_close()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let body = format!(r#"[ {{ "webSocketDebuggerUrl": "ws://127.0.0.1:{port}/abc" }} ]"#);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).await?;
        let head = format!(
            "HTTP/1.0 200 OK\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await?;
        // Keep the socket open until the test is done.
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok::<_, std::io::Error>(())
    });

    let url = tokio::time::timeout(
        Duration::from_secs(5),
        discover(port, Duration::from_secs(5), &CancellationToken::new()),
    )
    .await??;
    assert_eq!(url, format!("ws://127.0.0.1:{port}/abc"));
    server.abort();
    Ok(())
}
