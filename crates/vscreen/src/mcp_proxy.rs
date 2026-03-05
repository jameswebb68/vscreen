//! Lightweight MCP stdio-to-SSE proxy.
//!
//! Bridges stdin/stdout (for Cursor or other MCP clients) to an existing
//! vscreen SSE MCP server. Does not start any dev environment, pipelines,
//! or HTTP servers — just forwards JSON-RPC messages bidirectionally.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Run the stdio proxy, forwarding MCP messages between stdin/stdout and the SSE server.
///
/// Exits when stdin is closed (e.g., Cursor disconnects).
pub(crate) async fn run_stdio_proxy(server_url: &str) -> Result<(), String> {
    info!(url = server_url, "starting MCP stdio proxy");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))?;

    let session_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let url: Arc<str> = server_url.into();

    // Spawn a background task that listens for server-initiated notifications via GET SSE.
    // This is started after we get a session ID from the first POST.
    let sse_session_id = session_id.clone();
    let sse_stdout = stdout.clone();
    let sse_url = url.clone();
    let sse_client = client.clone();
    let sse_handle = tokio::spawn(async move {
        // Wait until we have a session ID before opening the GET stream
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let sid = sse_session_id.lock().await.clone();
            if sid.is_some() {
                if let Err(e) = run_sse_listener(&sse_client, &sse_url, &sid.unwrap(), &sse_stdout).await {
                    warn!(error = %e, "SSE listener ended");
                }
                break;
            }
        }
    });

    // Read JSON-RPC messages from stdin, POST to server, write responses to stdout.
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await
            .map_err(|e| format!("stdin read error: {e}"))?;
        if n == 0 {
            info!("stdin closed, proxy shutting down");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        debug!(len = trimmed.len(), "proxy: stdin -> server");

        let sid = session_id.lock().await.clone();
        let mut req = client
            .post(url.as_ref())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(ref sid) = sid {
            req = req.header("Mcp-Session-Id", sid.as_str());
        }

        let response = match req.body(trimmed.to_string()).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "proxy: POST failed");
                let err_response = format!(
                    r#"{{"jsonrpc":"2.0","error":{{"code":-32603,"message":"proxy POST failed: {}"}},"id":null}}"#,
                    e.to_string().replace('"', "'")
                );
                write_line(&stdout, &err_response).await;
                continue;
            }
        };

        let status = response.status();

        // Extract session ID from response headers
        if let Some(new_sid) = response.headers().get("mcp-session-id") {
            if let Ok(s) = new_sid.to_str() {
                let mut guard = session_id.lock().await;
                if guard.as_deref() != Some(s) {
                    info!(session_id = s, "proxy: got session ID");
                    *guard = Some(s.to_string());
                }
            }
        }

        if status == reqwest::StatusCode::ACCEPTED {
            debug!("proxy: 202 Accepted (notification)");
            continue;
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // SSE stream — read events and write each data payload to stdout
            handle_sse_response(response, &stdout).await;
        } else {
            // JSON response — write directly to stdout
            match response.text().await {
                Ok(body) => {
                    let body = body.trim();
                    if !body.is_empty() {
                        write_line(&stdout, body).await;
                    }
                }
                Err(e) => {
                    error!(error = %e, "proxy: failed to read response body");
                }
            }
        }
    }

    sse_handle.abort();
    info!("proxy shutdown complete");
    Ok(())
}

/// Read SSE events from a response and write each `data:` payload to stdout.
async fn handle_sse_response(
    response: reqwest::Response,
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
) {
    use futures_util::StreamExt;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "SSE stream error");
                break;
            }
        };

        let text = match std::str::from_utf8(&chunk) {
            Ok(t) => t,
            Err(_) => continue,
        };

        buffer.push_str(text);

        // Process complete SSE events (separated by double newlines)
        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            for line in event_block.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if !data.is_empty() {
                        write_line(stdout, data).await;
                    }
                }
            }
        }
    }

    // Process any remaining buffered data
    if !buffer.is_empty() {
        for line in buffer.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() {
                    write_line(stdout, data).await;
                }
            }
        }
    }
}

/// Listen for server-initiated SSE notifications via GET and forward to stdout.
async fn run_sse_listener(
    client: &reqwest::Client,
    url: &str,
    session_id: &str,
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
) -> Result<(), String> {
    debug!(session_id, "starting SSE notification listener");

    let response = client
        .get(url)
        .header("Accept", "text/event-stream")
        .header("Mcp-Session-Id", session_id)
        .send()
        .await
        .map_err(|e| format!("SSE GET failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            debug!("server does not support GET SSE stream");
            return Ok(());
        }
        return Err(format!("SSE GET returned {status}"));
    }

    handle_sse_response(response, stdout).await;
    debug!("SSE notification listener ended");
    Ok(())
}

/// Write a line to stdout (newline-delimited JSON-RPC).
async fn write_line(stdout: &Arc<Mutex<tokio::io::Stdout>>, data: &str) {
    let mut out = stdout.lock().await;
    let line = format!("{data}\n");
    if let Err(e) = out.write_all(line.as_bytes()).await {
        error!(error = %e, "failed to write to stdout");
    }
    if let Err(e) = out.flush().await {
        error!(error = %e, "failed to flush stdout");
    }
}
