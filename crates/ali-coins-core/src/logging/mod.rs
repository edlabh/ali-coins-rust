//! Logging estruturado com redaction compatível com `logger.js` do oráculo.
//!
//! Contratos preservados (ADR-0005):
//! - `--json`: todas as linhas de log vão para **stderr**; stdout fica livre.
//! - Sem `--json`: JSON em stdout quando não há TTY; formato humano (aproximado
//!   ao `pino-pretty`) quando stdout é TTY.
//! - Níveis trace…silent, com `fatal` para o crash handler.
//! - Redaction: chaves sensíveis recursivas, sanitização de query strings,
//!   token de bot Telegram, `Bearer` e linhas de cookie/authorization.
//!
//! O layout exato do `pino-pretty` é classificado como **flexível** em
//! `docs/05-divergencias-conhecidas.md`.

use crate::config::LogLevel;
use chrono::Utc;
use regex::Regex;
use serde_json::{Map, Value};
use std::io::{IsTerminal as _, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Profundidade máxima de scrubbing (igual ao oráculo).
pub const MAX_LOG_DEPTH: usize = 12;
/// Marcador de valor redigido.
pub const REDACTED: &str = "[REDACTED]";

const SENSITIVE_KEY_PATTERN: &str = r"(?i)(secret|passwd|password|\bpass\b|\bpwd\b|senha|token|cookie|authorization|\bbearer\b|credential|access[_-]?key|private[_-]?key|api[_-]?key)";

const QUERY_PATTERN: &str = r"(?i)([?&;#](?:access_token|api[_-]?key|apikey|auth|authorization|code|password|passwd|secret|session|ticket|token)=)[^&#;\s]+";
const BOT_TOKEN_PATTERN: &str = r"(?i)(bot\d+:[\w-]{20,})";
const BEARER_PATTERN: &str = r"(?i)(Bearer\s+)[\w\-._~+/=]+";
const HEADER_PATTERN: &str =
    r"(?i)((?:authorization|cookie|set-cookie|x-api-key)\s*[:=]\s*)[^\r\n]+";

fn regex(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static Regex>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut cache = cache.lock().expect("cache de regex");
    if let Some(found) = cache.get(pattern) {
        return found;
    }
    let compiled: &'static Regex = Box::leak(Box::new(Regex::new(pattern).expect("regex válida")));
    cache.insert(pattern, compiled);
    compiled
}

/// Sanitiza parâmetros sensíveis em strings/URLs (query, bot token, Bearer, headers).
#[must_use]
pub fn sanitize_sensitive_query_params(input: &str) -> String {
    let result = regex(QUERY_PATTERN).replace_all(input, "${1}[REDACTED]");
    let result = regex(BOT_TOKEN_PATTERN).replace_all(&result, "bot[REDACTED_TOKEN]");
    let result = regex(BEARER_PATTERN).replace_all(&result, "${1}[REDACTED]");
    regex(HEADER_PATTERN)
        .replace_all(&result, "${1}[REDACTED]")
        .into_owned()
}

/// Chave cujo nome sugere segredo?
#[must_use]
pub fn is_sensitive_key(key: &str) -> bool {
    regex(SENSITIVE_KEY_PATTERN).is_match(key)
}

/// Mascara recursivamente valores de chaves sensíveis (sem alterar o original).
#[must_use]
pub fn scrub_sensitive_fields(value: &Value, depth: usize) -> Value {
    if depth > MAX_LOG_DEPTH {
        return value.clone();
    }
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| scrub_sensitive_fields(item, depth + 1))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                if is_sensitive_key(key) {
                    out.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else {
                    out.insert(key.clone(), scrub_sensitive_fields(val, depth + 1));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Aplica a sanitização de strings em todos os valores (recursivo).
#[must_use]
pub fn sanitize_log_strings(value: &Value, depth: usize) -> Value {
    if depth > MAX_LOG_DEPTH {
        return value.clone();
    }
    match value {
        Value::String(text) => Value::String(sanitize_sensitive_query_params(text)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_log_strings(item, depth + 1))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                out.insert(key.clone(), sanitize_log_strings(val, depth + 1));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Pipeline completo: scrub de chaves + sanitização de strings.
#[must_use]
pub fn sanitize_record(value: &Value) -> Value {
    sanitize_log_strings(&scrub_sensitive_fields(value, 0), 0)
}

/// Formato de saída.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// JSON estruturado (pino-like).
    Json,
    /// Formato humano aproximado.
    Human,
}

/// Destino das linhas de log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSink {
    /// stdout.
    Stdout,
    /// stderr (obrigatório em `--json`).
    Stderr,
}

/// Logger estruturado.
#[derive(Debug)]
pub struct Logger {
    level: AtomicU8,
    format: LogFormat,
    sink: LogSink,
}

impl Logger {
    /// Cria um logger.
    #[must_use]
    pub fn new(level: LogLevel, format: LogFormat, sink: LogSink) -> Self {
        Self {
            level: AtomicU8::new(level_u8(level)),
            format,
            sink,
        }
    }

    /// Nível atual.
    #[must_use]
    pub fn level(&self) -> LogLevel {
        level_from_u8(self.level.load(Ordering::Relaxed))
    }

    /// Atualiza o nível (equivalente a `updateLogLevel`).
    pub fn set_level(&self, level: LogLevel) {
        self.level.store(level_u8(level), Ordering::Relaxed);
    }

    /// O nível está habilitado?
    #[must_use]
    pub fn enabled(&self, level: LogLevel) -> bool {
        // Ordenação: trace (0) é o mais verboso; o corte habilita >= nível atual.
        level_u8(level) >= self.level.load(Ordering::Relaxed)
    }

    /// Registra uma linha.
    pub fn log(&self, level: LogLevel, msg: &str, fields: &[(&str, Value)]) {
        if !self.enabled(level) {
            return;
        }
        let msg = sanitize_sensitive_query_params(msg);
        let mut record = Map::new();
        record.insert("level".to_string(), Value::from(level_number(level)));
        record.insert(
            "time".to_string(),
            Value::String(Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
        );
        record.insert("pid".to_string(), Value::from(std::process::id()));
        record.insert("msg".to_string(), Value::String(msg.clone()));
        for (key, value) in fields {
            record.insert((*key).to_string(), sanitize_record(value));
        }
        let record = Value::Object(record);

        let line = match self.format {
            LogFormat::Json => serde_json::to_string(&record).unwrap_or_default(),
            LogFormat::Human => {
                let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                format!("SYS {time} {}: {msg}", level.as_str().to_uppercase())
            }
        };
        match self.sink {
            LogSink::Stdout => {
                let mut handle = std::io::stdout().lock();
                let _ = writeln!(handle, "{line}");
            }
            LogSink::Stderr => {
                let mut handle = std::io::stderr().lock();
                let _ = writeln!(handle, "{line}");
            }
        }
    }

    /// Atalho para `trace`.
    pub fn trace(&self, msg: &str, fields: &[(&str, Value)]) {
        self.log(LogLevel::Trace, msg, fields);
    }

    /// Atalho para `debug`.
    pub fn debug(&self, msg: &str, fields: &[(&str, Value)]) {
        self.log(LogLevel::Debug, msg, fields);
    }

    /// Atalho para `info`.
    pub fn info(&self, msg: &str, fields: &[(&str, Value)]) {
        self.log(LogLevel::Info, msg, fields);
    }

    /// Atalho para `warn`.
    pub fn warn(&self, msg: &str, fields: &[(&str, Value)]) {
        self.log(LogLevel::Warn, msg, fields);
    }

    /// Atalho para `error`.
    pub fn error(&self, msg: &str, fields: &[(&str, Value)]) {
        self.log(LogLevel::Error, msg, fields);
    }

    /// Atalho para `fatal`.
    pub fn fatal(&self, msg: &str, fields: &[(&str, Value)]) {
        self.log(LogLevel::Fatal, msg, fields);
    }

    /// Força o flush dos streams padrão.
    pub fn flush(&self) {
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
    }
}

/// Inicializa o logger global (idempotente; atualiza o nível se já iniciado).
#[must_use]
pub fn init(level: LogLevel, json_mode: bool) -> &'static Logger {
    let global = global();
    global.set_level(level);
    let _ = json_mode;
    global
}

/// Logger global (JSON em stderr com `--json`; humano em TTY; JSON em stdout caso contrário).
#[must_use]
pub fn global() -> &'static Logger {
    static GLOBAL: OnceLock<Logger> = OnceLock::new();
    GLOBAL.get_or_init(|| {
        let json_mode = std::env::args().any(|arg| arg == "--json");
        let format = if json_mode || !std::io::stdout().is_terminal() {
            LogFormat::Json
        } else {
            LogFormat::Human
        };
        let sink = if json_mode {
            LogSink::Stderr
        } else {
            LogSink::Stdout
        };
        Logger::new(LogLevel::Info, format, sink)
    })
}

fn level_u8(level: LogLevel) -> u8 {
    match level {
        LogLevel::Trace => 0,
        LogLevel::Debug => 1,
        LogLevel::Info => 2,
        LogLevel::Warn => 3,
        LogLevel::Error => 4,
        LogLevel::Fatal => 5,
        LogLevel::Silent => 6,
    }
}

fn level_from_u8(value: u8) -> LogLevel {
    match value {
        0 => LogLevel::Trace,
        1 => LogLevel::Debug,
        2 => LogLevel::Info,
        3 => LogLevel::Warn,
        4 => LogLevel::Error,
        5 => LogLevel::Fatal,
        _ => LogLevel::Silent,
    }
}

/// Número do nível no formato do pino.
#[must_use]
pub fn level_number(level: LogLevel) -> u8 {
    match level {
        LogLevel::Trace => 10,
        LogLevel::Debug => 20,
        LogLevel::Info => 30,
        LogLevel::Warn => 40,
        LogLevel::Error => 50,
        LogLevel::Fatal => 60,
        LogLevel::Silent => 100,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitiza_query_params() {
        let input = "https://x.com/cb?token=segredo&ok=1&session=abc#code=xyz";
        let out = sanitize_sensitive_query_params(input);
        assert!(!out.contains("segredo"));
        assert!(!out.contains("abc"));
        assert!(!out.contains("xyz"));
        assert!(out.contains("ok=1"));
    }

    #[test]
    fn sanitiza_bot_token_bearer_e_headers() {
        let input = "bot123456789:AAbbCCddEEffGGhhIIjjKKllMMnnOOpp pedido";
        let out = sanitize_sensitive_query_params(input);
        assert!(out.contains("bot[REDACTED_TOKEN]"));

        let bearer = sanitize_sensitive_query_params("Authorization: Bearer abc.def.ghi");
        assert!(!bearer.contains("abc.def.ghi"));

        let cookie = sanitize_sensitive_query_params("cookie: a=1; b=2; c=3\nproxima");
        assert!(cookie.contains("cookie: [REDACTED]"));
        assert!(!cookie.contains("b=2"));
        assert!(cookie.contains("proxima"));
    }

    #[test]
    fn scrub_de_chaves_sensiveis_recursivo() {
        let value = json!({
            "user": "ok",
            "ALI_PASSWORD": "segredo",
            "nested": { "apiKey": "k", "my_secret_field": "s", "deep": [ { "token": "t" } ] }
        });
        let scrubbed = scrub_sensitive_fields(&value, 0);
        assert_eq!(scrubbed["ALI_PASSWORD"], REDACTED);
        assert_eq!(scrubbed["nested"]["apiKey"], REDACTED);
        assert_eq!(scrubbed["nested"]["my_secret_field"], REDACTED);
        assert_eq!(scrubbed["nested"]["deep"][0]["token"], REDACTED);
        assert_eq!(scrubbed["user"], "ok");
    }

    #[test]
    fn sanitiza_strings_em_campos_estruturados() {
        let value = json!({
            "url": "https://x.com?a=1&token=segredo",
            "lista": ["https://y.com/cb?session=abc"],
        });
        let safe = sanitize_record(&value);
        assert!(!safe.to_string().contains("segredo"));
        assert!(!safe.to_string().contains("session=abc"));
    }

    #[test]
    fn niveis_e_numeracao() {
        let logger = Logger::new(LogLevel::Warn, LogFormat::Json, LogSink::Stderr);
        assert!(logger.enabled(LogLevel::Error));
        assert!(logger.enabled(LogLevel::Warn));
        assert!(!logger.enabled(LogLevel::Info));
        assert_eq!(level_number(LogLevel::Info), 30);
        assert_eq!(level_number(LogLevel::Fatal), 60);
        logger.set_level(LogLevel::Debug);
        assert!(logger.enabled(LogLevel::Debug));
    }
}
