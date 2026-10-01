//! Paridade byte-a-byte das mensagens do Telegram contra o oráculo.
//!
//! Fixture: `tools/parity/fixtures/common/notify.json`
//! (gere com `./tools/parity/generate-fixtures.sh`).

use ali_coins_core::notify::telegram::{
    ImportedSessionCheck, TelegramContext, TelegramEvent, build_message_at,
    build_multi_account_message_at, check_imported_session_expired,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;

fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/parity/fixtures/common/notify.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "fixture {} ausente ({err}); rode ./tools/parity/generate-fixtures.sh",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("fixture JSON")
}

fn event_from(name: &str) -> TelegramEvent {
    match name {
        "dry_run" => TelegramEvent::DryRun,
        "manual_test" => TelegramEvent::ManualTest,
        "success" => TelegramEvent::Success,
        "already_collected" => TelegramEvent::AlreadyCollected,
        "failure" => TelegramEvent::Failure,
        "lock_active" => TelegramEvent::LockActive,
        "streak_break" => TelegramEvent::StreakBreak,
        "2fa_required" => TelegramEvent::TwoFactorRequired,
        "captcha_required" => TelegramEvent::CaptchaRequired,
        "captcha_cooldown_released" => TelegramEvent::CaptchaCooldownReleased,
        other => panic!("evento sem mapeamento no teste: {other}"),
    }
}

#[test]
fn mensagens_do_telegram_iguais_ao_oraculo() {
    let fixture = fixture();
    let now: DateTime<Utc> = fixture["now"]
        .as_str()
        .expect("now da fixture")
        .parse()
        .expect("now ISO");
    let host = fixture["host"].as_str().expect("host").to_string();
    let version = fixture["version"].as_str().expect("version").to_string();
    let cases = fixture["cases"].as_array().expect("casos");
    assert!(!cases.is_empty(), "fixture sem casos");

    for case in cases {
        let name = case["name"].as_str().expect("nome");
        let event = event_from(case["event"].as_str().expect("evento"));
        let user = case["user"].as_str();
        // O erro pode ser string (mensagem) ou objeto com flag estruturada,
        // como nos produtores do oráculo (`err.isImportedSessionExpired`).
        let (error, error_flag) = match case.get("error") {
            Some(Value::String(text)) => (Some(text.as_str()), false),
            Some(Value::Object(object)) => (
                object.get("message").and_then(Value::as_str),
                object
                    .get("isImportedSessionExpired")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            _ => (None, false),
        };
        let previous = case
            .get("previousStreakDays")
            .and_then(Value::as_i64)
            .map(|value| value.to_string());
        let streak = case
            .get("streakDays")
            .and_then(Value::as_i64)
            .map(|value| value.to_string());
        let balance = case.get("totalBalance").and_then(Value::as_str);
        let expected = case["message"].as_str().expect("mensagem");

        // Casos multi-conta: o payload vem pronto na fixture.
        let produced = if let Some(report) = case.get("report").filter(|value| !value.is_null()) {
            build_multi_account_message_at(report, event, error, &host, &version, now)
        } else {
            let report_flag = case
                .get("report")
                .and_then(|report| report.get("isImportedSessionExpired"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let meta_imported = case
                .get("metaImported")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let imported = check_imported_session_expired(&ImportedSessionCheck {
                error_message: error,
                error_flag,
                report_flag,
                account_flags: &[],
                report_is_multi: false,
                meta_imported,
            });
            let context = TelegramContext {
                user,
                total_balance: balance,
                streak_days: streak.as_deref(),
                previous_streak_days: previous.as_deref(),
                error,
                host: Some(host.as_str()),
                version: Some(version.as_str()),
                imported_session_expired: imported,
                ..TelegramContext::default()
            };
            build_message_at(event, &context, now)
        };
        assert_eq!(
            produced, expected,
            "mensagem divergente do oráculo no caso '{name}'"
        );
    }
}
