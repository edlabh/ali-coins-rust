//! Configuração compatível com o schema Zod do oráculo (`config.js`).
//!
//! Cobre a tabela de variáveis, as três famílias de coerção booleana, os
//! defaults silenciosos dos inteiros, as mensagens PT-BR e as regras do
//! `superRefine` (pausas, 24 h, Telegram, heartbeat/webhook e `SESSION_SECRET`).

pub mod accounts;
pub mod dry_run;
pub mod env;

pub use accounts::{Account, load_accounts, mask_chat_id, mask_user};
pub use dry_run::DryRunSummary;
pub use env::EnvSource;

use env::{
    bool_encrypt_local, bool_headless, bool_skip_app_only, bool_true_exact, non_negative_int,
    positive_int,
};
use std::path::{Path, PathBuf};
use url::{Host, Url};

/// Teto de 24 h para as pausas configuráveis.
pub const MAX_DELAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Problema de validação no formato do Zod (`path` + `message`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// Caminho do campo (nome da variável).
    pub path: String,
    /// Mensagem PT-BR (ou do Zod, para enums).
    pub message: String,
}

/// Erro de configuração (espelha `ConfigValidationError`).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Falha de validação com a lista de issues.
    #[error("Falha na validação das variáveis de configuração em credentials.env.")]
    Validation {
        /// Issues coletadas na ordem do schema.
        issues: Vec<ValidationIssue>,
    },
}

impl ConfigError {
    /// Issues associadas ao erro.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        match self {
            Self::Validation { issues } => issues,
        }
    }
}

/// Nível de log aceito por `LOG_LEVEL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// `trace`
    Trace,
    /// `debug`
    Debug,
    /// `info`
    Info,
    /// `warn`
    Warn,
    /// `error`
    Error,
    /// `fatal`
    Fatal,
    /// `silent`
    Silent,
}

impl LogLevel {
    /// Valores válidos, na ordem do schema.
    pub const VALUES: [&'static str; 7] =
        ["trace", "debug", "info", "warn", "error", "fatal", "silent"];

    fn parse(value: &str) -> Option<Self> {
        match value {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            "fatal" => Some(Self::Fatal),
            "silent" => Some(Self::Silent),
            _ => None,
        }
    }

    /// Representação textual.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Fatal => "fatal",
            Self::Silent => "silent",
        }
    }
}

