//! Notificações via Telegram (equivalente a `libs/notify.js`).
//!
//! Mecânica fiel ao oráculo:
//! - `sendMessage` com `parse_mode: HTML` e `disable_notification` opcional.
//! - 3 tentativas com backoff 800ms·2^(n-1) (teto 2.5s, jitter 80–120%);
//!   retry apenas em **5xx/429**; **timeout ambíguo nunca retenta** (evita
//!   mensagem duplicada); `retry_after` do 429 respeitado (teto 30s).
//! - Fallback de 400: remove tags, reenvia sem `parse_mode`.
//! - Escaping HTML, truncamento em **4096 unidades UTF-16** sem quebrar
//!   surrogate/entidade e recuo até o último `\n` acima de 2000.
//!
//! Os templates cobrem os eventos do contrato C-12; a paridade byte-a-byte dos
//! snapshots está registrada como pendência em `docs/05-divergencias-conhecidas.md`.

use super::http::SafeHttpClient;
use crate::config::mask_user;
use crate::report::{
    CheckinInput, TasksInput, compute_checkin_coins_gained, compute_tasks_coins_gained,
};
use crate::time::{format_date, format_duration};
use rand::Rng as _;
use serde_json::{Map, Value};
use std::time::Duration;

/// Limite oficial de mensagem do Telegram (unidades UTF-16).
pub const TELEGRAM_MAX_UTF16: usize = 4096;
/// Corte seguro antes de anexar o aviso de truncamento.
pub const SAFE_TRUNCATE_UTF16: usize = 3900;
const RETRY_ATTEMPTS: u32 = 3;
const RETRY_BASE_MS: u64 = 800;
const RETRY_MAX_MS: u64 = 2500;
const RETRY_AFTER_MAX: Duration = Duration::from_secs(30);

/// Configuração efetiva do Telegram.
#[derive(Debug, Clone, Default)]
pub struct TelegramConfig {
    /// Habilitado.
    pub enabled: bool,
    /// Token do bot.
    pub bot_token: String,
    /// Chat ID (global).
    pub chat_id: String,
    /// Enviar sem som.
    pub silent: bool,
    /// Timeout por tentativa.
    pub timeout_ms: u64,
    /// Base da API (default `https://api.telegram.org`; injetável em testes).
    pub api_base: String,
}

/// Eventos do contrato de notificação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramEvent {
    /// Validação sem navegador.
    DryRun,
    /// Teste manual.
    ManualTest,
    /// Sucesso consolidado.
    Success,
    /// Check-in já feito hoje.
    AlreadyCollected,
    /// Falha de execução.
    Failure,
    /// Lock ativo.
    LockActive,
    /// Streak quebrado.
    StreakBreak,
    /// 2FA necessária.
    TwoFactorRequired,
    /// Captcha solicitado.
    CaptchaRequired,
    /// Cooldown de captcha liberado.
    CaptchaCooldownReleased,
    /// Check-in isolado.
    Checkin,
    /// Tarefas isoladas.
    Tasks,
    /// Relatório multi-conta.
    MultiAccount,
}

/// Contexto opcional para montar a mensagem.
#[derive(Debug, Clone, Default)]
pub struct TelegramContext<'a> {
    /// Usuário (mascarado pelo chamador quando necessário).
    pub user: Option<&'a str>,
    /// Saldo total.
    pub total_balance: Option<&'a str>,
    /// Moedas ganhas.
    pub coins_gained: Option<i64>,
    /// Streak atual.
    pub streak_days: Option<&'a str>,
    /// Streak anterior (quebra).
    pub previous_streak_days: Option<&'a str>,
    /// Duração total.
    pub duration: Option<&'a str>,
    /// Erro.
    pub error: Option<&'a str>,
    /// Host (rodapé).
    pub host: Option<&'a str>,
    /// Versão (rodapé).
    pub version: Option<&'a str>,
    /// Falha associada a sessão importada expirada
    /// (`checkIfImportedSessionExpired` do oráculo).
    pub imported_session_expired: bool,
}

/// Entradas do check de "sessão importada expirada" (port de
/// `checkIfImportedSessionExpired`).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportedSessionCheck<'a> {
    /// Mensagem do erro.
    pub error_message: Option<&'a str>,
    /// Flag estruturada no erro (`err.isImportedSessionExpired`).
    pub error_flag: bool,
    /// Flag no relatório (`report.isImportedSessionExpired`).
    pub report_flag: bool,
    /// Flags por conta (`accounts[].isImportedSessionExpired`).
    pub account_flags: &'a [bool],
    /// Relatório multi-conta (desliga o fallback pelo `session_meta.json`).
    pub report_is_multi: bool,
    /// `session_meta.json` indica sessão importada.
    pub meta_imported: bool,
}

/// Decide se a falha é de sessão importada expirada, na ordem do oráculo:
/// flags → padrão de mensagem → fallback pelo meta (só conta única).
#[must_use]
pub fn check_imported_session_expired(check: &ImportedSessionCheck<'_>) -> bool {
    if check.error_flag || check.report_flag || check.account_flags.iter().any(|flag| *flag) {
        return true;
    }
    let message = check.error_message.unwrap_or("");
    if message_matches_remote_session(message) {
        return true;
    }
    if check.report_is_multi {
        return false;
    }
    check.meta_imported && message_matches_auth_context(message)
}

/// `/sessão.*(importada|remota).*expir/i` ou `/node export_session\.js/i`.
fn message_matches_remote_session(message: &str) -> bool {
    let lower = message.to_lowercase();
    if let Some(start) = lower.find("sessão") {
        let rest = &lower[start + "sessão".len()..];
        let middle = [rest.find("importada"), rest.find("remota")]
            .into_iter()
            .flatten()
            .min();
        if let Some(middle) = middle {
            if rest[middle..].contains("expir") {
                return true;
            }
        }
    }
    lower.contains("node export_session.js")
}

