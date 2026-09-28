//! Modelo de metadados e validações de sessão (equivalente às partes puras do oráculo).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Metadados da sessão (`session_meta*.json`), com campos extras preservados.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionMeta {
    /// Conta vinculada.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Data de gravação (ISO).
    #[serde(default, rename = "savedAt", skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
    /// Se o arquivo está cifrado.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
    /// Sessão importada de outro host.
    #[serde(
        default,
        rename = "isImported",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_imported: Option<bool>,
    /// Data de importação.
    #[serde(
        default,
        rename = "importedAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub imported_at: Option<String>,
    /// Data de exportação (origem remota).
    #[serde(
        default,
        rename = "exportedAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub exported_at: Option<String>,
    /// Expiração estimada.
    #[serde(default, rename = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Host de origem da exportação.
    #[serde(
        default,
        rename = "exportedFrom",
        skip_serializing_if = "Option::is_none"
    )]
    pub exported_from: Option<String>,
    /// Último streak persistido.
    #[serde(
        default,
        rename = "lastStreakDays",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_streak_days: Option<i64>,
    /// Data do último check-in.
    #[serde(
        default,
        rename = "lastCheckinDate",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_checkin_date: Option<String>,
    /// Data do último desafio anti-bot (cooldown).
    #[serde(
        default,
        rename = "lastCaptchaAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_captcha_at: Option<String>,
    /// Data da última rotação de chave.
    #[serde(
        default,
        rename = "lastRotatedAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_rotated_at: Option<String>,
    /// Data da última migração de formato.
    #[serde(
        default,
        rename = "migratedAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub migrated_at: Option<String>,
    /// Campos desconhecidos, preservados como no `.passthrough()` do Zod.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Resultado de validação de sessão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    /// Sessão válida?
    pub valid: bool,
    /// Motivo da invalidação (mensagem PT-BR do oráculo).
    pub reason: Option<String>,
}

impl Validation {
    fn ok() -> Self {
        Self {
            valid: true,
            reason: None,
        }
    }

    fn invalid(reason: impl Into<String>) -> Self {
        Self {
            valid: false,
            reason: Some(reason.into()),
        }
    }
}

/// Indica se a sessão veio de importação remota.
#[must_use]
pub fn is_imported_session(meta: Option<&SessionMeta>) -> bool {
    meta.is_some_and(|meta| meta.is_imported == Some(true) || meta.imported_at.is_some())
}

/// Cookie expirado? (`expires` em segundos Unix, > 0).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn is_cookie_expired(cookie: &Value) -> bool {
    cookie
        .get("expires")
        .and_then(Value::as_f64)
        .is_some_and(|expires| expires > 0.0 && expires * 1000.0 <= now_ms() as f64)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn normalize_user(value: &str) -> String {
    value.trim().to_lowercase()
}

fn is_auth_cookie(cookie: &Value) -> bool {
    let name = cookie.get("name").and_then(Value::as_str);
    let value = cookie.get("value").and_then(Value::as_str);
    matches!(name, Some("xman_us_t" | "login_aliyunid_ticket"))
        && value.is_some_and(|v| !v.is_empty())
}

/// O storage state contém ao menos um cookie de autenticação com valor?
#[must_use]
pub fn has_auth_cookies(payload: &Value) -> bool {
    payload
        .get("cookies")
        .and_then(Value::as_array)
        .is_some_and(|cookies| cookies.iter().any(is_auth_cookie))
}

/// Valida a sessão contra a conta esperada e a validade dos cookies.
#[must_use]
pub fn validate_session(
    payload: &Value,
    meta: Option<&SessionMeta>,
    expected_user: Option<&str>,
) -> Validation {
    let Some(cookies) = payload.get("cookies").and_then(Value::as_array) else {
        return Validation::invalid("Dados da sessão ausentes ou sem lista de cookies.");
    };

    if let Some(expected) = expected_user {
        let Some(meta_user) = meta.and_then(|meta| meta.user.as_deref()) else {
            return Validation::invalid(
                "Metadados de sessão ausentes ou sem identificação da conta vinculada.",
            );
        };
        if normalize_user(meta_user) != normalize_user(expected) {
            return Validation::invalid(format!(
                "Conta da sessão ativa (\"{meta_user}\") não corresponde à conta configurada (\"{expected}\")."
            ));
        }
    }

    let auth_cookies: Vec<&Value> = cookies
        .iter()
        .filter(|cookie| is_auth_cookie(cookie))
        .collect();
    if auth_cookies.is_empty() {
        return Validation::invalid(
            "Nenhum cookie de autenticação válido (xman_us_t / login_aliyunid_ticket) encontrado.",
        );
    }
    if auth_cookies.iter().all(|cookie| is_cookie_expired(cookie)) {
        return Validation::invalid(
            "Todos os cookies de autenticação do AliExpress estão expirados.",
        );
    }

    Validation::ok()
}

