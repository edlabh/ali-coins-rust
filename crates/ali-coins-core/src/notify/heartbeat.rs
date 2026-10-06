//! Heartbeat / dead man's switch compatível com `libs/heartbeat.js`.
//!
//! - `start` → `POST {base}/start` com `"AliExpress Coins job started"`.
//! - `success` → `POST {base}` com o relatório (texto; JSON grande vira resumo).
//! - `fail` → `POST {base}/fail` com `"{label}: {mensagem}"` (≤ 2000 chars).
//! - Fallback para **GET** quando o destino responde 405.
//! - Falhas nunca lançam: devolvem `HeartbeatResult { ok: false, .. }`.

use super::http::SafeHttpClient;
use url::Url;

/// Teto do corpo de sucesso (32 KB, igual ao oráculo).
pub const MAX_SUCCESS_BODY_BYTES: usize = 32 * 1024;
/// Teto do corpo de falha (2000 chars).
pub const MAX_FAIL_BODY_CHARS: usize = 2000;

/// Ação do heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatAction {
    /// Início do job.
    Start,
    /// Sucesso (POST na base).
    Success,
    /// Falha (`/fail`).
    Fail,
}

/// Configuração efetiva do heartbeat.
#[derive(Debug, Clone, Default)]
pub struct HeartbeatConfig {
    /// Habilitado.
    pub enabled: bool,
    /// URL base (aceita sufixos `/start`/`/fail`).
    pub url: String,
    /// Timeout por tentativa.
    pub timeout_ms: u64,
}

/// Resultado do envio (nunca lança).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResult {
    /// Sucesso (2xx).
    pub ok: bool,
    /// Desabilitado/sem URL.
    pub skipped: bool,
    /// Status HTTP final, quando houve resposta.
    pub status: Option<u16>,
    /// Erro, quando houve.
    pub error: Option<String>,
}

impl HeartbeatResult {
    fn skipped() -> Self {
        Self {
            ok: false,
            skipped: true,
            status: None,
            error: None,
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            skipped: false,
            status: None,
            error: Some(error.into()),
        }
    }
}

/// Normaliza a URL base removendo barras finais e sufixos de ação.
#[must_use]
pub fn normalize_base_url(raw: &str) -> String {
    let mut base = raw.trim().trim_end_matches('/').to_string();
    for suffix in ["/start", "/fail"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_string();
            break;
        }
    }
    base
}

/// Monta a URL da ação preservando a query string.
#[must_use]
pub fn build_action_url(base: &str, action: HeartbeatAction) -> String {
    let normalized = normalize_base_url(base);
    let path = match action {
        HeartbeatAction::Success => return normalized,
        HeartbeatAction::Start => "/start",
        HeartbeatAction::Fail => "/fail",
    };
    match normalized.find('?') {
        Some(index) => format!("{}{}{}", &normalized[..index], path, &normalized[index..]),
        None => format!("{normalized}{path}"),
    }
}

/// Mascara a URL do heartbeat para logs (credenciais, hash e segmentos longos).
#[must_use]
pub fn mask_heartbeat_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return "***".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);

    let path = url.path().to_string();
    let masked_path: Vec<String> = path.split('/').map(mask_segment).collect();
    url.set_path(&masked_path.join("/"));

    if let Some(query) = url.query() {
        let masked_query: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, _)) => format!("{key}=***"),
                None => "***".to_string(),
            })
            .collect();
        url.set_query(Some(&masked_query.join("&")));
    }
    url.to_string()
}

fn mask_segment(segment: &str) -> String {
    if segment.is_empty() || segment == "start" || segment == "fail" {
        return segment.to_string();
    }
    let chars: Vec<char> = segment.chars().collect();
    let count = chars.len();
    if count > 8 {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[count - 4..].iter().collect();
        format!("{head}***{tail}")
    } else if count > 4 {
        let head: String = chars[..2].iter().collect();
        let tail: String = chars[count - 2..].iter().collect();
        format!("{head}***{tail}")
    } else {
        "***".to_string()
    }
}

/// Corpo da ação (`None` para sucesso sem payload).
#[must_use]
pub fn build_action_body(
    action: HeartbeatAction,
    host_label: &str,
    payload: Option<&str>,
) -> String {
    match action {
        HeartbeatAction::Start => "AliExpress Coins job started".to_string(),
        HeartbeatAction::Success => {
            let body = payload.unwrap_or("").to_string();
            if body.len() > MAX_SUCCESS_BODY_BYTES {
                let summary: String = body.chars().take(MAX_FAIL_BODY_CHARS).collect();
                format!("{summary}… [resumo: payload acima de 32KB]")
            } else {
                body
            }
        }
        HeartbeatAction::Fail => {
            let message = payload.unwrap_or("");
            let body = format!("{host_label}: {message}");
            body.chars().take(MAX_FAIL_BODY_CHARS).collect()
        }
    }
}