/// Configuração validada.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Config {
    /// `ALI_USER` (aparado).
    pub ali_user: String,
    /// `ALI_PASSWORD`.
    pub ali_password: String,
    /// `SESSION_SECRET` (>= 32) ou ausente.
    pub session_secret: Option<String>,
    /// `SESSION_SECRET_OLD`.
    pub session_secret_old: Option<String>,
    /// `ENCRYPT_LOCAL_SESSION` (default true).
    pub encrypt_local_session: bool,
    /// `ALLOW_MEDIA` (default false).
    pub allow_media: bool,
    /// `HEADLESS` (default true).
    pub headless: bool,
    /// `LOG_LEVEL`.
    pub log_level: LogLevel,
    /// `NO_SANDBOX` (default false).
    pub no_sandbox: bool,
    /// `NAV_TIMEOUT`.
    pub nav_timeout: u64,
    /// `NAV_TIMEOUT_SHORT`.
    pub nav_timeout_short: u64,
    /// `SELECTOR_TIMEOUT`.
    pub selector_timeout: u64,
    /// `ELEMENT_TIMEOUT`.
    pub element_timeout: u64,
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
    /// `SCROLL_WAIT_SECONDS`.
    pub scroll_wait_seconds: u64,
    /// `LOCK_STALE_TIMEOUT_MS`.
    pub lock_stale_timeout_ms: u64,
    /// `CAPTCHA_COOLDOWN_HOURS` (0 desliga).
    pub captcha_cooldown_hours: u64,
    /// `TASK_RETRY_UNFINISHED`.
    pub task_retry_unfinished: bool,
    /// `TASK_RETRY_PASSES`.
    pub task_retry_passes: u64,
    /// `TASK_RETRY_DELAY_MS`.
    pub task_retry_delay_ms: u64,
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
    /// `SKIP_APP_ONLY_TASKS`.
    pub skip_app_only_tasks: bool,
    /// `PW_TRACE`.
    pub pw_trace: String,
    /// `PW_SCREENSHOT`.
    pub pw_screenshot: String,
    /// `PW_VIDEO`.
    pub pw_video: String,
    /// `PW_OUTPUT_DIR`.
    pub pw_output_dir: PathBuf,
    /// `TELEGRAM_ENABLED`.
    pub telegram_enabled: bool,
    /// `TELEGRAM_BOT_TOKEN`.
    pub telegram_bot_token: String,
    /// `TELEGRAM_CHAT_ID`.
    pub telegram_chat_id: String,
    /// `TELEGRAM_SILENT`.
    pub telegram_silent: bool,
    /// `TELEGRAM_PER_ACCOUNT`.
    pub telegram_per_account: bool,
    /// `TELEGRAM_TIMEOUT_MS`.
    pub telegram_timeout_ms: u64,
    /// `NOTIFY_HOST_LABEL` (máx. 64).
    pub notify_host_label: String,
    /// `NOTIFY_WEBHOOK_URL`.
    pub notify_webhook_url: String,
    /// `HEARTBEAT_ENABLED`.
    pub heartbeat_enabled: bool,
    /// `HEARTBEAT_URL`.
    pub heartbeat_url: String,
    /// `HEARTBEAT_TIMEOUT_MS`.
    pub heartbeat_timeout_ms: u64,
    /// `ALLOW_PRIVATE_WEBHOOKS`.
    pub allow_private_webhooks: bool,
    /// Diretório base do projeto (equivalente ao `__dirname`).
    pub base_dir: PathBuf,
}

impl Config {
    /// Carrega e valida a configuração (equivalente a `loadConfig`).
    pub fn load(
        env: &EnvSource,
        base_dir: &Path,
        require_credentials: bool,
        notify_override: Option<bool>,
        heartbeat_override: Option<bool>,
    ) -> Result<Self, ConfigError> {
        let issues: std::cell::RefCell<Vec<ValidationIssue>> = std::cell::RefCell::new(Vec::new());
        let mut push = |path: &str, message: &str| {
            issues.borrow_mut().push(ValidationIssue {
                path: path.to_string(),
                message: message.to_string(),
            });
        };

        let mut ali_user_raw = env.get("ALI_USER").map(str::to_string);
        let mut ali_password_raw = env.get("ALI_PASSWORD").map(str::to_string);
        if !require_credentials {
            if ali_user_raw.as_deref().is_none_or(str::is_empty) {
                ali_user_raw = Some("placeholder@ali.local".to_string());
            }
            if ali_password_raw.as_deref().is_none_or(str::is_empty) {
                ali_password_raw = Some("placeholder_password".to_string());
            }
        }

        let ali_user = match ali_user_raw {
            None => {
                push(
                    "ALI_USER",
                    "A variável ALI_USER é obrigatória no credentials.env.",
                );
                String::new()
            }
            Some(value) => {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    push("ALI_USER", "ALI_USER não pode estar vazio.");
                }
                trimmed
            }
        };
        let ali_password = match ali_password_raw {
            None => {
                push(
                    "ALI_PASSWORD",
                    "A variável ALI_PASSWORD é obrigatória no credentials.env.",
                );
                String::new()
            }
            Some(value) => {
                if value.is_empty() {
                    push("ALI_PASSWORD", "ALI_PASSWORD não pode estar vazio.");
                }
                value
            }
        };

        let session_secret = optional_secret(env, "SESSION_SECRET", &mut push);
        let session_secret_old = optional_secret(env, "SESSION_SECRET_OLD", &mut push);

        let encrypt_local_session = bool_encrypt_local(env.get("ENCRYPT_LOCAL_SESSION"));
        let allow_media = bool_true_exact(env.get("ALLOW_MEDIA"));
        let headless = bool_headless(env.get("HEADLESS"));
        let log_level = parse_log_level(env.get("LOG_LEVEL"), &mut push);
        let no_sandbox = bool_true_exact(env.get("NO_SANDBOX"));

