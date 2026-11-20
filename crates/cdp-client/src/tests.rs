//! Transport tests against the scripted fake server. No real Node needed.

use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::testing::{FakeCdpServer, Reply};
use crate::{CdpClient, CdpError, CdpEvent, CdpEvents, discover};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Echoes `params` back as the result.
fn echo(request: &Value) -> Vec<Reply> {
    vec![Reply::Result(
        request.get("params").cloned().unwrap_or(Value::Null),
    )]
}

async fn connect(server: &FakeCdpServer) -> Result<(CdpClient, CdpEvents), CdpError> {
    CdpClient::connect(&server.url(), CancellationToken::new()).await
}

async fn next(events: &mut CdpEvents) -> Result<CdpEvent, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await?
        .ok_or_else(|| "event stream ended".into())
}

#[tokio::test]
async fn discover_then_connect_then_call_then_close() -> TestResult {
    let server = FakeCdpServer::start(echo).await?;
    let cancel = CancellationToken::new();
    let url = discover(server.port(), Duration::from_secs(5), &cancel).await?;
    assert_eq!(url, server.url());

    let (client, mut events) = CdpClient::connect(&url, cancel).await?;
    let answer: Value = client.call("Debugger.enable", json!({ "x": 1 })).await?;
    assert_eq!(answer, json!({ "x": 1 }));

    client.close().await;
    client.close().await; // idempotent
    let CdpEvent::Closed { .. } = next(&mut events).await? else {
        return Err("expected Closed".into());
    };
    assert!(events.recv().await.is_none());
    let late: Result<Value, _> = client.call("Debugger.enable", ()).await;
    assert_eq!(late, Err(CdpError::Closed));
    Ok(())
}

#[tokio::test]
async fn unit_params_are_omitted_from_the_request() -> TestResult {
    let server = FakeCdpServer::start(|_| vec![Reply::Result(json!({}))]).await?;
    let (client, _events) = connect(&server).await?;
    let _: Value = client.call("Runtime.enable", ()).await?;
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "Runtime.enable");
    assert!(requests[0].get("params").is_none());
    Ok(())
}

#[tokio::test]
async fn answers_are_matched_by_id_even_out_of_order() -> TestResult {
    // The first request is answered late, the second at once.
    let server = FakeCdpServer::start(|request| {
        let params = request["params"].clone();
        if params["slow"] == true {
            vec![Reply::ResultAfter(Duration::from_millis(150), params)]
        } else {
            vec![Reply::Result(params)]
        }
    })
    .await?;
    let (client, _events) = connect(&server).await?;
    let slow = client.clone();
    let slow_call =
        tokio::spawn(async move { slow.call::<_, Value>("A", json!({ "slow": true })).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let fast: Value = client.call("B", json!({ "slow": false })).await?;
    assert_eq!(fast, json!({ "slow": false }));
    assert_eq!(slow_call.await??, json!({ "slow": true }));

    let ids: Vec<u64> = server
        .requests()
        .iter()
        .filter_map(|r| r["id"].as_u64())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    Ok(())
}

#[tokio::test]
async fn many_concurrent_calls_each_get_their_own_answer() -> TestResult {
    let server = FakeCdpServer::start(echo).await?;
    let (client, _events) = connect(&server).await?;
    let mut tasks = Vec::new();
    for n in 0..50 {
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            client.call::<_, Value>("Echo", json!({ "n": n })).await
        }));
    }
    for (n, task) in tasks.into_iter().enumerate() {
        assert_eq!(task.await??, json!({ "n": n }));
    }
    Ok(())
}

#[tokio::test]
async fn a_protocol_error_is_returned_with_code_and_message() -> TestResult {
    let server = FakeCdpServer::start(|_| {
        vec![Reply::Error {
            code: -32601,
            message: "'Nope.nothing' wasn't found".into(),
        }]
    })
    .await?;
    let (client, _events) = connect(&server).await?;
    let result: Result<Value, _> = client.call("Nope.nothing", ()).await;
    assert_eq!(
        result,
        Err(CdpError::Protocol {
            code: -32601,
            message: "'Nope.nothing' wasn't found".into()
        })
    );
    // The connection survives a protocol error.
    let again: Result<Value, _> = client.call("Nope.nothing", ()).await;
    assert!(matches!(again, Err(CdpError::Protocol { .. })));
    Ok(())
}

