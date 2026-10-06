//! Testes de integração do Telegram e webhooks com servidor TCP local.

use ali_coins_core::notify::{
    SafeHttpClient, TelegramConfig, TelegramEvent, build_message, send_telegram, send_webhook,
};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Counters {
    by_path: Mutex<HashMap<String, usize>>,
    total_started: AtomicUsize,
}

impl Counters {
    fn hit(&self, path: &str) -> usize {
        let mut map = self.by_path.lock().expect("counters");
        let entry = map.entry(path.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }
}

fn start_server() -> (u16, Arc<Counters>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let counters = Arc::new(Counters::default());
    let shared = Arc::clone(&counters);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let counters = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut buffer = [0_u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let first_line = request.lines().next().unwrap_or_default().to_string();
                let mut parts = first_line.split_whitespace();
                let _method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("/").to_string();
                counters.total_started.fetch_add(1, Ordering::SeqCst);
                let hit = counters.hit(&path);
                let (status, body) = route(&path, hit);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
    (port, counters)
}

fn route(path: &str, hit: usize) -> (&'static str, String) {
    if path.contains("flaky") {
        if hit == 1 {
            ("500 Internal Server Error", "{\"ok\":false}".to_string())
        } else {
            ("200 OK", "{\"ok\":true}".to_string())
        }
    } else if path.contains("bad") {
        (
            "400 Bad Request",
            "{\"ok\":false,\"description\":\"can't parse entities\"}".to_string(),
        )
    } else if path.contains("throttled") {
        if hit == 1 {
            (
                "429 Too Many Requests",
                "{\"ok\":false,\"parameters\":{\"retry_after\":0.001}}".to_string(),
            )
        } else {
            ("200 OK", "{\"ok\":true}".to_string())
        }
    } else if path.contains("slow") {
        std::thread::sleep(Duration::from_millis(300));
        ("200 OK", "{\"ok\":true}".to_string())
    } else {
        ("200 OK", "{\"ok\":true}".to_string())
    }
}

fn telegram_config(port: u16, token: &str) -> TelegramConfig {
    TelegramConfig {
        enabled: true,
        bot_token: token.to_string(),
        chat_id: "123".to_string(),
        silent: false,
        timeout_ms: 5000,
        api_base: format!("http://127.0.0.1:{port}"),
    }
}

#[tokio::test]
async fn retenta_5xx_e_entrega() {
    let (port, counters) = start_server();
    let client = SafeHttpClient::new(true, Duration::from_secs(5)).expect("cliente");
    let config = telegram_config(port, "flaky");
    let result = send_telegram(&client, &config, "<b>oi</b>").await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.status, Some(200));
    assert_eq!(
        counters
            .by_path
            .lock()
            .unwrap()
            .get("/botflaky/sendMessage"),
        Some(&2)
    );
}

#[tokio::test]
async fn respeita_retry_after_em_429() {
    let (port, counters) = start_server();
    let client = SafeHttpClient::new(true, Duration::from_secs(5)).expect("cliente");
    let config = telegram_config(port, "throttled");
    let result = send_telegram(&client, &config, "oi").await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.status, Some(200));
    assert_eq!(
        counters
            .by_path
            .lock()
            .unwrap()
            .get("/botthrottled/sendMessage"),
        Some(&2),
        "deve reenviar após 429"
    );
}

#[tokio::test]
async fn fallback_de_400_sem_parse_mode() {
    let (port, counters) = start_server();
    let client = SafeHttpClient::new(true, Duration::from_secs(5)).expect("cliente");
    let config = telegram_config(port, "bad");
    let result = send_telegram(&client, &config, "<b>oi</b>").await;
    assert!(!result.ok);
    assert_eq!(result.status, Some(400));
    // Primeira tentativa + fallback sem parse_mode.
    assert_eq!(
        counters.by_path.lock().unwrap().get("/botbad/sendMessage"),
        Some(&2)
    );
}

#[tokio::test]
async fn timeout_ambiguo_nao_retenta() {
    let (port, counters) = start_server();
    let client = SafeHttpClient::new(true, Duration::from_millis(100)).expect("cliente");
    let config = telegram_config(port, "slow");
    let result = send_telegram(&client, &config, "oi").await;
    assert!(!result.ok);
    assert!(result.error.as_deref().unwrap_or("").contains("timeout"));
    assert_eq!(
        counters.by_path.lock().unwrap().get("/botslow/sendMessage"),
        Some(&1),
        "timeout ambíguo não deve retentar"
    );
}

#[tokio::test]
async fn desabilitado_nao_faz_requisicao() {
    let (port, counters) = start_server();
    let client = SafeHttpClient::new(true, Duration::from_secs(5)).expect("cliente");
    let mut config = telegram_config(port, "ok");
    config.enabled = false;
    let result = send_telegram(&client, &config, "oi").await;
    assert!(result.skipped);
    assert_eq!(counters.total_started.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn webhook_generico_ok_e_falha() {
    let (port, _counters) = start_server();
    let client = SafeHttpClient::new(true, Duration::from_secs(5)).expect("cliente");
    let payload = json!({ "text": "relatório", "user": "fulano@example.com" });
    assert!(send_webhook(&client, &format!("http://127.0.0.1:{port}/ok"), &payload).await);
    assert!(!send_webhook(&client, &format!("http://127.0.0.1:{port}/bad"), &payload).await);
}

#[tokio::test]
async fn mensagem_e_montada_antes_do_envio() {
    let ctx = ali_coins_core::notify::TelegramContext {
        user: Some("fulano@example.com"),
        ..Default::default()
    };
    let message = build_message(TelegramEvent::DryRun, &ctx);
    assert!(message.contains("Teste Dry-Run"));
}