/// `/login|autentic|sess[aã]o|streak|saldo|desafio|challenge/i`.
fn message_matches_auth_context(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "login",
        "autentic",
        "sessao",
        "sessão",
        "streak",
        "saldo",
        "desafio",
        "challenge",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Regra dos produtores do oráculo (`collect.js`/`do_tasks.js`): sessão
/// importada + erro de autenticação/navegação marca `isImportedSessionExpired`.
#[must_use]
pub fn imported_session_error_flag(error_message: Option<&str>, meta_imported: bool) -> bool {
    if !meta_imported {
        return false;
    }
    let Some(message) = error_message else {
        return false;
    };
    let lower = message.to_lowercase();
    [
        "login",
        "autentic",
        "sessao",
        "sessão",
        "streak",
        "saldo",
        "desafio",
        "challenge",
        "cookie",
        "navigat",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Check observável completo para os fluxos do port (produtores + notify).
#[must_use]
pub fn detect_imported_session_expired(error_message: Option<&str>, meta_imported: bool) -> bool {
    check_imported_session_expired(&ImportedSessionCheck {
        error_message,
        error_flag: imported_session_error_flag(error_message, meta_imported),
        report_flag: false,
        account_flags: &[],
        report_is_multi: false,
        meta_imported,
    })
}

/// Resultado do envio (nunca lança).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSendResult {
    /// Sucesso (2xx).
    pub ok: bool,
    /// Desabilitado/sem credenciais.
    pub skipped: bool,
    /// Status HTTP final.
    pub status: Option<u16>,
    /// Erro, quando houver.
    pub error: Option<String>,
}

/// Escapa `& < > "` para HTML do Telegram.
#[must_use]
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Trunca respeitando UTF-16, sem quebrar entidade HTML e com recuo de linha.
#[must_use]
pub fn truncate_telegram_message(text: &str) -> String {
    if text.encode_utf16().count() <= TELEGRAM_MAX_UTF16 {
        return text.to_string();
    }
    let mut utf16 = 0_usize;
    let mut cut = 0_usize;
    for (index, character) in text.char_indices() {
        utf16 += character.len_utf16();
        if utf16 > SAFE_TRUNCATE_UTF16 {
            break;
        }
        cut = index + character.len_utf8();
    }
    let mut sliced = text[..cut].to_string();
    if sliced.len() > 2000 {
        if let Some(position) = sliced.rfind('\n') {
            sliced.truncate(position);
        }
    }
    if let Some(position) = sliced.rfind('&') {
        if !sliced[position..].contains(';') {
            sliced.truncate(position);
        }
    }
    while sliced.ends_with('<') {
        sliced.pop();
    }
    sliced.push_str("\n… (mensagem truncada)");
    sliced
}

/// Sequência formatada como o oráculo (`N dias` ou `N/D`).
fn to_safe_streak(raw: Option<&str>) -> String {
    if let Some(text) = raw {
        let clean = text.trim();
        if !clean.is_empty() && clean.chars().all(|character| character.is_ascii_digit()) {
            if let Ok(parsed) = clean.parse::<i64>() {
                if parsed > 0 {
                    return format!("{parsed} dias");
                }
            }
        }
    }
    "N/D".to_string()
}

/// Extrai a mensagem de erro relevante (port de `extractRelevantErrorMessage`).
#[must_use]
pub fn extract_relevant_error_message(error: Option<&str>) -> String {
    const UNKNOWN: &str = "Erro desconhecido durante o processamento.";
    let Some(error) = error else {
        return UNKNOWN.to_string();
    };
    let raw = error.trim();
    if raw.is_empty() {
        return UNKNOWN.to_string();
    }
    let top_lines = |text: &str| -> Option<String> {
        let lines: Vec<&str> = text
            .split('\n')
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(3)
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    };
    let logs_split = regex::Regex::new(r"={5,}\s*logs?\s*={5,}").expect("regex de logs");
    if logs_split.is_match(raw) {
        if let Some(before) = logs_split.split(raw).next().map(str::trim) {
            if !before.is_empty() {
                if let Some(top) = top_lines(before) {
                    return top;
                }
            }
        }
    }
    let call_log = regex::Regex::new(r"(?i)Call log:").expect("regex call log");
    if call_log.is_match(raw) {
        if let Some(before) = call_log.split(raw).next().map(str::trim) {
            if !before.is_empty() {
                if let Some(top) = top_lines(before) {
                    return top;
                }
            }
        }
    }
    let lines: Vec<&str> = raw
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() <= 3 {
        return lines.join("\n");
    }
    let irrelevant = regex::Regex::new(r"(?i)^(at\s+|-\s*\[pid=|<\s*gracefully|\(?node:internal)")
        .expect("regex de linhas irrelevantes");
    let meaningful: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| !irrelevant.is_match(line))
        .collect();
    let indicator = regex::Regex::new(
        r"(?i)(error|fatal|fail|sandboxing|timeout|recusad|inválid|expirad|bloque|crash|exception)",
    )
    .expect("regex de indicadores");
    let error_lines: Vec<&str> = meaningful
        .iter()
        .copied()
        .filter(|line| indicator.is_match(line))
        .take(3)
        .collect();
    if !error_lines.is_empty() {
        return error_lines.join("\n");
    }
    if !meaningful.is_empty() {
        return meaningful
            .into_iter()
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
    }
    lines.into_iter().take(3).collect::<Vec<_>>().join("\n")
}

/// Redige segredos em URLs/headers textualizados (port de `sanitizeSensitiveQueryParams`).
#[must_use]
pub fn sanitize_sensitive_query_params(text: &str) -> String {
    static QUERY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static BOT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static BEARER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static HEADERS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let query = QUERY.get_or_init(|| {
        regex::Regex::new(
            r"(?i)([?&;#](?:access_token|api[_-]?key|apikey|auth|authorization|code|password|passwd|secret|session|ticket|token)=)[^&#;\s]+",
        )
        .expect("regex de query sensível")
    });
    let bot = BOT.get_or_init(|| {
        regex::Regex::new(r"(?i)(bot\d+:[\w-]{20,})").expect("regex de token de bot")
    });
    let bearer = BEARER
        .get_or_init(|| regex::Regex::new(r"(?i)(Bearer\s+)[\w\-._~+/=]+").expect("regex bearer"));
    let headers = HEADERS.get_or_init(|| {
        regex::Regex::new(r"(?i)((?:authorization|cookie|set-cookie|x-api-key)\s*[:=]\s*)[^\r\n]+")
            .expect("regex de headers")
    });
    let text = query.replace_all(text, "$1[REDACTED]");
    let text = bot.replace_all(&text, "bot[REDACTED_TOKEN]");
    let text = bearer.replace_all(&text, "$1[REDACTED]");
    headers.replace_all(&text, "$1[REDACTED]").to_string()
}

/// Host com versão (escapado), como o oráculo (`NOTIFY_HOST_LABEL` > hostname).
fn safe_host(ctx: &TelegramContext<'_>) -> String {
    let host = ctx.host.map_or_else(
        || {
            std::env::var("NOTIFY_HOST_LABEL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(crate::lock::hostname)
        },
        str::to_string,
    );
    let version = ctx.version.unwrap_or(env!("CARGO_PKG_VERSION"));
    escape_html(&format!("{host} (v{version})"))
}

/// Usuário exibido nas mensagens (ctx > `ALI_USER` mascarado > `desconhecida`).
fn message_user(ctx: &TelegramContext<'_>) -> String {
    if let Some(user) = ctx.user.filter(|value| !value.trim().is_empty()) {
        return user.to_string();
    }
    std::env::var("ALI_USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map_or_else(|| "desconhecida".to_string(), |value| mask_user(&value))
}

/// Nome do evento no formato do oráculo (para o fallback).
fn event_name(event: TelegramEvent) -> &'static str {
    match event {
        TelegramEvent::DryRun => "dry_run",
        TelegramEvent::ManualTest => "manual_test",
        TelegramEvent::Success => "success",
        TelegramEvent::AlreadyCollected => "already_collected",
        TelegramEvent::Failure => "failure",
        TelegramEvent::LockActive => "lock_active",
        TelegramEvent::StreakBreak => "streak_break",
        TelegramEvent::TwoFactorRequired => "2fa_required",
        TelegramEvent::CaptchaRequired => "captcha_required",
        TelegramEvent::CaptchaCooldownReleased => "captcha_cooldown_released",
        TelegramEvent::Checkin => "checkin",
        TelegramEvent::Tasks => "tasks",
        TelegramEvent::MultiAccount => "multi_account_report",
    }
}

/// Monta a mensagem HTML do evento no instante atual.
#[must_use]
pub fn build_message(event: TelegramEvent, ctx: &TelegramContext<'_>) -> String {
    build_message_at(event, ctx, chrono::Utc::now())
}

/// Monta a mensagem HTML do evento num instante fixo (testes de paridade).
#[must_use]
pub fn build_message_at(
    event: TelegramEvent,
    ctx: &TelegramContext<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let now = crate::time::format_date_time(now);
    let host = safe_host(ctx);
    let user = message_user(ctx);
    let user_line = format!("👤 <b>Conta:</b> <code>{}</code>", escape_html(&user));
    let body = match event {
        TelegramEvent::DryRun => format!(
            "🧪 <b>AliExpress Moedas - Teste Dry-Run</b>\n\n\
             A validação de ambiente e credenciais foi concluída com sucesso!\n\
             📅 <b>Data:</b> {now}\n\
             🖥️ <b>Host:</b> <code>{host}</code>\n\
             🔔 <b>Notificações Telegram:</b> Operacionais e ativas"
        ),
        TelegramEvent::ManualTest => format!(
            "🔔 <b>AliExpress Moedas - Teste de Notificação Telegram</b>\n\n\
             Se você está lendo esta mensagem, o bot do Telegram foi configurado com sucesso e está operando perfeitamente! 🎉\n\
             📅 <b>Data:</b> {now}\n\
             🖥️ <b>Host:</b> <code>{host}</code>"
        ),
        TelegramEvent::LockActive => {
            let details = ctx.error.unwrap_or("").trim();
            // O oráculo aplica `filter(Boolean)`: a linha vazia após o título cai.
            let mut lines = vec![
                "⚠️ <b>AliExpress Moedas - Execução Bloqueada (Lock Ativo)</b>".to_string(),
                "Outra instância da automação já está em execução no host. A execução atual foi finalizada para evitar sobreposição.".to_string(),
            ];
            if !details.is_empty() {
                lines.push(format!("ℹ️ <i>{}</i>", escape_html(details)));
            }
            if ctx.user.is_some() {
                lines.push(user_line.clone());
            }
            lines.push(format!("📅 <b>Data:</b> {now}"));
            lines.push(format!("🖥️ <b>Host:</b> <code>{host}</code>"));
            lines.join("\n")
        }
        TelegramEvent::Failure => {
            let snippet =
                sanitize_sensitive_query_params(&extract_relevant_error_message(ctx.error));
            let mut lines = vec![
                format!("🔴 ali-coins — {now}"),
                format!("⚠️ <b>Erro:</b> <code>{}</code>", escape_html(&snippet)),
            ];
            if ctx.user.is_some() {
                lines.push(user_line.clone());
            }
            if ctx.imported_session_expired {
                lines.push(String::new());
                lines.push("⚠️ <b>Aviso de Sessão Remota:</b>".to_string());
                lines.push(
                    "A sessão em uso foi importada de outro host (via <code>import_session.js</code>) e parece ter expirado ou sido invalidada pelo AliExpress."
                        .to_string(),
                );
                lines.push(
                    "💡 <i>Ação necessária:</i> É necessário gerar uma nova sessão executando <code>node export_session.js</code> no servidor de origem e importá-la neste host com <code>node import_session.js</code>."
                        .to_string(),
                );
            }
            lines.push(format!("🖥️ <b>Host:</b> <code>{host}</code>"));
            lines.join("\n")
        }
        TelegramEvent::StreakBreak => {
            let yesterday = to_safe_streak(ctx.previous_streak_days);
            let today = to_safe_streak(ctx.streak_days);
            let balance = ctx
                .total_balance
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("N/D");
            let mut lines = vec![
                format!("🚨 <b>STREAK QUEBRADO</b> — {now}"),
                String::new(),
                "⚠️ <b>Atenção:</b> A sequência diária de check-in foi interrompida ou resetada!"
                    .to_string(),
            ];
            if ctx.user.is_some() {
                lines.push(user_line.clone());
            }
            lines.push(format!("🖥️ <b>Host:</b> <code>{host}</code>"));
            lines.push(format!(
                "📉 <b>Ontem:</b> {} ➔ <b>Hoje:</b> {}",
                escape_html(&yesterday),
                escape_html(&today)
            ));
            lines.push(format!("💰 <b>Saldo Atual:</b> {}", escape_html(balance)));
            lines.join("\n")
        }
        TelegramEvent::TwoFactorRequired => {
            let mut lines = vec![
                format!("🔐 <b>AliExpress Moedas - Verificação 2FA Solicitada</b> — {now}"),
                String::new(),
            ];
            if ctx.user.is_some() {
                lines.push(user_line.clone());
            }
            lines.push(format!("🖥️ <b>Host:</b> <code>{host}</code>"));
            lines.push(String::new());
            lines.push("⚠️ <b>Execução Não-Interativa (Cron / CI):</b>".to_string());
            lines.push(
                "O AliExpress solicitou verificação 2FA (e-mail ou SMS) e a automação foi finalizada em &lt;5s para evitar travamento."
                    .to_string(),
            );
            lines.push(String::new());
            lines.push("💡 <b>Guia de Resolução (2FA no Cron):</b>".to_string());
            lines.push(
                "1. Execute localmente no seu computador: <code>./run_all.sh</code>".to_string(),
            );
            lines
                .push("2. Digite o código de 6 dígitos quando solicitado no terminal.".to_string());
            lines.push(
                "3. Exporte a nova sessão gerada: <code>node export_session.js</code>".to_string(),
            );
            lines.push("4. Importe a sessão no servidor: <code>node import_session.js &lt; session_token.txt</code>".to_string());
            lines.join("\n")
        }
        TelegramEvent::CaptchaRequired => {
            let snippet =
                sanitize_sensitive_query_params(&extract_relevant_error_message(ctx.error));
            let mut lines = vec![
                format!("🤖 ali-coins — {now}"),
                "⚠️ <b>Desafio anti-bot (captcha) detectado no login</b>".to_string(),
            ];
            if ctx.user.is_some() {
                lines.push(user_line.clone());
            }
            lines.push(format!("🖥️ <b>Host:</b> <code>{host}</code>"));
            lines.push(String::new());
            lines.push(format!("ℹ️ <i>{}</i>", escape_html(&snippet)));
            lines.push(String::new());
            lines.push(
                "💡 <b>Ação:</b> renove a sessão localmente (rede residencial) e importe com <code>node import_session.js</code>. Novas tentativas de login ficam pausadas pelo cooldown (<code>CAPTCHA_COOLDOWN_HOURS</code>)."
                    .to_string(),
            );
            lines.join("\n")
        }
        TelegramEvent::CaptchaCooldownReleased => {
            let mut lines = vec![
                format!("🤖 ali-coins — {now}"),
                "✅ <b>Cooldown pós-captcha liberado</b>".to_string(),
            ];
            if ctx.user.is_some() {
                lines.push(user_line.clone());
            }
            lines.push(format!("🖥️ <b>Host:</b> <code>{host}</code>"));
            lines.push(String::new());
            lines.push(
                "A janela de pausa expirou; a automação voltará a tentar o login normalmente."
                    .to_string(),
            );
            lines.join("\n")
        }
        TelegramEvent::Success
        | TelegramEvent::AlreadyCollected
        | TelegramEvent::Checkin
        | TelegramEvent::Tasks
        | TelegramEvent::MultiAccount => {
            let mut lines = vec![
                "🔔 <b>AliExpress Moedas - Notificação</b>".to_string(),
                String::new(),
                format!("Status: {}", event_name(event)),
                user_line,
                format!("📅 <b>Data:</b> {now}"),
                format!("🖥️ <b>Host:</b> <code>{host}</code>"),
            ];
            if ctx.user.is_none() {
                lines.remove(3);
            }
            lines.join("\n")
        }
    };
    truncate_telegram_message(&body)
}

/// Converte `streakDays` (número ou texto) para o formato aceito por `to_safe_streak`.
fn streak_value_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// Inteiro seguro do payload (número/string de dígitos; negativo → default).
fn to_safe_int(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            number
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map_or(0, |value| value.floor() as i64)
        }
        Some(Value::String(text)) => text
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|value| *value >= 0)
            .unwrap_or(0),
        _ => 0,
    }
}