        let nav_timeout = positive_int(env.get("NAV_TIMEOUT"), 35_000);
        let nav_timeout_short = positive_int(env.get("NAV_TIMEOUT_SHORT"), 20_000);
        let selector_timeout = positive_int(env.get("SELECTOR_TIMEOUT"), 8_000);
        let element_timeout = positive_int(env.get("ELEMENT_TIMEOUT"), 4_000);
        let task_max_actions = positive_int(env.get("TASK_MAX_ACTIONS"), 25);
        let task_max_attempts = positive_int(env.get("TASK_MAX_ATTEMPTS"), 4);
        let task_round_max_attempts = positive_int(env.get("TASK_ROUND_MAX_ATTEMPTS"), 3);
        let task_max_duration_ms = positive_int(env.get("TASK_MAX_DURATION_MS"), 180_000);
        let task_scroll_max_ms = positive_int(env.get("TASK_SCROLL_MAX_MS"), 30_000);
        let scroll_wait_seconds = positive_int(env.get("SCROLL_WAIT_SECONDS"), 10);
        let lock_stale_timeout_ms = positive_int(env.get("LOCK_STALE_TIMEOUT_MS"), 1_800_000);

        // BUG CONHECIDO DO ORÁCULO (v1.7.1): `CAPTCHA_COOLDOWN_HOURS` não é
        // incluído no `rawEnv` de `config.js`, então o schema sempre aplica o
        // default 12 e o valor do credentials.env é ignorado (verificável em
        // `config.CAPTCHA_COOLDOWN_HOURS` e usado por `collect.js`/`all.js`).
        // Paridade primeiro (ADR-0006): replicamos o comportamento; quando o
        // upstream corrigir, ajustamos aqui e nos cenários de paridade.
        let captcha_cooldown_hours = 12_u64;

        let task_retry_unfinished = bool_true_exact(env.get("TASK_RETRY_UNFINISHED"));
        let task_retry_passes = positive_int(env.get("TASK_RETRY_PASSES"), 1);
        let task_retry_delay_ms = positive_int(env.get("TASK_RETRY_DELAY_MS"), 5_000);

        let task_pause_min_ms = non_negative_int(env.get("TASK_PAUSE_MIN_MS"), 0);
        let task_pause_max_ms = non_negative_int(env.get("TASK_PAUSE_MAX_MS"), 0);
        let account_delay_min_ms = non_negative_int(env.get("ACCOUNT_DELAY_MIN_MS"), 0);
        let account_delay_max_ms = non_negative_int(env.get("ACCOUNT_DELAY_MAX_MS"), 0);
        let start_delay_min_ms = non_negative_int(env.get("START_DELAY_MIN_MS"), 0);
        let start_delay_max_ms = non_negative_int(env.get("START_DELAY_MAX_MS"), 0);

        let skip_app_only_tasks = bool_skip_app_only(env.get("SKIP_APP_ONLY_TASKS"));

        let pw_trace = parse_enum(
            env.get("PW_TRACE"),
            &["off", "on", "retain-on-failure", "on-first-retry"],
            "retain-on-failure",
            "PW_TRACE",
            &mut push,
        );
        let pw_screenshot = parse_enum(
            env.get("PW_SCREENSHOT"),
            &["off", "on", "only-on-failure"],
            "only-on-failure",
            "PW_SCREENSHOT",
            &mut push,
        );
        let pw_video = parse_enum(
            env.get("PW_VIDEO"),
            &["off", "on", "retain-on-failure", "on-first-retry"],
            "off",
            "PW_VIDEO",
            &mut push,
        );
        let pw_output_dir = env.get("PW_OUTPUT_DIR").map_or_else(
            || base_dir.join("scratch").to_string_lossy().into_owned(),
            str::to_string,
        );
        let pw_output_dir = PathBuf::from(pw_output_dir);

