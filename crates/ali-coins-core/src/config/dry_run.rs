//! Resumo do `--dry-run --json`, com os mesmos campos/ordem do oráculo.

use crate::config::{Account, Config};
use serde::Serialize;

/// Payload completo do dry-run JSON.
///
/// O contrato JSON do oráculo é composto majoritariamente por booleanos.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunSummary {
    /// Sempre `true` neste caminho.
    pub dry_run: bool,
    /// Sempre `true` (config inválida nem chega aqui).
    pub valid: bool,
    /// Usuário mascarado.
    pub user: String,
    /// `ALI_PASSWORD` configurada.
    pub password_configured: bool,
    /// `SESSION_SECRET` configurado.
    pub session_secret_configured: bool,
    /// `SESSION_SECRET_OLD` configurado.
    pub session_secret_old_configured: bool,
    /// `ENCRYPT_LOCAL_SESSION`.
    pub encrypt_local_session: bool,
    /// `ALLOW_MEDIA`.
    pub allow_media: bool,
    /// `HEADLESS`.
    pub headless: bool,
    /// `LOG_LEVEL`.
    pub log_level: &'static str,
    /// `CAPTCHA_COOLDOWN_HOURS`.
    pub captcha_cooldown_hours: u64,
    /// `NO_SANDBOX`.
    pub no_sandbox: bool,
    /// `NAV_TIMEOUT`.
    pub nav_timeout: u64,
    /// `TASK_MAX_ACTIONS`.
    pub task_max_actions: u64,
    /// `TASK_MAX_ATTEMPTS`.
    pub task_max_attempts: u64,
    /// `TASK_ROUND_MAX_ATTEMPTS`.
    pub task_round_max_attempts: u64,
    /// `TASK_MAX_DURATION_MS`.
    pub task_max_duration_ms: u64,
    /// `TASK_SCROLL_MAX_MS`.
    pub task_scroll_max_ms: u64,
    /// `TASK_PAUSE_MIN_MS`.
    pub task_pause_min_ms: u64,
    /// `TASK_PAUSE_MAX_MS`.
    pub task_pause_max_ms: u64,
    /// `ACCOUNT_DELAY_MIN_MS`.
    pub account_delay_min_ms: u64,
    /// `ACCOUNT_DELAY_MAX_MS`.
    pub account_delay_max_ms: u64,
    /// `START_DELAY_MIN_MS`.
    pub start_delay_min_ms: u64,
    /// `START_DELAY_MAX_MS`.
    pub start_delay_max_ms: u64,
    /// Bloco do Telegram.
    pub telegram: TelegramSummary,
    /// Bloco do heartbeat.
    pub heartbeat: HeartbeatSummary,
    /// Bloco do webhook.
    pub webhook: WebhookSummary,
    /// Contas mascaradas.
    pub accounts: Vec<AccountSummary>,
}

/// Bloco `telegram` do resumo.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramSummary {
    /// `TELEGRAM_ENABLED`.
    pub enabled: bool,
    /// Token presente.
    pub bot_token_configured: bool,
    /// Chat ID presente.
    pub chat_id_configured: bool,
    /// `TELEGRAM_SILENT`.
    pub silent: bool,
    /// Se um teste real foi disparado (`--notify`).
    pub test_sent: bool,
    /// Resultado do teste, quando disparado (omitido caso contrário, como no Node).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_success: Option<bool>,
}

/// Bloco `heartbeat` do resumo.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatSummary {
    /// `HEARTBEAT_ENABLED`.
    pub enabled: bool,
    /// URL presente.
    pub url_configured: bool,
    /// `HEARTBEAT_TIMEOUT_MS`.
    pub timeout_ms: u64,
}

/// Bloco `webhook` do resumo.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookSummary {
    /// URL presente.
    pub url_configured: bool,
    /// `ALLOW_PRIVATE_WEBHOOKS`.
    pub allow_private_webhooks: bool,
}

/// Entrada de `accounts`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    /// Índice 1-based.
    pub index: usize,
    /// Usuário mascarado.
    pub user: String,
    /// Senha resolvida.
    pub password_configured: bool,
}

impl DryRunSummary {
    /// Monta o resumo a partir da config validada e das contas.
    #[must_use]
    pub fn build(config: &Config, accounts: &[Account]) -> Self {
        Self {
            dry_run: true,
            valid: true,
            user: config.ali_user_masked(),
            password_configured: !config.ali_password.is_empty(),
            session_secret_configured: config.session_secret.is_some(),
            session_secret_old_configured: config.session_secret_old.is_some(),
            encrypt_local_session: config.encrypt_local_session,
            allow_media: config.allow_media,
            headless: config.headless,
            log_level: config.log_level.as_str(),
            captcha_cooldown_hours: config.captcha_cooldown_hours,
            no_sandbox: config.no_sandbox,
            nav_timeout: config.nav_timeout,
            task_max_actions: config.task_max_actions,
            task_max_attempts: config.task_max_attempts,
            task_round_max_attempts: config.task_round_max_attempts,
            task_max_duration_ms: config.task_max_duration_ms,
            task_scroll_max_ms: config.task_scroll_max_ms,
            task_pause_min_ms: config.task_pause_min_ms,
            task_pause_max_ms: config.task_pause_max_ms,
            account_delay_min_ms: config.account_delay_min_ms,
            account_delay_max_ms: config.account_delay_max_ms,
            start_delay_min_ms: config.start_delay_min_ms,
            start_delay_max_ms: config.start_delay_max_ms,
            telegram: TelegramSummary {
                enabled: config.telegram_enabled,
                bot_token_configured: !config.telegram_bot_token.is_empty(),
                chat_id_configured: !config.telegram_chat_id.is_empty(),
                silent: config.telegram_silent,
                test_sent: false,
                test_success: None,
            },
            heartbeat: HeartbeatSummary {
                enabled: config.heartbeat_enabled,
                url_configured: !config.heartbeat_url.is_empty(),
                timeout_ms: config.heartbeat_timeout_ms,
            },
            webhook: WebhookSummary {
                url_configured: !config.notify_webhook_url.is_empty(),
                allow_private_webhooks: config.allow_private_webhooks,
            },
            accounts: accounts
                .iter()
                .map(|account| AccountSummary {
                    index: account.index,
                    user: account.masked_user.clone(),
                    password_configured: !account.password.is_empty(),
                })
                .collect(),
        }
    }

    /// Serializa em JSON indentado (mesmo formato do `JSON.stringify(..., null, 2)`).
    #[must_use]
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

impl Config {
    /// Usuário primário mascarado.
    #[must_use]
    pub fn ali_user_masked(&self) -> String {
        crate::config::mask_user(&self.ali_user)
    }
}