/// Mensagem consolidada multi-conta (port de `buildMultiAccountMessage`).
#[must_use]
pub fn build_multi_account_message_at(
    report: &Value,
    event: TelegramEvent,
    error: Option<&str>,
    host: &str,
    version: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let (emoji, desc) = match event {
        TelegramEvent::AlreadyCollected => ("ℹ️", "Já Coletado"),
        TelegramEvent::Failure => ("🔴", "Falha"),
        TelegramEvent::CaptchaRequired => ("🤖", "Captcha Solicitado"),
        _ => ("✅", "Sucesso"),
    };
    let report_date = crate::time::format_date(now);
    let safe_host = escape_html(&format!("{host} (v{version})"));
    let meta = report.get("meta");

    let mut lines = vec![
        format!("{emoji} <b>AliExpress Moedas - Multi-Conta ({desc}) — {report_date}</b>"),
        format!(
            "📊 <b>Resumo:</b> {}/{} contas processadas com sucesso",
            to_safe_int(meta.and_then(|meta| meta.get("successfulAccounts"))),
            to_safe_int(meta.and_then(|meta| meta.get("totalAccounts")))
        ),
        String::new(),
    ];

    let accounts = report
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, account) in accounts.iter().enumerate() {
        let user = account.get("user").and_then(Value::as_str).unwrap_or("");
        let user_masked = escape_html(user);
        if let Some(error) = account.get("error").and_then(Value::as_str) {
            lines.push(format!(
                "[{}] <code>{user_masked}</code>: ❌ Falha ({})",
                index + 1,
                escape_html(&sanitize_sensitive_query_params(error))
            ));
            continue;
        }

        let checkin_value = account.get("checkin").filter(|value| !value.is_null());
        let tasks_value = account.get("tasks").filter(|value| !value.is_null());
        let checkin_input: Option<CheckinInput> =
            checkin_value.and_then(|value| serde_json::from_value(value.clone()).ok());
        let tasks_input: Option<TasksInput> =
            tasks_value.and_then(|value| serde_json::from_value(value.clone()).ok());
        let account_meta = account.get("meta");

        let checkin_coins = account_meta
            .and_then(|meta| meta.get("checkinCoinsGained"))
            .map_or_else(
                || compute_checkin_coins_gained(checkin_input.as_ref()),
                |value| to_safe_int(Some(value)),
            );
        let tasks_coins = account_meta
            .and_then(|meta| meta.get("tasksCoinsGained"))
            .map_or_else(
                || compute_tasks_coins_gained(tasks_input.as_ref(), checkin_input.as_ref()),
                |value| to_safe_int(Some(value)),
            );
        let total_coins = account_meta
            .and_then(|meta| meta.get("totalCoinsGained"))
            .map_or(checkin_coins + tasks_coins, |value| {
                to_safe_int(Some(value))
            });
        let streak = to_safe_streak(
            streak_value_text(checkin_value.and_then(|checkin| checkin.get("streakDays")))
                .as_deref(),
        );
        let balance = account_meta
            .and_then(|meta| meta.get("finalBalance"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                checkin_value
                    .and_then(|checkin| checkin.get("totalBalance"))
                    .and_then(Value::as_str)
                    .map(|total| format!("{total} moedas"))
            })
            .unwrap_or_else(|| "N/D".to_string());

        lines.push(format!(
            "[{}] <code>{user_masked}</code>: 💰 <b>{}</b> | 🪙 +{total_coins} (+{checkin_coins}/+{tasks_coins}) | Streak: {}",
            index + 1,
            escape_html(&balance),
            escape_html(&streak)
        ));
    }

    let agenda: Vec<&Value> = accounts
        .iter()
        .filter(|account| {
            account.get("startTime").is_some() || account.get("nextAccountAt").is_some()
        })
        .collect();
    if !agenda.is_empty() {
        lines.push(String::new());
        lines.push("📅 <b>Agenda:</b>".to_string());
        for (index, account) in agenda.iter().enumerate() {
            let user = escape_html(account.get("user").and_then(Value::as_str).unwrap_or(""));
            let inicio = account
                .get("startTime")
                .and_then(Value::as_str)
                .and_then(parse_iso_clock)
                .unwrap_or_else(|| "N/D".to_string());
            let fim = account
                .get("endTime")
                .and_then(Value::as_str)
                .and_then(parse_iso_clock)
                .unwrap_or_else(|| "N/D".to_string());
            let proxima = account
                .get("nextAccountAt")
                .and_then(Value::as_str)
                .and_then(parse_iso_time)
                .map_or_else(|| "última".to_string(), |time| format!("próxima: {time}"));
            lines.push(format!(
                "[{}] <code>{user}</code>: {inicio} → {fim} ({proxima})",
                index + 1
            ));
        }
    }

    lines.push(String::new());
    let mut multi_total_duration = meta
        .and_then(|meta| meta.get("totalDuration"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value != "0s")
        .map(str::to_string);
    if multi_total_duration.is_none() {
        if let (Some(start), Some(end)) = (
            meta.and_then(|meta| meta.get("startTime"))
                .and_then(Value::as_str)
                .and_then(parse_iso_millis),
            meta.and_then(|meta| meta.get("endTime"))
                .and_then(Value::as_str)
                .and_then(parse_iso_millis),
        ) {
            if end > start {
                multi_total_duration = Some(crate::time::format_duration(end - start));
            }
        }
    }
    if let Some(duration) = multi_total_duration.filter(|value| value != "0s") {
        lines.push(format!(
            "⏱️ <b>Duração Total:</b> {}",
            escape_html(&duration)
        ));
    }
    let report_flag = report
        .get("isImportedSessionExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let account_flags: Vec<bool> = report
        .get("accounts")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.get("isImportedSessionExpired")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        || item
                            .get("error")
                            .and_then(|error| error.get("isImportedSessionExpired"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    if check_imported_session_expired(&ImportedSessionCheck {
        error_message: error,
        error_flag: false,
        report_flag,
        account_flags: &account_flags,
        report_is_multi: true,
        meta_imported: false,
    }) {
        lines.push(String::new());
        lines.push("⚠️ <b>Aviso de Sessão Remota:</b>".to_string());
        lines.push(
            "Uma ou mais contas utilizam sessão importada de outro host que parece ter expirado."
                .to_string(),
        );
        lines.push(
            "💡 <i>Ação necessária:</i> Gere uma nova sessão com <code>node export_session.js</code> no servidor de origem e importe com <code>node import_session.js</code>."
                .to_string(),
        );
    }

    lines.push(format!(
        "📅 <b>Data:</b> {}",
        crate::time::format_date_time(now)
    ));
    lines.push(format!("🖥️ <b>Host:</b> <code>{safe_host}</code>"));

    truncate_telegram_message(&lines.join("\n"))
}

fn parse_iso_clock(raw: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| crate::time::format_time(parsed.with_timezone(&chrono::Utc)))
}

fn parse_iso_time(raw: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| {
            let utc = parsed.with_timezone(&chrono::Utc);
            format!(
                "{} {}",
                crate::time::format_time(utc),
                crate::time::get_report_timezone_label(utc)
            )
        })
}

fn parse_iso_millis(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| parsed.timestamp_millis())
}