        let telegram_override_raw = notify_override.map(|value| value.to_string());
        let telegram_enabled = if require_credentials {
            bool_true_exact(
                telegram_override_raw
                    .as_deref()
                    .or(env.get("TELEGRAM_ENABLED")),
            )
        } else {
            false
        };
        let telegram_bot_token = env
            .get("TELEGRAM_BOT_TOKEN")
            .unwrap_or("")
            .trim()
            .to_string();
        let telegram_chat_id = env.get("TELEGRAM_CHAT_ID").unwrap_or("").trim().to_string();
        let telegram_silent = bool_true_exact(env.get("TELEGRAM_SILENT"));
        let telegram_per_account = bool_true_exact(env.get("TELEGRAM_PER_ACCOUNT"));
        let telegram_timeout_ms = positive_int(env.get("TELEGRAM_TIMEOUT_MS"), 15_000);
        let notify_host_label = env
            .get("NOTIFY_HOST_LABEL")
            .unwrap_or("")
            .trim()
            .chars()
            .take(64)
            .collect::<String>();
        let notify_webhook_url = env
            .get("NOTIFY_WEBHOOK_URL")
            .unwrap_or("")
            .trim()
            .to_string();

        let raw_heartbeat_url = env.get("HEARTBEAT_URL").unwrap_or("").trim().to_string();
        let heartbeat_enabled_raw: String = if let Some(value) = heartbeat_override {
            value.to_string()
        } else {
            match env.get("HEARTBEAT_ENABLED").map(str::trim) {
                Some(value) if !value.is_empty() => value.to_string(),
                _ => (!raw_heartbeat_url.is_empty()).to_string(),
            }
        };
        let heartbeat_enabled = if require_credentials {
            bool_true_exact(Some(&heartbeat_enabled_raw))
        } else {
            false
        };
        let heartbeat_timeout_ms = positive_int(env.get("HEARTBEAT_TIMEOUT_MS"), 5_000);

        let allow_private_webhooks = allow_private_targets(env);

