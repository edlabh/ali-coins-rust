//! Testes de integração da camada HTTP segura + heartbeat com um servidor TCP local.

use ali_coins_core::notify::{HeartbeatAction, HeartbeatConfig, SafeHttpClient, send_heartbeat};
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::time::Duration;

/// Sobe um servidor HTTP mínimo e devolve a porta.
fn start_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut buffer = [0_u8; 8192];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let first_line = request.lines().next().unwrap_or_default().to_string();
            let mut parts = first_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("/").to_string();
            let (status, extra_headers, body) = route(&method, &path);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

fn route(method: &str, path: &str) -> (&'static str, String, String) {
    if path == "/ok" {
        ("200 OK", String::new(), "{\"ok\":true}".to_string())
    } else if path == "/redir" {
        ("302 Found", "Location: /ok\r\n".to_string(), String::new())
    } else if path == "/loop" {
        (
            "302 Found",
            "Location: /loop\r\n".to_string(),
            String::new(),
        )
    } else if path.starts_with("/no405") {
        if method == "POST" {
            ("405 Method Not Allowed", String::new(), String::new())
        } else {
            ("200 OK", String::new(), "ok-get".to_string())
        }
    } else {
        ("404 Not Found", String::new(), "nope".to_string())
    }
}

fn permissive_client() -> SafeHttpClient {
    SafeHttpClient::new(true, Duration::from_secs(5)).expect("cliente")
}

#[tokio::test]
async fn get_simples_e_post_json() {
    let port = start_server();
    let client = permissive_client();
    let response = client
        .get(&format!("http://127.0.0.1:{port}/ok"))
        .await
        .expect("get");
    assert_eq!(response.status, 200);
    assert!(response.body.contains("ok"));

    let response = client
        .post_json(
            &format!("http://127.0.0.1:{port}/ok"),
            &serde_json::json!({"a": 1}),
        )
        .await
        .expect("post");
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn segue_redirect_e_limita_loop() {
    let port = start_server();
    let client = permissive_client();
    let response = client
        .get(&format!("http://127.0.0.1:{port}/redir"))
        .await
        .expect("redirect");
    assert_eq!(response.status, 200);
    assert!(response.final_url.ends_with("/ok"));

    let error = client
        .get(&format!("http://127.0.0.1:{port}/loop"))
        .await
        .expect_err("loop deve falhar");
    assert!(error.to_string().contains("redirects"));
}

#[tokio::test]
async fn bloqueia_localhost_e_ip_privado_sem_optin() {
    let client = SafeHttpClient::new(false, Duration::from_secs(5)).expect("cliente");
    let error = client
        .get("http://localhost:9/hook")
        .await
        .expect_err("localhost bloqueado");
    assert!(error.is_ssrf_blocked(), "{error}");

    let error = client
        .get("http://127.0.0.1:9/hook")
        .await
        .expect_err("IP privado bloqueado");
    assert!(error.is_ssrf_blocked(), "{error}");

    let error = client
        .get("http://169.254.169.254/latest/meta-data")
        .await
        .expect_err("metadata bloqueado");
    assert!(error.is_ssrf_blocked(), "{error}");
}

#[tokio::test]
async fn heartbeat_start_com_fallback_405() {
    let port = start_server();
    let client = permissive_client();
    let config = HeartbeatConfig {
        enabled: true,
        url: format!("http://127.0.0.1:{port}/no405"),
        timeout_ms: 5000,
    };
    let result = send_heartbeat(&client, HeartbeatAction::Start, &config, "host", None).await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.status, Some(200));
}

#[tokio::test]
async fn heartbeat_desabilitado_e_skipped() {
    let client = permissive_client();
    let config = HeartbeatConfig {
        enabled: false,
        url: "https://hc-ping.com/uuid".to_string(),
        timeout_ms: 5000,
    };
    let result = send_heartbeat(&client, HeartbeatAction::Success, &config, "host", None).await;
    assert!(result.skipped);
}