/// Valida o payload de importação (`{ session: { cookies, ... }, meta?: {...} }`).
///
/// Mensagens principais idênticas ao oráculo; erros de tipo usam texto aproximado
/// (classificado como flexível em `docs/05-divergencias-conhecidas.md`).
pub fn validate_session_payload(raw: &Value) -> Result<(), String> {
    let mut issues: Vec<String> = Vec::new();

    let session = raw.get("session");
    match session {
        Some(Value::Object(session)) => match session.get("cookies") {
            Some(Value::Array(cookies)) => {
                if cookies.is_empty() {
                    issues.push(
                        "session.cookies: A sessão deve conter ao menos um cookie.".to_string(),
                    );
                }
                for (index, cookie) in cookies.iter().enumerate() {
                    match cookie {
                        Value::Object(cookie) => {
                            if cookie.get("name").and_then(Value::as_str).is_none() {
                                issues.push(format!(
                                    "session.cookies.{index}.name: Invalid input: expected string, received undefined"
                                ));
                            }
                            if cookie.get("value").and_then(Value::as_str).is_none() {
                                issues.push(format!(
                                    "session.cookies.{index}.value: Invalid input: expected string, received undefined"
                                ));
                            }
                        }
                        _ => issues.push(format!(
                            "session.cookies.{index}: Invalid input: expected object, received {}",
                            json_type(cookie)
                        )),
                    }
                }
            }
            Some(other) => issues.push(format!(
                "session.cookies: Invalid input: expected array, received {}",
                json_type(other)
            )),
            None => issues.push(
                "session.cookies: Invalid input: expected array, received undefined".to_string(),
            ),
        },
        Some(other) => issues.push(format!(
            "session: Invalid input: expected object, received {}",
            json_type(other)
        )),
        None => {
            issues.push("session: Invalid input: expected object, received undefined".to_string());
        }
    }

    if let Some(meta) = raw.get("meta") {
        match meta {
            Value::Object(meta) => {
                if let Some(user) = meta.get("user") {
                    if user.as_str().is_none_or(str::is_empty) {
                        issues.push(
                            "meta.user: O usuário no metadado da sessão não pode ser vazio."
                                .to_string(),
                        );
                    }
                }
            }
            Value::Null => {}
            other => issues.push(format!(
                "meta: Invalid input: expected object, received {}",
                json_type(other)
            )),
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Estrutura de sessão inválida: {}",
            issues.join(", ")
        ))
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Cooldown pós-captcha ativo? (janela em horas; 0 desliga)
#[must_use]
pub fn is_captcha_cooldown_active(last_captcha_at: Option<&str>, hours: u64, now_ms: i64) -> bool {
    if hours == 0 {
        return false;
    }
    let Some(raw) = last_captcha_at else {
        return false;
    };
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return false;
    };
    let hours_ms = i64::try_from(hours).unwrap_or(i64::MAX) * 60 * 60 * 1000;
    now_ms - then.timestamp_millis() < hours_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta_with_user(user: &str) -> SessionMeta {
        SessionMeta {
            user: Some(user.to_string()),
            ..SessionMeta::default()
        }
    }

    #[test]
    fn valida_sessao_feliz() {
        let payload = json!({ "cookies": [{ "name": "xman_us_t", "value": "abc" }] });
        let result = validate_session(&payload, Some(&meta_with_user("u@e.com")), Some("u@e.com"));
        assert!(result.valid);
    }

    #[test]
    fn rejeita_conta_divergente_e_cookies_expirados() {
        let payload = json!({ "cookies": [{ "name": "xman_us_t", "value": "abc" }] });
        let result = validate_session(
            &payload,
            Some(&meta_with_user("outra@e.com")),
            Some("u@e.com"),
        );
        assert!(!result.valid);
        assert!(result.reason.unwrap().contains("não corresponde"));

        let expired = json!({ "cookies": [{ "name": "xman_us_t", "value": "abc", "expires": 1 }] });
        let result = validate_session(&expired, None, None);
        assert!(!result.valid);
        assert!(result.reason.unwrap().contains("expirados"));
    }

    #[test]
    fn cookie_sem_auth_e_invalido() {
        let payload = json!({ "cookies": [{ "name": "outro", "value": "x" }] });
        let result = validate_session(&payload, None, None);
        assert!(!result.valid);
        assert!(
            result
                .reason
                .unwrap()
                .contains("Nenhum cookie de autenticação")
        );
    }

    #[test]
    fn payload_de_importacao() {
        let raw = json!({
            "session": { "cookies": [{ "name": "xman_us_t", "value": "abc" }] },
            "meta": { "user": "u@e.com", "extra": 1 }
        });
        assert!(validate_session_payload(&raw).is_ok());

        let empty = json!({ "session": { "cookies": [] } });
        let err = validate_session_payload(&empty).unwrap_err();
        assert!(err.contains("A sessão deve conter ao menos um cookie."));
    }

    #[test]
    fn cooldown_de_captcha() {
        let now = chrono::Utc::now().timestamp_millis();
        let recent = chrono::DateTime::from_timestamp_millis(now - 3_600_000)
            .unwrap()
            .to_rfc3339();
        assert!(is_captcha_cooldown_active(Some(&recent), 12, now));
        assert!(!is_captcha_cooldown_active(Some(&recent), 0, now));
        assert!(!is_captcha_cooldown_active(None, 12, now));
        assert!(!is_captcha_cooldown_active(Some("invalido"), 12, now));
    }
}