        // `superRefine` só roda quando o schema base passa (semântica do Zod).
        if issues.borrow().is_empty() {
            if task_pause_max_ms < task_pause_min_ms {
                push(
                    "TASK_PAUSE_MAX_MS",
                    "TASK_PAUSE_MAX_MS deve ser >= TASK_PAUSE_MIN_MS.",
                );
            }
            if account_delay_max_ms < account_delay_min_ms {
                push(
                    "ACCOUNT_DELAY_MAX_MS",
                    "ACCOUNT_DELAY_MAX_MS deve ser >= ACCOUNT_DELAY_MIN_MS.",
                );
            }
            if start_delay_max_ms < start_delay_min_ms {
                push(
                    "START_DELAY_MAX_MS",
                    "START_DELAY_MAX_MS deve ser >= START_DELAY_MIN_MS.",
                );
            }
            for (key, value) in [
                ("TASK_PAUSE_MAX_MS", task_pause_max_ms),
                ("ACCOUNT_DELAY_MAX_MS", account_delay_max_ms),
                ("START_DELAY_MAX_MS", start_delay_max_ms),
            ] {
                if value > MAX_DELAY_MS {
                    push(
                        key,
                        &format!(
                            "{key} deve ser <= {MAX_DELAY_MS} (24 h). Lembre que o valor é em milissegundos."
                        ),
                    );
                }
            }
            if telegram_enabled {
                if !is_valid_telegram_token(&telegram_bot_token) {
                    push(
                        "TELEGRAM_BOT_TOKEN",
                        "TELEGRAM_BOT_TOKEN é obrigatório e deve ter formato válido (/^\\d+:[\\w-]{30,}$/) quando TELEGRAM_ENABLED=true.",
                    );
                }
                if telegram_chat_id.trim().is_empty() {
                    push(
                        "TELEGRAM_CHAT_ID",
                        "TELEGRAM_CHAT_ID é obrigatório quando TELEGRAM_ENABLED=true.",
                    );
                }
            }
            if heartbeat_enabled {
                if raw_heartbeat_url.is_empty() || !starts_with_http(&raw_heartbeat_url) {
                    push(
                        "HEARTBEAT_URL",
                        "HEARTBEAT_URL é obrigatória e deve ser uma URL válida (http/https) quando HEARTBEAT_ENABLED=true.",
                    );
                } else if let Some(url) = parse_url(&raw_heartbeat_url) {
                    let host = host_string(&url);
                    if url.scheme() == "http" && !is_loopback_host(&host) && !allow_private_webhooks
                    {
                        push(
                            "HEARTBEAT_URL",
                            "HEARTBEAT_URL deve usar https:// (o token do dead man's switch não deve trafegar em claro). http:// é aceito apenas para localhost ou com ALLOW_PRIVATE_WEBHOOKS=true.",
                        );
                    }
                }
            }
            if !notify_webhook_url.is_empty() {
                if !starts_with_http(&notify_webhook_url) {
                    push(
                        "NOTIFY_WEBHOOK_URL",
                        "NOTIFY_WEBHOOK_URL deve ser uma URL válida começando com http:// ou https://.",
                    );
                } else if let Some(url) = parse_url(&notify_webhook_url) {
                    let host = host_string(&url);
                    if url.scheme() == "http" && !is_loopback_host(&host) && !allow_private_webhooks
                    {
                        push(
                            "NOTIFY_WEBHOOK_URL",
                            "NOTIFY_WEBHOOK_URL deve usar https:// (o webhook não deve trafegar em claro). http:// é aceito apenas para localhost ou com ALLOW_PRIVATE_WEBHOOKS=true.",
                        );
                    }
                } else {
                    push(
                        "NOTIFY_WEBHOOK_URL",
                        "NOTIFY_WEBHOOK_URL deve ser uma URL válida começando com http:// ou https://.",
                    );
                }
            }
            if encrypt_local_session && session_secret.is_none() {
                push(
                    "SESSION_SECRET",
                    "SESSION_SECRET é obrigatório (>= 32 caracteres) quando ENCRYPT_LOCAL_SESSION=true (padrão): sem ele a sessão NÃO é persistida. Defina SESSION_SECRET ou use ENCRYPT_LOCAL_SESSION=false explicitamente.",
                );
            }
        }

        let collected = issues.take();
        if !collected.is_empty() {
            return Err(ConfigError::Validation { issues: collected });
        }

        Ok(Self {
            ali_user,
            ali_password,
            session_secret,
            session_secret_old,
            encrypt_local_session,
            allow_media,
            headless,
            log_level,
            no_sandbox,
            nav_timeout,
            nav_timeout_short,
            selector_timeout,
            element_timeout,
            task_max_actions,
            task_max_attempts,
            task_round_max_attempts,
            task_max_duration_ms,
            task_scroll_max_ms,
            scroll_wait_seconds,
            lock_stale_timeout_ms,
            captcha_cooldown_hours,
            task_retry_unfinished,
            task_retry_passes,
            task_retry_delay_ms,
            task_pause_min_ms,
            task_pause_max_ms,
            account_delay_min_ms,
            account_delay_max_ms,
            start_delay_min_ms,
            start_delay_max_ms,
            skip_app_only_tasks,
            pw_trace,
            pw_screenshot,
            pw_video,
            pw_output_dir,
            telegram_enabled,
            telegram_bot_token,
            telegram_chat_id,
            telegram_silent,
            telegram_per_account,
            telegram_timeout_ms,
            notify_host_label,
            notify_webhook_url,
            heartbeat_enabled,
            heartbeat_url: raw_heartbeat_url,
            heartbeat_timeout_ms,
            allow_private_webhooks,
            base_dir: base_dir.to_path_buf(),
        })
    }
}

/// `ALLOW_PRIVATE_WEBHOOKS` (true/1/on/yes, case-insensitive).
#[must_use]
pub fn allow_private_targets(env: &EnvSource) -> bool {
    matches!(
        env.get("ALLOW_PRIVATE_WEBHOOKS")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("true" | "1" | "on" | "yes")
    )
}

