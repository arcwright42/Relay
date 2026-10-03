use super::config::Config;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{
        self, Message, client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig,
    },
};

pub(super) type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Cancellation also covers DNS, TLS handshake, upload, receive and playback backpressure.
pub(super) fn execute<T>(
    cancelled: &AtomicBool,
    timeout: Duration,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Voice request cancelled.".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start voice network worker.")?;
    let result = runtime.block_on(async {
        tokio::select! {
            result = future => result,
            _ = tokio::time::sleep(timeout) => Err("Voice request timed out.".into()),
            _ = async { loop {
                if cancelled.load(Ordering::Acquire) { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }} => Err("Voice request cancelled.".into()),
        }
    });
    // OS name resolution can outlive the socket; never block the voice worker on it.
    runtime.shutdown_timeout(Duration::from_millis(100));
    result
}

pub(super) async fn connect(config: &Config) -> Result<Socket, String> {
    let mut request = config
        .endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| "Invalid voice endpoint.")?;
    let mut authorization = HeaderValue::from_str(&format!("Bearer {}", config.key))
        .map_err(|_| "Invalid voice API key.")?;
    authorization.set_sensitive(true);
    request.headers_mut().insert("Authorization", authorization);
    request
        .headers_mut()
        .insert("User-Agent", HeaderValue::from_static("Relay/0.1"));
    if let Some(workspace) = &config.workspace {
        request.headers_mut().insert(
            "X-DashScope-WorkSpace",
            HeaderValue::from_str(workspace).map_err(|_| "Invalid voice workspace.")?,
        );
    }
    let limits = WebSocketConfig::default()
        .max_message_size(Some(512 * 1024))
        .max_frame_size(Some(512 * 1024));
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_with_config(request, Some(limits), true),
    )
    .await
    .map_err(|_| "Voice connection timed out.")?
    .map_err(network_error)?;
    Ok(socket)
}

// Do not surface Debug/Display of network responses, URLs or provider error messages:
// upstream bodies may echo Authorization, user audio or submitted text.
pub(super) fn network_error(error: tungstenite::Error) -> String {
    match error {
        tungstenite::Error::Http(response) => match response.status().as_u16() {
            401 | 403 => {
                "Voice authentication failed (401/403). Check the key, region and workspace."
            }
            429 => "Voice service is rate limited. Try again shortly.",
            _ => "Voice WebSocket handshake was rejected.",
        },
        tungstenite::Error::Tls(_) => "Voice TLS connection failed.",
        tungstenite::Error::Capacity(_) => "Voice service response exceeded its size limit.",
        _ => "Voice connection failed or closed before completion.",
    }
    .into()
}

pub(super) fn header(action: &str, id: &str) -> Value {
    json!({"action":action,"task_id":id,"streaming":"duplex"})
}
pub(super) fn finish(id: &str) -> Message {
    message(json!({"header":header("finish-task", id),"payload":{"input":{}}}))
}
pub(super) fn message(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

pub(super) fn event(text: &str, id: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|_| "Invalid voice service event.")?;
    if value["header"]["task_id"].as_str() != Some(id) {
        return Err("Voice service returned an unrelated task.".into());
    }
    if value["header"]["event"] == "task-failed" {
        let reason = match value["header"]["error_code"].as_str().unwrap_or_default() {
            "InvalidApiKey" | "AccessDenied" | "Unauthorized" | "Forbidden" => {
                "Voice authentication failed. Check the key, region and workspace."
            }
            "InvalidParameter" | "InvalidParameter.Model" => {
                "Voice model or audio parameters were rejected. Check model access in the selected region."
            }
            "Throttling" | "Throttling.RateQuota" => {
                "Voice service is rate limited. Try again shortly."
            }
            "QuotaExhausted" | "Arrearage" => "Voice account quota is exhausted.",
            _ => "Voice service could not complete the task. Check model access and account quota.",
        };
        return Err(reason.into());
    }
    Ok(value)
}

pub(super) async fn start(config: &Config, id: &str, payload: Value) -> Result<Socket, String> {
    let mut socket = connect(config).await?;
    socket
        .send(message(
            json!({"header":header("run-task",id),"payload":payload}),
        ))
        .await
        .map_err(network_error)?;
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(incoming) = socket.next().await {
            match incoming.map_err(network_error)? {
                Message::Text(text) if event(&text, id)?["header"]["event"] == "task-started" => {
                    return Ok::<(), String>(());
                }
                Message::Ping(bytes) => socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(network_error)?,
                Message::Pong(_) => {}
                _ => return Err("Voice service did not start the task.".into()),
            }
        }
        Err("Voice service closed before starting the task.".into())
    })
    .await
    .map_err(|_| "Voice task start timed out.")??;
    Ok(socket)
}
