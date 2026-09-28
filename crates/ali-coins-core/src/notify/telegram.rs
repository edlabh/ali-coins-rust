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

fn footer(ctx: &TelegramContext<'_>) -> String {
    match (ctx.host, ctx.version) {
        (Some(host), Some(version)) => format!("\n🌐 {host} • v{version}"),
        (Some(host), None) => format!("\n🌐 {host}"),
        _ => String::new(),
    }
}

/// Monta a mensagem HTML do evento.
#[must_use]
pub fn build_message(event: TelegramEvent, ctx: &TelegramContext<'_>) -> String {
    let user = ctx.user.unwrap_or("conta");
    let body = match event {
        TelegramEvent::DryRun => format!(
            "🧪 <b>ali-coins — Dry-run</b>\nConfiguração validada para <code>{}</code> sem abrir o navegador.",
            escape_html(user)
        ),
        TelegramEvent::ManualTest => "🔔 <b>ali-coins — Teste de notificação</b>\nO envio pelo Telegram está configurado corretamente.".to_string(),
        TelegramEvent::Success => {
            let mut lines = vec![format!(
                "✅ <b>ali-coins — Execução concluída</b>\nConta: <code>{}</code>",
                escape_html(user)
            )];
            if let Some(balance) = ctx.total_balance {
                lines.push(format!("💰 Saldo: {balance}"));
            }
            if let Some(coins) = ctx.coins_gained {
                lines.push(format!("🪙 Moedas ganhas: +{coins}"));
            }
            if let Some(streak) = ctx.streak_days {
                lines.push(format!("🔥 Streak: {streak} dias"));
            }
            if let Some(duration) = ctx.duration {
                lines.push(format!("⏱️ Duração: {duration}"));
            }
            lines.join("\n")
        }
        TelegramEvent::AlreadyCollected => format!(
            "ℹ️ <b>ali-coins — Já coletado hoje</b>\nConta: <code>{}</code>{}",
            escape_html(user),
            ctx.streak_days
                .map(|streak| format!("\n🔥 Streak: {streak} dias"))
                .unwrap_or_default()
        ),
        TelegramEvent::Failure => format!(
            "🔴 <b>ali-coins — Falha na execução</b>\nConta: <code>{}</code>\n<b>Erro:</b> {}",
            escape_html(user),
            escape_html(ctx.error.unwrap_or("erro desconhecido"))
        ),
        TelegramEvent::LockActive => format!(
            "🔒 <b>ali-coins — Execução já em andamento</b>\nOutra instância mantém o lock para <code>{}</code>.",
            escape_html(user)
        ),
        TelegramEvent::StreakBreak => format!(
            "⚠️ <b>ali-coins — Streak quebrado</b>\nConta: <code>{}</code>\nOntem: {} → Hoje: {}{}",
            escape_html(user),
            ctx.previous_streak_days.unwrap_or("N/D"),
            ctx.streak_days.unwrap_or("N/D"),
            ctx.total_balance
                .map(|balance| format!("\n💰 Saldo: {balance}"))
                .unwrap_or_default()
        ),
        TelegramEvent::TwoFactorRequired => format!(
            "🔐 <b>ali-coins — Verificação 2FA necessária</b>\nConta: <code>{}</code>\nRenove a sessão localmente com <code>node export_session.js</code> e importe com <code>node import_session.js</code>.",
            escape_html(user)
        ),
        TelegramEvent::CaptchaRequired => format!(
            "🤖 <b>ali-coins — Captcha solicitado pelo AliExpress</b>\nConta: <code>{}</code>\nNovas tentativas ficam pausadas pelo cooldown.",
            escape_html(user)
        ),
        TelegramEvent::CaptchaCooldownReleased => format!(
            "✅ <b>ali-coins — Cooldown pós-captcha liberado</b>\nConta: <code>{}</code> pode tentar login novamente.",
            escape_html(user)
        ),
        TelegramEvent::Checkin => format!(
            "✅ <b>ali-coins — Check-in diário</b>\nConta: <code>{}</code>{}",
            escape_html(user),
            ctx.total_balance
                .map(|balance| format!("\n💰 Saldo: {balance}"))
                .unwrap_or_default()
        ),
        TelegramEvent::Tasks => format!(
            "✅ <b>ali-coins — Tarefas diárias</b>\nConta: <code>{}</code>{}",
            escape_html(user),
            ctx.coins_gained
                .map(|coins| format!("\n🪙 Ganho: +{coins}"))
                .unwrap_or_default()
        ),
        TelegramEvent::MultiAccount => format!(
            "✅ <b>ali-coins — Multi-conta</b>\nContas processadas: <code>{}</code>{}",
            escape_html(user),
            ctx.total_balance
                .map(|balance| format!("\n💰 Saldo final: {balance}"))
                .unwrap_or_default()
        ),
    };
    truncate_telegram_message(&format!("{body}{}", footer(ctx)))
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
    fn mensagens_por_evento() {
        let ctx = TelegramContext {
            user: Some("fulano@example.com"),
            total_balance: Some("150"),
            streak_days: Some("42"),
            previous_streak_days: Some("50"),
            duration: Some("1m 20s"),
            error: Some("timeout"),
            host: Some("vps-1"),
            version: Some("0.1.0"),
            coins_gained: Some(35),
        };
        assert!(build_message(TelegramEvent::Success, &ctx).contains("Execução concluída"));
        assert!(build_message(TelegramEvent::Failure, &ctx).contains("timeout"));
        assert!(build_message(TelegramEvent::StreakBreak, &ctx).contains("50 → Hoje: 42"));
        assert!(build_message(TelegramEvent::TwoFactorRequired, &ctx).contains("2FA"));
        assert!(build_message(TelegramEvent::Success, &ctx).contains("vps-1 • v0.1.0"));
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
}
