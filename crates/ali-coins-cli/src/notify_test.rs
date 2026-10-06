//! Subcomando `notify-test`: envia uma mensagem de teste pelo Telegram.

use crate::context::bootstrap;
use ali_coins_core::notify::{
    SafeHttpClient, TelegramConfig, TelegramContext, TelegramEvent, build_message, send_telegram,
};
use ali_coins_core::{exit::ExitCode, logging};
use std::process::ExitCode as StdExitCode;
use std::time::Duration;

/// `ali-coins notify-test`
pub fn run(_args: &[String]) -> StdExitCode {
    let Some(ctx) = bootstrap() else {
        return StdExitCode::from(1);
    };
    run_with_context(ctx)
}

/// Núcleo do subcomando (contexto injetável nos testes).
pub(crate) fn run_with_context(ctx: crate::context::CliContext) -> StdExitCode {
    let crate::context::CliContext {
        config, accounts, ..
    } = ctx;
    if !config.telegram_enabled {
        logging::global().error("TELEGRAM_ENABLED não está ativo no credentials.env.", &[]);
        return StdExitCode::from(u8::try_from(ExitCode::Failure.as_i32()).unwrap_or(1));
    }
    let account = &accounts[0];
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Falha ao iniciar o runtime tokio: {error}");
            return StdExitCode::from(1);
        }
    };

    runtime
        .block_on(async {
            let client = SafeHttpClient::new(
                config.allow_private_webhooks,
                Duration::from_millis(config.telegram_timeout_ms),
            )
            .map_err(|error| error.to_string())?;
            let chat_id = account
                .telegram_chat_id
                .clone()
                .unwrap_or_else(|| config.telegram_chat_id.clone());
            let telegram_config = TelegramConfig {
                enabled: true,
                bot_token: config.telegram_bot_token.clone(),
                chat_id,
                silent: config.telegram_silent,
                timeout_ms: config.telegram_timeout_ms,
                api_base: String::new(),
            };
            let host = notify_host(&config);
            let context = TelegramContext {
                user: Some(account.masked_user.as_str()),
                host: Some(host.as_str()),
                version: Some(env!("CARGO_PKG_VERSION")),
                ..TelegramContext::default()
            };
            let message = build_message(TelegramEvent::ManualTest, &context);
            let result = send_telegram(&client, &telegram_config, &message).await;
            if result.ok {
                logging::global().info("Mensagem de teste enviada pelo Telegram.", &[]);
                Ok(())
            } else {
                Err(result
                    .error
                    .unwrap_or_else(|| "erro desconhecido no envio".to_string()))
            }
        })
        .map_or_else(
            |error: String| {
                logging::global().error(&format!("Falha no teste de Telegram: {error}"), &[]);
                StdExitCode::from(u8::try_from(ExitCode::Failure.as_i32()).unwrap_or(1))
            },
            |()| StdExitCode::from(u8::try_from(ExitCode::Success.as_i32()).unwrap_or(0)),
        )
}

/// Rótulo de host exibido nas notificações (`NOTIFY_HOST_LABEL` > hostname).
fn notify_host(config: &ali_coins_core::config::Config) -> String {
    if config.notify_host_label.trim().is_empty() {
        ali_coins_core::lock::hostname()
    } else {
        config.notify_host_label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_core::config::EnvSource;

    #[test]
    fn telegram_desativado_retorna_1() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::context_from(
            dir.path().to_path_buf(),
            EnvSource::from_pairs([
                ("ALI_USER", "user@example.com"),
                ("ALI_PASSWORD", "senha"),
                ("SESSION_SECRET", "0123456789abcdef0123456789abcdef"),
            ]),
        )
        .expect("contexto");
        let code = run_with_context(ctx);
        assert_eq!(code, StdExitCode::from(1));
    }
}