fn optional_secret(
    env: &EnvSource,
    key: &str,
    push: &mut impl FnMut(&str, &str),
) -> Option<String> {
    match env.get(key) {
        None => None,
        Some(value) => {
            if value.chars().count() < 32 {
                push(key, &format!("{key} deve conter no mínimo 32 caracteres."));
                None
            } else {
                Some(value.to_string())
            }
        }
    }
}

fn parse_log_level(raw: Option<&str>, push: &mut impl FnMut(&str, &str)) -> LogLevel {
    match raw {
        None => LogLevel::Info,
        Some(value) => {
            if let Some(level) = LogLevel::parse(value.trim()) {
                level
            } else {
                push("LOG_LEVEL", &enum_error(&LogLevel::VALUES));
                LogLevel::Info
            }
        }
    }
}

fn parse_enum(
    raw: Option<&str>,
    values: &[&str],
    default: &str,
    key: &str,
    push: &mut impl FnMut(&str, &str),
) -> String {
    match raw {
        None => default.to_string(),
        Some(value) => {
            if values.contains(&value) {
                value.to_string()
            } else {
                push(key, &enum_error(values));
                default.to_string()
            }
        }
    }
}

fn enum_error(values: &[&str]) -> String {
    let joined = values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join("|");
    format!("Invalid option: expected one of {joined}")
}

