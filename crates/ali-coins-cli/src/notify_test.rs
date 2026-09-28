//! Subcomando `notify-test`: envia uma mensagem de teste pelo Telegram.

use crate::export_import::bootstrap;
use ali_coins_core::notify::{
    SafeHttpClient, TelegramConfig, TelegramContext, TelegramEvent, build_message, send_telegram,
};
use ali_coins_core::{exit::ExitCode, logging};
use std::process::ExitCode as StdExitCode;
use std::time::Duration;

/// `ali-coins notify-test`
pub fn run(_args: &[String]) -> StdExitCode {
    let Some((_base_dir, _env, config, accounts)) = bootstrap() else {
        return StdExitCode::from(1);
    };
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
            let host = ali_coins_core::lock::hostname();
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
