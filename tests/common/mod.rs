//! Shared test harness: an in-process mock System One server.
//!
//! The mock plays a tiny deterministic "model": it inspects the request's
//! state text and returns answers driven by simple patterns, so the full
//! pipeline — input resolution, question building, HTTP, rendering, exit
//! codes — is exercised with no network.
//!
//! Patterns recognized in the state:
//!
//! * `high-risk` — noul 0.93, score near the top level
//! * `coin-flip` — noul 0.5
//! * otherwise   — noul 0.07, score near the bottom
//! * choice answers pick the first option whose name appears in the state,
//!   falling back to the first option.

#![allow(dead_code)]

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use jev_sdk::TypeSafeClient;

/// Start the mock server; returns its base URL.
pub async fn mock_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Some((request_line, body)) = read_request(&mut socket).await else {
                    return;
                };
                let (status, response) = route(&request_line, &body);
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

/// Read one HTTP request; returns the request line and the body.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<(String, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut head_end = None;
    let mut content_length = 0usize;
    loop {
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if head_end.is_none() {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        head_end = Some(pos + 4);
                        let head = String::from_utf8_lossy(&buf[..pos]);
                        for line in head.lines() {
                            if let Some(v) =
                                line.to_ascii_lowercase().strip_prefix("content-length:")
                            {
                                content_length = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
                if let Some(end) = head_end {
                    if buf.len() >= end + content_length {
                        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                        let request_line = head.lines().next().unwrap_or("").to_owned();
                        let body =
                            String::from_utf8_lossy(&buf[end..end + content_length]).into_owned();
                        return Some((request_line, body));
                    }
                }
            }
        }
    }
}

/// Route the request; returns an HTTP status line and a JSON body.
fn route(request_line: &str, body: &str) -> (&'static str, String) {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    match (method, path) {
        ("POST", "/v1/systemone") => ("200 OK", systemone_response(body)),
        ("GET", "/v1/models") => (
            "200 OK",
            serde_json::json!({
                "models": [
                    {"name": "mock-jev", "description": "Mock model for tests",
                     "release_date": "2026-01-01T00:00:00Z"},
                    {"name": "mock-jev-small", "description": "Smaller mock",
                     "release_date": "2025-06-01T00:00:00Z"},
                ]
            })
            .to_string(),
        ),
        _ => ("404 Not Found", r#"{"detail":"not found"}"#.to_owned()),
    }
}

/// Fabricate a System One response: one type-matching answer per question,
/// probabilities driven by patterns in the state. The requested model is
/// echoed back like the real API echoes the resolved model version.
fn systemone_response(request_body: &str) -> String {
    let request: serde_json::Value =
        serde_json::from_str(request_body).expect("mock got non-JSON request");
    let state_text = request["state"].to_string();

    let high = state_text.contains("high-risk");
    let noul_p = if high {
        0.93
    } else if state_text.contains("coin-flip") {
        0.5
    } else {
        0.07
    };

    let mut answers = serde_json::Map::new();
    for (id, question) in request["questions"].as_object().unwrap() {
        let answer = match question["type"].as_str().unwrap() {
            "noul" => serde_json::json!({"type": "noul", "noul": noul_p}),
            "choice" => {
                let options = question["criteria"].as_object().unwrap();
                let picked = options
                    .keys()
                    .find(|name| state_text.contains(name.as_str()))
                    .cloned()
                    .or_else(|| options.keys().next().cloned())
                    .unwrap_or_default();
                let n = options.len() as f64;
                let mut probabilities = serde_json::Map::new();
                for option in options.keys() {
                    let p = if *option == picked {
                        0.9
                    } else {
                        0.1 / (n - 1.0).max(1.0)
                    };
                    probabilities.insert(option.clone(), serde_json::json!(p));
                }
                serde_json::json!({
                    "type": "choice",
                    "choice": picked,
                    "probabilities": probabilities,
                    "confidence": 0.88,
                })
            }
            "score" => {
                let levels = question["criteria"].as_array().unwrap();
                let mut legend = serde_json::Map::new();
                for (i, level) in levels.iter().enumerate() {
                    legend.insert(i.to_string(), level.clone());
                }
                let top = levels.len().saturating_sub(1) as f64;
                let score = if high {
                    top - 0.1
                } else if state_text.contains("coin-flip") {
                    top / 2.0
                } else {
                    0.1
                };
                serde_json::json!({
                    "type": "score",
                    "score": score,
                    "legend": legend,
                    "probabilities": {},
                    "confidence": 0.9,
                })
            }
            other => panic!("unexpected question type {other}"),
        };
        answers.insert(id.clone(), answer);
    }
    serde_json::json!({
        "model": request["model"],
        "answers": answers,
        "usage": {"input_tokens": 100, "output_tokens": 10},
    })
    .to_string()
}

/// A client pointed at the mock, with retries off (tests want determinism).
pub fn test_client(base_url: &str) -> TypeSafeClient {
    TypeSafeClient::builder()
        .api_key("test-key")
        .base_url(base_url)
        .retry(jev_sdk::RetryPolicy::none())
        .build()
        .unwrap()
}