fn is_valid_telegram_token(token: &str) -> bool {
    let Some((prefix, suffix)) = token.split_once(':') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_digit())
        && suffix.chars().count() >= 30
        && suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn starts_with_http(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn parse_url(value: &str) -> Option<Url> {
    Url::parse(value).ok()
}

fn host_string(url: &Url) -> String {
    match url.host() {
        Some(Host::Domain(domain)) => domain.trim_end_matches('.').to_ascii_lowercase(),
        Some(Host::Ipv4(ip)) => ip.to_string(),
        Some(Host::Ipv6(ip)) => ip.to_string().to_ascii_lowercase(),
        None => String::new(),
    }
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.starts_with("::ffff:127.") || is_127_range(host)
}

fn is_127_range(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 || parts[0] != "127" {
        return false;
    }
    parts[1..]
        .iter()
        .all(|part| !part.is_empty() && part.len() <= 3 && part.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn env_with(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn config_minima_valida() {
        let env = env_with(&[
            ("ALI_USER", "user@example.com"),
            ("ALI_PASSWORD", "senha"),
            ("SESSION_SECRET", "0123456789abcdef0123456789abcdef"),
        ]);
        let config = Config::load(&env, Path::new("/tmp"), true, None, None).expect("válida");
        assert_eq!(config.ali_user, "user@example.com");
        assert_eq!(config.log_level, LogLevel::Info);
        assert_eq!(config.nav_timeout, 35_000);
        assert!(config.encrypt_local_session);
        assert_eq!(config.pw_output_dir, Path::new("/tmp").join("scratch"));
    }

    #[test]
    fn usuario_obrigatorio() {
        let env = env_with(&[("ALI_PASSWORD", "senha")]);
        let err = Config::load(&env, Path::new("/tmp"), true, None, None).unwrap_err();
        assert_eq!(err.issues()[0].path, "ALI_USER");
        assert!(err.issues()[0].message.contains("obrigatória"));
    }

    #[test]
    fn session_secret_obrigatorio_com_criptografia() {
        let env = env_with(&[("ALI_USER", "u@e.com"), ("ALI_PASSWORD", "p")]);
        let err = Config::load(&env, Path::new("/tmp"), true, None, None).unwrap_err();
        assert!(
            err.issues()
                .iter()
                .any(|issue| issue.path == "SESSION_SECRET"
                    && issue.message.contains("ENCRYPT_LOCAL_SESSION"))
        );
    }

    #[test]
    fn pausas_invertidas_e_teto_de_24h() {
        let env = env_with(&[
            ("ALI_USER", "u@e.com"),
            ("ALI_PASSWORD", "p"),
            ("ENCRYPT_LOCAL_SESSION", "false"),
            ("TASK_PAUSE_MIN_MS", "5000"),
            ("TASK_PAUSE_MAX_MS", "1000"),
            ("START_DELAY_MAX_MS", "99999999999"),
        ]);
        let err = Config::load(&env, Path::new("/tmp"), true, None, None).unwrap_err();
        let paths: Vec<&str> = err
            .issues()
            .iter()
            .map(|issue| issue.path.as_str())
            .collect();
        assert!(paths.contains(&"TASK_PAUSE_MAX_MS"));
        assert!(paths.contains(&"START_DELAY_MAX_MS"));
    }

    #[test]
    fn telegram_habilitado_exige_token_e_chat() {
        let env = env_with(&[
            ("ALI_USER", "u@e.com"),
            ("ALI_PASSWORD", "p"),
            ("ENCRYPT_LOCAL_SESSION", "false"),
            ("TELEGRAM_ENABLED", "true"),
        ]);
        let err = Config::load(&env, Path::new("/tmp"), true, None, None).unwrap_err();
        let paths: Vec<&str> = err
            .issues()
            .iter()
            .map(|issue| issue.path.as_str())
            .collect();
        assert!(paths.contains(&"TELEGRAM_BOT_TOKEN"));
        assert!(paths.contains(&"TELEGRAM_CHAT_ID"));
    }

    #[test]
    fn heartbeat_e_webhook_http_fora_de_loopback() {
        let env = env_with(&[
            ("ALI_USER", "u@e.com"),
            ("ALI_PASSWORD", "p"),
            ("ENCRYPT_LOCAL_SESSION", "false"),
            ("HEARTBEAT_ENABLED", "true"),
            ("HEARTBEAT_URL", "http://example.com/ping"),
            ("NOTIFY_WEBHOOK_URL", "http://example.com/hook"),
        ]);
        let err = Config::load(&env, Path::new("/tmp"), true, None, None).unwrap_err();
        let messages: Vec<&str> = err
            .issues()
            .iter()
            .map(|issue| issue.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("HEARTBEAT_URL deve usar https://"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("NOTIFY_WEBHOOK_URL deve usar https://"))
        );
    }

    #[test]
    fn heartbeat_http_loopback_e_permitido() {
        let env = env_with(&[
            ("ALI_USER", "u@e.com"),
            ("ALI_PASSWORD", "p"),
            ("ENCRYPT_LOCAL_SESSION", "false"),
            ("HEARTBEAT_ENABLED", "true"),
            ("HEARTBEAT_URL", "http://127.0.0.1:8080/ping"),
        ]);
        assert!(Config::load(&env, Path::new("/tmp"), true, None, None).is_ok());
    }

    #[test]
    fn heartbeat_url_deriva_enabled_quando_env_ausente() {
        let env = env_with(&[
            ("ALI_USER", "u@e.com"),
            ("ALI_PASSWORD", "p"),
            ("ENCRYPT_LOCAL_SESSION", "false"),
            ("HEARTBEAT_URL", "https://hc-ping.com/uuid"),
        ]);
        let config = Config::load(&env, Path::new("/tmp"), true, None, None).expect("válida");
        assert!(config.heartbeat_enabled);
        assert_eq!(config.heartbeat_timeout_ms, 5_000);
    }

    #[test]
    fn override_de_notify_desliga_mesmo_com_env_ligado() {
        let env = env_with(&[
            ("ALI_USER", "u@e.com"),
            ("ALI_PASSWORD", "p"),
            ("ENCRYPT_LOCAL_SESSION", "false"),
            ("TELEGRAM_ENABLED", "true"),
            (
                "TELEGRAM_BOT_TOKEN",
                "123:abcdefghijklmnopqrstuvwxyz0123456789",
            ),
            ("TELEGRAM_CHAT_ID", "1"),
        ]);
        let config =
            Config::load(&env, Path::new("/tmp"), true, Some(false), None).expect("válida");
        assert!(!config.telegram_enabled);
    }
}