#[tokio::test]
async fn a_result_of_the_wrong_shape_is_a_decode_error() -> TestResult {
    let server = FakeCdpServer::start(|_| vec![Reply::Result(json!("text"))]).await?;
    let (client, _events) = connect(&server).await?;
    let result: Result<Vec<u8>, _> = client.call("X", ()).await;
    assert!(matches!(result, Err(CdpError::Decode(_))));
    Ok(())
}

#[tokio::test]
async fn a_call_without_an_answer_times_out_and_the_client_keeps_working() -> TestResult {
    let server = FakeCdpServer::start(|request| {
        if request["method"] == "Silent" {
            vec![Reply::Ignore]
        } else {
            echo(request)
        }
    })
    .await?;
    let (client, _events) = connect(&server).await?;
    let timed_out = client
        .call_raw("Silent", Value::Null, Duration::from_millis(80))
        .await;
    assert_eq!(timed_out, Err(CdpError::Timeout));
    let ok: Value = client.call("Echo", json!(7)).await?;
    assert_eq!(ok, json!(7));
    Ok(())
}

#[tokio::test]
async fn cancelling_fails_pending_calls_and_ends_the_connection() -> TestResult {
    let server = FakeCdpServer::start(|_| vec![Reply::Ignore]).await?;
    let cancel = CancellationToken::new();
    let (client, mut events) = CdpClient::connect(&server.url(), cancel.clone()).await?;
    let pending = {
        let client = client.clone();
        tokio::spawn(async move { client.call::<_, Value>("Hang", ()).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    assert_eq!(pending.await?, Err(CdpError::Cancelled));
    assert!(matches!(next(&mut events).await?, CdpEvent::Closed { .. }));
    let after: Result<Value, _> = client.call("Hang", ()).await;
    assert_eq!(after, Err(CdpError::Cancelled));
    Ok(())
}

#[tokio::test]
async fn cancelling_while_connecting_returns_cancelled() -> TestResult {
    // A listener that never completes the WebSocket handshake.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        CdpClient::connect(&format!("ws://127.0.0.1:{port}/x"), cancel),
    )
    .await?;
    assert!(matches!(result, Err(CdpError::Cancelled)));
    drop(listener);
    Ok(())
}

#[tokio::test]
async fn a_closing_peer_fails_pending_calls_and_ends_with_closed() -> TestResult {
    let server = FakeCdpServer::start(|request| {
        if request["method"] == "Die" {
            vec![Reply::Close]
        } else {
            echo(request)
        }
    })
    .await?;
    let (client, mut events) = connect(&server).await?;
    let result: Result<Value, _> = client.call("Die", ()).await;
    assert_eq!(result, Err(CdpError::Closed));
    assert!(matches!(next(&mut events).await?, CdpEvent::Closed { .. }));
    assert!(events.recv().await.is_none());
    Ok(())
}

#[tokio::test]
async fn events_arrive_in_order_before_closed() -> TestResult {
    let server = FakeCdpServer::start(|request| {
        if request["method"] == "Runtime.runIfWaitingForDebugger" {
            vec![
                Reply::Event {
                    method: "Debugger.paused".into(),
                    params: json!({ "reason": "Break" }),
                },
                Reply::Result(json!({})),
            ]
        } else {
            echo(request)
        }
    })
    .await?;
    let (client, mut events) = connect(&server).await?;
    let _: Value = client.call("Runtime.runIfWaitingForDebugger", ()).await?;
    server.send_event("Debugger.resumed", Value::Null);
    server.close_peer();
    assert_eq!(
        next(&mut events).await?,
        CdpEvent::Event {
            method: "Debugger.paused".into(),
            params: json!({ "reason": "Break" })
        }
    );
    assert_eq!(
        next(&mut events).await?,
        CdpEvent::Event {
            method: "Debugger.resumed".into(),
            params: Value::Null
        }
    );
    assert!(matches!(next(&mut events).await?, CdpEvent::Closed { .. }));
    Ok(())
}

#[tokio::test]
async fn an_event_flood_is_delivered_completely_and_in_order() -> TestResult {
    const COUNT: u32 = 5_000; // far more than the event buffer holds
    let server = FakeCdpServer::start(|request| {
        if request["method"] == "Flood" {
            vec![
                Reply::Flood {
                    method: "Runtime.consoleAPICalled".into(),
                    count: COUNT,
                },
                Reply::Result(json!({ "done": true })),
            ]
        } else {
            echo(request)
        }
    })
    .await?;
    let (client, mut events) = connect(&server).await?;
    // The consumer is slow to start: the task must be throttled, not grow.
    let call = {
        let client = client.clone();
        tokio::spawn(async move { client.call::<_, Value>("Flood", ()).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    for expected in 0..COUNT {
        match next(&mut events).await? {
            CdpEvent::Event { params, .. } => assert_eq!(params["n"], expected),
            other => return Err(format!("unexpected {other:?}").into()),
        }
    }
    assert_eq!(call.await??, json!({ "done": true }));
    // Still responsive after the flood.
    let ok: Value = client.call("Echo", json!(1)).await?;
    assert_eq!(ok, json!(1));
    Ok(())
}

#[tokio::test]
async fn malformed_and_unknown_messages_are_ignored() -> TestResult {
    let server = FakeCdpServer::start(|request| {
        vec![
            Reply::Raw("this is not json".into()),
            Reply::Raw(json!({ "id": 9999, "result": {} }).to_string()),
            Reply::Raw(json!({ "neither": "id nor method" }).to_string()),
            Reply::Result(request["params"].clone()),
        ]
    })
    .await?;
    let (client, _events) = connect(&server).await?;
    let ok: Value = client.call("X", json!("fine")).await?;
    assert_eq!(ok, json!("fine"));
    Ok(())
}

#[tokio::test]
async fn non_loopback_and_malformed_urls_are_refused_without_connecting() {
    for url in [
        "ws://192.168.1.5:9229/abc",
        "ws://localhost:9229/abc",
        "ws://[::1]:9229/abc",
        "ws://example.com:9229/abc",
        "ws://127.0.0.1@evil.example:9229/abc",
        "ws://127.0.0.1.evil.example:9229/abc",
        "ws://127.0.0.1/abc",
        "ws://127.0.0.1:0/abc",
        "wss://127.0.0.1:9229/abc",
        "http://127.0.0.1:9229/abc",
        "127.0.0.1:9229",
        "",
    ] {
        let result = CdpClient::connect(url, CancellationToken::new()).await;
        assert!(
            matches!(result, Err(CdpError::Refused(_))),
            "{url} should be refused, got {:?}",
            result.map(|_| ())
        );
    }
}

#[tokio::test]
async fn discover_retries_until_the_inspector_listens() -> TestResult {
    // Reserve a port, release it, and start the server only later.
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = reserved.local_addr()?.port();
    drop(reserved);
    let cancel = CancellationToken::new();
    let finding = {
        let cancel = cancel.clone();
        tokio::spawn(async move { discover(port, Duration::from_secs(10), &cancel).await })
    };
    tokio::time::sleep(Duration::from_millis(250)).await;
    let server = FakeCdpServer::start_on(port, echo).await?;
    assert_eq!(finding.await??, server.url());
    Ok(())
}

#[tokio::test]
async fn discover_times_out_and_can_be_cancelled() -> TestResult {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = reserved.local_addr()?.port();
    drop(reserved); // nothing listens here now
    let started = tokio::time::Instant::now();
    let timed_out = discover(port, Duration::from_millis(200), &CancellationToken::new()).await;
    assert_eq!(timed_out, Err(CdpError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));

    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let cancelled = discover(port, Duration::from_secs(30), &cancel).await;
    assert_eq!(cancelled, Err(CdpError::Cancelled));
    Ok(())
}