/// Envia a ação do heartbeat (nunca lança).
pub async fn send_heartbeat(
    client: &SafeHttpClient,
    action: HeartbeatAction,
    config: &HeartbeatConfig,
    host_label: &str,
    payload: Option<&str>,
) -> HeartbeatResult {
    if !config.enabled || config.url.trim().is_empty() {
        return HeartbeatResult::skipped();
    }
    let url = build_action_url(&config.url, action);
    let body = build_action_body(action, host_label, payload);

    match client.post_text(&url, &body, "text/plain").await {
        Ok(response) if (200..300).contains(&response.status) => HeartbeatResult {
            ok: true,
            skipped: false,
            status: Some(response.status),
            error: None,
        },
        Ok(response) if response.status == 405 => {
            // Fallback GET exigido pelo oráculo.
            match client.get(&url).await {
                Ok(fallback) if (200..300).contains(&fallback.status) => HeartbeatResult {
                    ok: true,
                    skipped: false,
                    status: Some(fallback.status),
                    error: None,
                },
                Ok(fallback) => HeartbeatResult {
                    ok: false,
                    skipped: false,
                    status: Some(fallback.status),
                    error: Some(format!("GET fallback retornou {}", fallback.status)),
                },
                Err(err) => HeartbeatResult::failure(err.to_string()),
            }
        }
        Ok(response) => HeartbeatResult {
            ok: false,
            skipped: false,
            status: Some(response.status),
            error: Some(format!("status {}", response.status)),
        },
        Err(err) => HeartbeatResult::failure(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_base_e_acoes() {
        assert_eq!(
            normalize_base_url("https://hc-ping.com/uuid/"),
            "https://hc-ping.com/uuid"
        );
        assert_eq!(
            normalize_base_url("https://hc-ping.com/uuid/start"),
            "https://hc-ping.com/uuid"
        );
        assert_eq!(
            normalize_base_url("https://hc-ping.com/uuid/fail?x=1"),
            "https://hc-ping.com/uuid/fail?x=1"
        );
        assert_eq!(
            build_action_url("https://hc-ping.com/uuid", HeartbeatAction::Start),
            "https://hc-ping.com/uuid/start"
        );
        assert_eq!(
            build_action_url("https://hc-ping.com/uuid", HeartbeatAction::Success),
            "https://hc-ping.com/uuid"
        );
        assert_eq!(
            build_action_url("https://hc-ping.com/uuid?k=v", HeartbeatAction::Fail),
            "https://hc-ping.com/uuid/fail?k=v"
        );
    }

    #[test]
    fn mascara_url() {
        let masked = mask_heartbeat_url("https://hc-ping.com/12345678-abcd-uuid-secreto?token=xyz");
        assert!(!masked.contains("secreto"));
        assert!(!masked.contains("xyz"));
        assert!(masked.contains("1234***"));
        assert!(masked.contains("token=***"));
    }

    #[test]
    fn corpos_das_acoes() {
        assert_eq!(
            build_action_body(HeartbeatAction::Start, "host", None),
            "AliExpress Coins job started"
        );
        assert_eq!(
            build_action_body(HeartbeatAction::Fail, "host", Some("erro")),
            "host: erro"
        );
        let long = "x".repeat(MAX_FAIL_BODY_CHARS + 50);
        assert_eq!(
            build_action_body(HeartbeatAction::Fail, "host", Some(&long))
                .chars()
                .count(),
            MAX_FAIL_BODY_CHARS
        );
    }

    #[test]
    fn mascara_url_e_corpo_de_acao() {
        assert_eq!(mask_heartbeat_url("não é url"), "***");
        let mascarada = mask_heartbeat_url(
            "https://user:senha@heartbeat.example.com/api/abcdefghijkl/start?token=segredo&x=1#frag",
        );
        assert!(mascarada.contains("abcd***ijkl"), "{mascarada}");
        assert!(!mascarada.contains("senha"));
        assert!(!mascarada.contains("segredo"));
        assert!(!mascarada.contains('#'));
        let longo = build_action_body(HeartbeatAction::Success, "host", Some(&"x".repeat(40_000)));
        assert!(longo.contains("[resumo: payload acima de 32KB]"));
        assert_eq!(
            build_action_body(HeartbeatAction::Success, "host", Some("ok")),
            "ok"
        );
        assert_eq!(
            build_action_body(HeartbeatAction::Start, "host", None),
            "AliExpress Coins job started"
        );
        let falha = build_action_body(HeartbeatAction::Fail, "host", Some("erro"));
        assert!(falha.contains("erro"), "{falha}");
    }

    #[test]
    fn resultado_de_falha_e_skipped() {
        let resultado = HeartbeatResult::failure("deu ruim");
        assert!(!resultado.ok);
        assert!(!resultado.skipped);
        assert_eq!(resultado.status, None);
        assert_eq!(resultado.error.as_deref(), Some("deu ruim"));
        assert!(HeartbeatResult::skipped().skipped);
    }
}
