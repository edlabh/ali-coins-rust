//! Tracker e payloads de webhook (equivalente a `libs/webhooks.js` + cliente do `report.js`).
//!
//! - Sanitização: remove `sessionData`/`session`/`cookies`/`storageState`, mascara
//!   chaves de usuário, profundidade > 6 vira marcador e teto de 32 KB.
//! - Formatação por destino: Discord (`content`+`embeds`), Telegram (`text`) e
//!   genérico (payload sanitizado cru).
//! - Tracker de envios em voo com flush limitado por tempo (usado no shutdown).

use super::http::SafeHttpClient;
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

/// Teto de payload (bytes) enviado a webhooks externos.
pub const WEBHOOK_MAX_PAYLOAD_BYTES: usize = 32 * 1024;
/// Profundidade máxima antes de marcar como truncado.
pub const MAX_PAYLOAD_DEPTH: usize = 6;

/// Chaves que NUNCA devem sair para webhooks externos.
pub const WEBHOOK_FORBIDDEN_KEYS: [&str; 4] = ["sessiondata", "session", "cookies", "storagestate"];
/// Chaves de identificação de conta mascaradas antes do envio externo.
pub const WEBHOOK_USER_KEYS: [&str; 5] = ["user", "useremail", "email", "maskeduser", "account"];
/// Marcador de profundidade excedida (contrato do oráculo).
pub const MAX_DEPTH_MARKER: &str = "[profundidade máxima excedida]";

/// Remove dados sensíveis e mascara identificadores em contexto de usuário.
#[must_use]
pub fn sanitize_webhook_payload(value: &Value) -> Value {
    sanitize_value(value, 0, false)
}

fn sanitize_value(value: &Value, depth: usize, user_context: bool) -> Value {
    if depth > MAX_PAYLOAD_DEPTH {
        return Value::String(MAX_DEPTH_MARKER.to_string());
    }
    match value {
        Value::String(text) => {
            if user_context {
                Value::String(mask_user_value(&Value::String(text.clone())))
            } else {
                Value::String(text.clone())
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_value(item, depth + 1, user_context))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                let lower = key.to_lowercase();
                if WEBHOOK_FORBIDDEN_KEYS.contains(&lower.as_str()) {
                    continue;
                }
                let child_user_context =
                    user_context || WEBHOOK_USER_KEYS.contains(&lower.as_str());
                out.insert(
                    key.clone(),
                    sanitize_value(val, depth + 1, child_user_context),
                );
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn mask_user_value(value: &Value) -> String {
    let raw = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    crate::config::accounts::mask_user(&raw)
}

/// Formata o corpo conforme o destino (Discord/Telegram/genérico).
#[must_use]
pub fn format_webhook_body(url: &str, payload: &Value) -> Value {
    if url.contains("discord.com/api/webhooks") {
        let content = payload
            .get("text")
            .or_else(|| payload.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("Relatório ali-coins");
        let mut body = Map::new();
        body.insert(
            "content".to_string(),
            Value::String(escape_discord(content)),
        );
        return Value::Object(body);
    }
    if url.contains("api.telegram.org/bot") {
        let text = payload
            .get("text")
            .map_or_else(|| payload.to_string(), ToString::to_string);
        let mut body = Map::new();
        body.insert("text".to_string(), Value::String(text));
        return Value::Object(body);
    }
    payload.clone()
}

/// Neutraliza menções e markdown do Discord.
#[must_use]
pub fn escape_discord(text: &str) -> String {
    text.replace('@', "@\u{200b}")
        .replace("**", "\\*\\*")
        .replace('_', "\\_")
}

/// Serializa o payload respeitando o teto de 32 KB (corta e sinaliza).
#[must_use]
pub fn encode_webhook_payload(payload: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(payload).unwrap_or_default();
    if bytes.len() > WEBHOOK_MAX_PAYLOAD_BYTES {
        let truncated = serde_json::json!({
            "truncated": true,
            "text": "[payload acima de 32KB — truncado]"
        });
        bytes = serde_json::to_vec(&truncated).unwrap_or_default();
    }
    bytes
}

/// Envia um webhook (2xx = sucesso); erros nunca lançam.
pub async fn send_webhook(client: &SafeHttpClient, url: &str, payload: &Value) -> bool {
    let sanitized = sanitize_webhook_payload(payload);
    let body = format_webhook_body(url, &sanitized);
    let bytes = encode_webhook_payload(&body);
    let text = String::from_utf8_lossy(&bytes).to_string();
    match client.post_text(url, &text, "application/json").await {
        Ok(response) => (200..300).contains(&response.status),
        Err(_) => false,
    }
}

/// Tracker de webhooks em voo (flush com teto no encerramento).
#[derive(Debug, Clone, Default)]
pub struct WebhookTracker {
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl WebhookTracker {
    /// Cria um tracker vazio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra uma task em voo.
    pub fn track(&self, handle: JoinHandle<()>) {
        let mut handles = self.handles.lock().expect("tracker");
        handles.retain(|existing| !existing.is_finished());
        handles.push(handle);
    }

    /// Aguarda as tasks em voo com teto de tempo.
    pub async fn flush(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let pending: Vec<JoinHandle<()>> = {
                let mut handles = self.handles.lock().expect("tracker");
                std::mem::take(&mut *handles)
            };
            if pending.is_empty() {
                return;
            }
            for handle in pending {
                if tokio::time::Instant::now() >= deadline {
                    return;
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                let _ = tokio::time::timeout(remaining, handle).await;
            }
        }
    }

    /// Quantidade de envios pendentes.
    #[must_use]
    pub fn pending(&self) -> usize {
        let mut handles = self.handles.lock().expect("tracker");
        handles.retain(|handle| !handle.is_finished());
        handles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remove_sessao_e_mascara_usuario() {
        let payload = json!({
            "user": "fulano@example.com",
            "sessionData": { "cookies": ["segredo"] },
            "cookies": [1],
            "text": "ok"
        });
        let sanitized = sanitize_webhook_payload(&payload);
        assert!(sanitized.get("sessionData").is_none());
        assert!(sanitized.get("cookies").is_none());
        assert_eq!(sanitized["user"], "fu***@example.com");
        assert_eq!(sanitized["text"], "ok");
    }

    #[test]
    fn profundidade_limite() {
        let mut value = json!({"v": 1});
        for _ in 0..8 {
            value = json!({ "nested": value });
        }
        let sanitized = sanitize_webhook_payload(&value);
        assert!(
            sanitized
                .to_string()
                .contains("profundidade máxima excedida")
        );
    }

    #[test]
    fn formatos_por_destino() {
        let payload = json!({"text": "olá @everyone"});
        let discord = format_webhook_body("https://discord.com/api/webhooks/1/x", &payload);
        assert!(
            discord["content"]
                .as_str()
                .unwrap()
                .contains("@\u{200b}everyone")
        );
        let telegram = format_webhook_body("https://api.telegram.org/bot1/x", &payload);
        assert!(telegram.get("text").is_some());
        let generic = format_webhook_body("https://example.com/hook", &payload);
        assert_eq!(generic, payload);
    }

    #[test]
    fn teto_de_payload() {
        let huge = json!({"text": "x".repeat(WEBHOOK_MAX_PAYLOAD_BYTES + 100)});
        let bytes = encode_webhook_payload(&huge);
        assert!(bytes.len() < WEBHOOK_MAX_PAYLOAD_BYTES);
        assert!(String::from_utf8_lossy(&bytes).contains("truncated"));
    }
}