/// Inteiro do payload (número, string com dígitos ou ausente → 0).
fn payload_i64(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Value::String(text)) => text
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(0),
        _ => 0,
    }
}

/// Texto não vazio do payload (com `trim`).
fn payload_text(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Sequência exibida na mensagem (número, texto ou `N/D`).
fn payload_streak(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) if !text.trim().is_empty() => text.clone(),
        _ => "N/D".to_string(),
    }
}

/// Duração do relatório com os fallbacks do oráculo.
fn payload_duration(report: &Value, meta: Option<&Value>) -> String {
    let non_zero = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|text| !text.is_empty() && *text != "0s" && *text != "N/D")
            .map(str::to_string)
    };
    let from_meta = non_zero(payload_text(
        meta.and_then(|meta| meta.get("totalDuration")),
    ));
    let from_checkin = non_zero(payload_text(
        report
            .get("checkin")
            .and_then(|checkin| checkin.get("duration")),
    ));
    let from_tasks = non_zero(payload_text(
        report.get("tasks").and_then(|tasks| tasks.get("duration")),
    ));
    if let Some(duration) = from_meta.or(from_checkin).or(from_tasks) {
        return duration;
    }
    let parse_iso = |text: &str| {
        chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|dt| dt.timestamp_millis())
    };
    let start = payload_text(meta.and_then(|meta| meta.get("startTime"))).and_then(parse_iso);
    let end = payload_text(meta.and_then(|meta| meta.get("endTime"))).and_then(parse_iso);
    match (start, end) {
        (Some(start), Some(end)) if end > start => format_duration(end - start),
        _ => "0s".to_string(),
    }
}

/// Mensagem do relatório unificado no formato do oráculo (`notify.js` seção 7).
///
/// Mesmo formato para o dia recém-coletado e para "já coletado" (emoji ℹ️),
/// com os mesmos fallbacks de saldo/duração/streak do Node.
#[must_use]
pub fn build_unified_report_message(
    report: &Value,
    host: &str,
    version: &str,
    event: TelegramEvent,
) -> String {
    let meta = report.get("meta");
    let checkin_value = report.get("checkin").filter(|value| !value.is_null());
    let tasks_value = report.get("tasks").filter(|value| !value.is_null());
    let checkin_input: Option<CheckinInput> =
        checkin_value.and_then(|value| serde_json::from_value(value.clone()).ok());
    let tasks_input: Option<TasksInput> =
        tasks_value.and_then(|value| serde_json::from_value(value.clone()).ok());

    let checkin_coins = meta
        .and_then(|meta| meta.get("checkinCoinsGained"))
        .map_or_else(
            || compute_checkin_coins_gained(checkin_input.as_ref()),
            |value| payload_i64(Some(value)),
        );
    let tasks_coins = meta
        .and_then(|meta| meta.get("tasksCoinsGained"))
        .map_or_else(
            || compute_tasks_coins_gained(tasks_input.as_ref(), checkin_input.as_ref()),
            |value| payload_i64(Some(value)),
        );
    let total_coins = meta
        .and_then(|meta| meta.get("totalCoinsGained"))
        .map_or(checkin_coins + tasks_coins, |value| {
            payload_i64(Some(value))
        });

    let already_collected = matches!(event, TelegramEvent::AlreadyCollected)
        || (checkin_input
            .as_ref()
            .and_then(|checkin| checkin.already_collected)
            .unwrap_or(false)
            && tasks_coins == 0);
    let title_emoji = if already_collected { "ℹ️" } else { "✅" };

    let raw_user = payload_text(report.get("user")).unwrap_or("");
    let user = if raw_user.is_empty() {
        "desconhecida".to_string()
    } else if raw_user.contains("***") {
        raw_user.to_string()
    } else {
        mask_user(raw_user)
    };

    let streak = payload_streak(checkin_value.and_then(|checkin| checkin.get("streakDays")));

    let from_meta = payload_text(meta.and_then(|meta| meta.get("finalBalance")));
    let saldo = match from_meta {
        Some(balance) if balance != "N/D" => balance.to_string(),
        _ => payload_text(checkin_value.and_then(|checkin| checkin.get("totalBalance")))
            .map_or_else(|| "N/D".to_string(), |total| format!("{total} moedas")),
    };
    let saldo = if saldo != "N/D" && !saldo.contains("moedas") {
        format!("{saldo} moedas")
    } else {
        saldo
    };

    let duration = payload_duration(report, meta);
    let host_line = escape_html(&format!("{host} (v{version})"));
    let date = format_date(chrono::Utc::now());
    let body = format!(
        "{title_emoji} ali-coins — {date}\n\
         👤 <b>Conta:</b> <code>{}</code>\n\
         🖥️ <b>Host:</b> <code>{host_line}</code>\n\
         🪙 Ganhas hoje: +{total_coins} moedas (check-in +{checkin_coins} / tarefas +{tasks_coins})\n\
         📅 Sequência: {} dias\n\
         💰 Saldo: {}\n\
         ⏱️ Duração: {duration}",
        escape_html(&user),
        escape_html(&streak),
        escape_html(&saldo),
    );
    truncate_telegram_message(&body)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn backoff_delay(attempt: u32) -> Duration {
    let base = (RETRY_BASE_MS * 2_u64.pow(attempt.min(4))).min(RETRY_MAX_MS);
    let jitter = 0.8 + 0.4 * rand::rng().random::<f64>();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Duration::from_millis((base as f64 * jitter).round() as u64)
}

fn parse_retry_after(body: &str) -> Option<Duration> {
    let value: Value = serde_json::from_str(body).ok()?;
    let seconds = value
        .get("parameters")
        .and_then(|parameters| parameters.get("retry_after"))
        .and_then(Value::as_f64)?;
    if seconds <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let duration = Duration::from_millis((seconds * 1000.0).round() as u64);
    Some(duration.min(RETRY_AFTER_MAX))
}

fn strip_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside_tag = false;
    for character in text.chars() {
        match character {
            '<' => inside_tag = true,
            '>' if inside_tag => inside_tag = false,
            _ if !inside_tag => out.push(character),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
}

/// Envia a mensagem com a política de retry do oráculo (nunca lança).
pub async fn send_telegram(
    client: &SafeHttpClient,
    config: &TelegramConfig,
    text: &str,
) -> TelegramSendResult {
    if !config.enabled || config.bot_token.trim().is_empty() || config.chat_id.trim().is_empty() {
        return TelegramSendResult {
            ok: false,
            skipped: true,
            status: None,
            error: None,
        };
    }
    let api_base = if config.api_base.trim().is_empty() {
        "https://api.telegram.org"
    } else {
        config.api_base.trim_end_matches('/')
    };
    let url = format!("{api_base}/bot{}/sendMessage", config.bot_token);

    for attempt in 0..RETRY_ATTEMPTS {
        let mut body = Map::new();
        body.insert("chat_id".to_string(), Value::String(config.chat_id.clone()));
        body.insert(
            "text".to_string(),
            Value::String(truncate_telegram_message(text)),
        );
        body.insert("parse_mode".to_string(), Value::String("HTML".to_string()));
        if config.silent {
            body.insert("disable_notification".to_string(), Value::Bool(true));
        }
        let body = Value::Object(body);

        match client.post_json(&url, &body).await {
            Ok(response) if (200..300).contains(&response.status) => {
                return TelegramSendResult {
                    ok: true,
                    skipped: false,
                    status: Some(response.status),
                    error: None,
                };
            }
            Ok(response) if response.status == 429 || response.status >= 500 => {
                if attempt + 1 < RETRY_ATTEMPTS {
                    let delay =
                        parse_retry_after(&response.body).unwrap_or_else(|| backoff_delay(attempt));
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return TelegramSendResult {
                    ok: false,
                    skipped: false,
                    status: Some(response.status),
                    error: Some(format!("status {}", response.status)),
                };
            }
            Ok(response) if response.status == 400 => {
                // Fallback do oráculo: remove tags e reenvia sem parse_mode.
                let plain = strip_html_tags(&truncate_telegram_message(text));
                let mut fallback = Map::new();
                fallback.insert("chat_id".to_string(), Value::String(config.chat_id.clone()));
                fallback.insert("text".to_string(), Value::String(plain));
                match client.post_json(&url, &Value::Object(fallback)).await {
                    Ok(resp) if (200..300).contains(&resp.status) => {
                        return TelegramSendResult {
                            ok: true,
                            skipped: false,
                            status: Some(resp.status),
                            error: None,
                        };
                    }
                    _ => {
                        return TelegramSendResult {
                            ok: false,
                            skipped: false,
                            status: Some(400),
                            error: Some("400 (fallback sem parse_mode falhou)".to_string()),
                        };
                    }
                }
            }
            Ok(response) => {
                return TelegramSendResult {
                    ok: false,
                    skipped: false,
                    status: Some(response.status),
                    error: Some(format!("status {}", response.status)),
                };
            }
            Err(err) if err.is_timeout() => {
                // Timeout é ambíguo: NUNCA retenta para não duplicar mensagem.
                return TelegramSendResult {
                    ok: false,
                    skipped: false,
                    status: None,
                    error: Some(err.to_string()),
                };
            }
            Err(err) => {
                if attempt + 1 < RETRY_ATTEMPTS {
                    tokio::time::sleep(backoff_delay(attempt)).await;
                    continue;
                }
                return TelegramSendResult {
                    ok: false,
                    skipped: false,
                    status: None,
                    error: Some(err.to_string()),
                };
            }
        }
    }

    TelegramSendResult {
        ok: false,
        skipped: false,
        status: None,
        error: Some("tentativas esgotadas".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapa_html() {
        assert_eq!(
            escape_html("<b>a & b \"c\"</b>"),
            "&lt;b&gt;a &amp; b &quot;c&quot;&lt;/b&gt;"
        );
    }

    #[test]
    fn trunca_sem_quebrar_surrogate() {
        let text = "😀".repeat(1500); // 3000 unidades UTF-16 < 4096
        assert_eq!(truncate_telegram_message(&text), text);

        let long = "😀".repeat(3000); // 6000 unidades UTF-16
        let truncated = truncate_telegram_message(&long);
        assert!(truncated.encode_utf16().count() <= TELEGRAM_MAX_UTF16);
        assert!(truncated.contains("mensagem truncada"));
        // Não termina com metade de um par surrogate.
        for character in truncated.chars() {
            assert_ne!(character, char::REPLACEMENT_CHARACTER);
        }
    }

    #[test]
    fn mensagens_por_evento_no_formato_do_oraculo() {
        let ctx = TelegramContext {
            user: Some("fulano@example.com"),
            total_balance: Some("150 moedas"),
            streak_days: Some("1"),
            previous_streak_days: Some("50"),
            duration: Some("1m 20s"),
            error: Some("timeout"),
            host: Some("vps-1"),
            version: Some("0.1.0"),
            coins_gained: Some(35),
            imported_session_expired: false,
        };
        let success = build_message(TelegramEvent::Success, &ctx);
        assert!(success.contains("🔔 <b>AliExpress Moedas - Notificação</b>"));
        assert!(success.contains("Status: success"));
        let failure = build_message(TelegramEvent::Failure, &ctx);
        assert!(failure.contains("🔴 ali-coins —"));
        assert!(failure.contains("⚠️ <b>Erro:</b> <code>timeout</code>"));
        let streak = build_message(TelegramEvent::StreakBreak, &ctx);
        assert!(streak.contains("🚨 <b>STREAK QUEBRADO</b>"));
        assert!(streak.contains("📉 <b>Ontem:</b> 50 dias ➔ <b>Hoje:</b> 1 dias"));
        let two_factor = build_message(TelegramEvent::TwoFactorRequired, &ctx);
        assert!(two_factor.contains("Verificação 2FA Solicitada"));
        assert!(two_factor.contains("&lt;5s"));
        let lock = build_message(TelegramEvent::LockActive, &ctx);
        assert!(lock.contains("Execução Bloqueada (Lock Ativo)"));
        let captcha = build_message(TelegramEvent::CaptchaRequired, &ctx);
        assert!(captcha.contains("Desafio anti-bot (captcha) detectado no login"));
        let dry_run = build_message(TelegramEvent::DryRun, &ctx);
        assert!(dry_run.contains("🧪 <b>AliExpress Moedas - Teste Dry-Run</b>"));
        assert!(dry_run.contains("🔔 <b>Notificações Telegram:</b> Operacionais e ativas"));
    }

    #[test]
    fn sanitiza_segredos_em_erros() {
        let text = "GET https://x/y?token=abc123&session=zzz\nAuthorization: Bearer abc.def-ghi\nCookie: a=1; b=2";
        let sanitized = sanitize_sensitive_query_params(text);
        assert!(sanitized.contains("token=[REDACTED]"));
        assert!(sanitized.contains("session=[REDACTED]"));
        // A linha inteira de header é redigida (cookies multivalorados inclusos).
        assert!(sanitized.contains("Authorization: [REDACTED]"));
        assert!(sanitized.contains("Cookie: [REDACTED]"));
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("abc.def-ghi"));
        assert!(!sanitized.contains("a=1"));
    }

    #[test]
    fn retry_after_do_corpo() {
        let body = r#"{"ok":false,"error_code":429,"parameters":{"retry_after":2.5}}"#;
        assert_eq!(parse_retry_after(body), Some(Duration::from_millis(2500)));
        assert_eq!(parse_retry_after("{}"), None);
    }

    #[test]
    fn strip_tags_e_entidades() {
        assert_eq!(strip_html_tags("<b>a &amp; b</b>"), "a & b");
    }

    #[test]
    fn mensagem_unificada_no_formato_do_oraculo() {
        let report = serde_json::json!({
            "type": "unified_report",
            "user": "agiler@example.com",
            "checkin": {
                "alreadyCollected": false,
                "streakDays": 225,
                "totalBalance": "2980"
            },
            "tasks": { "duration": "7m 00s" },
            "meta": {
                "checkinCoinsGained": 40,
                "tasksCoinsGained": 56,
                "totalCoinsGained": 96,
                "finalBalance": "2980 moedas",
                "totalDuration": "8m 53s"
            }
        });
        let message =
            build_unified_report_message(&report, "oracle-vm", "1.7.1", TelegramEvent::Success);
        let expected = format!(
            "✅ ali-coins — {}\n\
             👤 <b>Conta:</b> <code>ag***@example.com</code>\n\
             🖥️ <b>Host:</b> <code>oracle-vm (v1.7.1)</code>\n\
             🪙 Ganhas hoje: +96 moedas (check-in +40 / tarefas +56)\n\
             📅 Sequência: 225 dias\n\
             💰 Saldo: 2980 moedas\n\
             ⏱️ Duração: 8m 53s",
            crate::time::format_date(chrono::Utc::now())
        );
        assert_eq!(message, expected);
    }

    #[test]
    fn mensagem_unificada_ja_coletado_mantem_formato() {
        let report = serde_json::json!({
            "type": "unified_report",
            "user": "edelanoali@gmail.com",
            "checkin": {
                "alreadyCollected": true,
                "streakDays": 1,
                "totalBalance": "N/D"
            },
            "tasks": null,
            "meta": {
                "checkinCoinsGained": 0,
                "tasksCoinsGained": 0,
                "totalCoinsGained": 0,
                "finalBalance": "N/D",
                "totalDuration": "1m 05s"
            }
        });
        let message = build_unified_report_message(
            &report,
            "vm-ali-rust",
            "0.1.0",
            TelegramEvent::AlreadyCollected,
        );
        assert!(message.starts_with("ℹ️ ali-coins — "));
        assert!(message.contains("👤 <b>Conta:</b> <code>ed***@gmail.com</code>"));
        assert!(message.contains("🖥️ <b>Host:</b> <code>vm-ali-rust (v0.1.0)</code>"));
        assert!(message.contains("🪙 Ganhas hoje: +0 moedas (check-in +0 / tarefas +0)"));
        assert!(message.contains("📅 Sequência: 1 dias"));
        assert!(message.contains("💰 Saldo: N/D"));
        assert!(message.contains("⏱️ Duração: 1m 05s"));
    }

    #[test]
    fn mensagem_unificada_sem_meta_usa_fallbacks() {
        // Payload mínimo: sem meta, com saldo no check-in e durações nas etapas.
        let report = serde_json::json!({
            "type": "unified_report",
            "user": "fulano@example.com",
            "checkin": {
                "alreadyCollected": false,
                "streakDays": 2,
                "totalBalance": "1234",
                "duration": "1m 20s"
            },
            "tasks": null
        });
        let message =
            build_unified_report_message(&report, "vps-1", "0.2.0", TelegramEvent::Success);
        assert!(message.contains("👤 <b>Conta:</b> <code>fu***@example.com</code>"));
        assert!(message.contains("💰 Saldo: 1234 moedas"));
        assert!(message.contains("⏱️ Duração: 1m 20s"));
        // Check-in com 2 dias → 15 moedas pelo oráculo.
        assert!(message.contains("🪙 Ganhas hoje: +15 moedas (check-in +15 / tarefas +0)"));
    }

    fn check(
        message: Option<&str>,
        error_flag: bool,
        meta_imported: bool,
    ) -> ImportedSessionCheck<'_> {
        ImportedSessionCheck {
            error_message: message,
            error_flag,
            report_flag: false,
            account_flags: &[],
            report_is_multi: false,
            meta_imported,
        }
    }

    #[test]
    fn sessao_importada_por_flags_e_regex() {
        // Flag estruturada no erro.
        assert!(check_imported_session_expired(&ImportedSessionCheck {
            error_flag: true,
            ..check(None, false, false)
        }));
        // Flag no relatório.
        assert!(check_imported_session_expired(&ImportedSessionCheck {
            report_flag: true,
            ..check(None, false, false)
        }));
        // Flag em qualquer conta.
        assert!(check_imported_session_expired(&ImportedSessionCheck {
            account_flags: &[false, true, false],
            ..check(None, false, false)
        }));
        // `/sessão.*(importada|remota).*expir/i` (case-insensitive).
        assert!(check_imported_session_expired(&check(
            Some("A SESSÃO IMPORTADA de outro host EXPIROU"),
            false,
            false
        )));
        assert!(check_imported_session_expired(&check(
            Some("Sessão remota expirou"),
            false,
            false
        )));
        // `/node export_session\.js/i`.
        assert!(check_imported_session_expired(&check(
            Some("Gere nova sessão com node export_session.js"),
            false,
            false
        )));
        // Negativos.
        assert!(!check_imported_session_expired(&check(
            Some("Request timed out"),
            false,
            false
        )));
        assert!(!check_imported_session_expired(&check(
            Some("A sessão expirou"),
            false,
            false
        )));
    }

    #[test]
    fn sessao_importada_fallback_pelo_meta() {
        // Meta importado + erro de autenticação → alerta (conta única).
        assert!(check_imported_session_expired(&check(
            Some("Erro ao efetuar o login: não foi possível obter streak e saldo"),
            false,
            true
        )));
        // Sem meta importado → não alerta.
        assert!(!check_imported_session_expired(&check(
            Some("Erro ao efetuar o login"),
            false,
            false
        )));
        // Multi-conta desliga o fallback pelo meta (ordem do oráculo).
        assert!(!check_imported_session_expired(&ImportedSessionCheck {
            report_is_multi: true,
            ..check(Some("Erro ao efetuar o login"), false, true)
        }));
        // Erro não relacionado a autenticação → não alerta mesmo com meta.
        assert!(!check_imported_session_expired(&check(
            Some("Request timed out"),
            false,
            true
        )));
    }

    #[test]
    fn produtor_marca_sessao_importada_em_erros_de_navegacao() {
        // Padrão dos produtores inclui cookie/navigat além do fallback do notify.
        assert!(imported_session_error_flag(
            Some("Navigation timeout"),
            true
        ));
        assert!(imported_session_error_flag(Some("Cookie inválido"), true));
        assert!(imported_session_error_flag(Some("Erro de login"), true));
        assert!(!imported_session_error_flag(Some("Erro de login"), false));
        assert!(!imported_session_error_flag(Some("timeout"), true));

        assert!(detect_imported_session_expired(
            Some("Erro ao efetuar o login"),
            true
        ));
        assert!(!detect_imported_session_expired(
            Some("Erro ao efetuar o login"),
            false
        ));
    }

    #[test]
    fn failure_inclui_aviso_de_sessao_remota() {
        let base = TelegramContext {
            user: Some("fulano@example.com"),
            error: Some("Sessão expirou ou exige login."),
            host: Some("vps-1"),
            version: Some("0.1.0"),
            ..TelegramContext::default()
        };
        let sem_aviso = build_message(TelegramEvent::Failure, &base);
        assert!(!sem_aviso.contains("Aviso de Sessão Remota"));

        let com_aviso = build_message(
            TelegramEvent::Failure,
            &TelegramContext {
                imported_session_expired: true,
                ..base
            },
        );
        assert!(com_aviso.contains("⚠️ <b>Aviso de Sessão Remota:</b>"));
        assert!(com_aviso.contains(
            "A sessão em uso foi importada de outro host (via <code>import_session.js</code>)"
        ));
        assert!(com_aviso.contains("<code>node export_session.js</code>"));
    }

    #[test]
    fn trunca_em_entidade_sem_ponto_e_virgula() {
        let texto = format!("{}&amp\n{}", "x".repeat(2500), "y".repeat(2000));
        let saida = truncate_telegram_message(&texto);
        assert!(saida.ends_with("… (mensagem truncada)"));
        assert!(
            !saida
                .trim_end_matches("… (mensagem truncada)")
                .ends_with('&'),
            "entidade incompleta deve ser removida: {saida}"
        );
    }

    #[test]
    fn extrai_erro_relevante_cobre_ramificacoes() {
        const UNKNOWN: &str = "Erro desconhecido durante o processamento.";
        assert_eq!(extract_relevant_error_message(None), UNKNOWN);
        assert_eq!(extract_relevant_error_message(Some("   ")), UNKNOWN);
        let logs = "Falha ao abrir a página\nSegunda linha útil\n===== logs =====\nruído interno";
        assert_eq!(
            extract_relevant_error_message(Some(logs)),
            "Falha ao abrir a página\nSegunda linha útil"
        );
        let call = "Timeout ao carregar\nCall log:\n - waiting for selector";
        assert_eq!(
            extract_relevant_error_message(Some(call)),
            "Timeout ao carregar"
        );
        let multi = "linha neutra\n    at foo (bar:1)\n  - [pid=123] nada\nerror: algo falhou\nfatal: crashou\noutra linha\nmais uma";
        let extraido = extract_relevant_error_message(Some(multi));
        assert!(extraido.contains("error: algo falhou"));
        assert!(extraido.contains("fatal: crashou"));
        assert!(!extraido.contains("at foo"));
        // Nenhuma linha com indicador: usa as 3 primeiras relevantes.
        assert_eq!(
            extract_relevant_error_message(Some("a\nb\nc\nd")),
            "a\nb\nc"
        );
        // Todas irrelevantes: devolve as 3 primeiras linhas cruas.
        assert_eq!(
            extract_relevant_error_message(Some("at a\nat b\nat c\nat d")),
            "at a\nat b\nat c"
        );
    }

    #[test]
    fn streak_segura_e_nomes_de_evento() {
        assert_eq!(to_safe_streak(Some("5")), "5 dias");
        assert_eq!(to_safe_streak(Some("0")), "N/D");
        assert_eq!(to_safe_streak(Some("abc")), "N/D");
        assert_eq!(to_safe_streak(None), "N/D");
        assert_eq!(event_name(TelegramEvent::DryRun), "dry_run");
        assert_eq!(event_name(TelegramEvent::ManualTest), "manual_test");
    }
}
