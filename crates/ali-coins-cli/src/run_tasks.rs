//! Subcomando `tasks`: executa o painel "Ganhe mais moedas" (runner conservador).

use crate::export_import::bootstrap;
use ali_coins_browser::cdp::CdpDriver;
use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
use ali_coins_browser::launch::{ChromiumArgsInput, build_chromium_args};
use ali_coins_core::lock::{LockError, LockOptions, acquire};
use ali_coins_core::notify::{
    SafeHttpClient, TelegramConfig, TelegramEvent, build_unified_report_message, send_telegram,
};
use ali_coins_core::report::{NumOrText, TasksInput, UnifiedMeta, build_unified_report_payload};
use ali_coins_core::session::{SessionOptions, load_session_files, save_session, validate_session};
use ali_coins_core::{exit::ExitCode, logging};
use ali_coins_flows::login::has_auth_cookies;
use ali_coins_flows::tasks_runner::{TasksOptions, run_tasks};
use std::process::ExitCode as StdExitCode;
use std::time::{Duration, Instant};

/// Perfil de browser novo por execução (o oráculo usa contexto descartável).
fn fresh_profile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ali-coins-profile-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

/// Rótulo de host exibido nas notificações (`NOTIFY_HOST_LABEL` > hostname).
fn notify_host(config: &ali_coins_core::config::Config) -> String {
    if config.notify_host_label.trim().is_empty() {
        ali_coins_core::lock::hostname()
    } else {
        config.notify_host_label.clone()
    }
}

/// `ali-coins tasks [--account <id>] [--json] [--force]`
pub fn run(args: &[String]) -> StdExitCode {
    let json = has_flag(args, "--json");
    let Some((base_dir, env, config, accounts)) = bootstrap() else {
        return StdExitCode::from(1);
    };
    let account = &accounts[0];

    let lock_options = LockOptions {
        path: account.lock_path.clone(),
        force: has_flag(args, "--force") || has_flag(args, "-f"),
        stale_timeout_ms: Some(config.lock_stale_timeout_ms),
        refresh_interval_ms: None,
    };
    let _guard = match acquire(&lock_options) {
        Ok(guard) => guard,
        Err(LockError::Active { message, .. }) => {
            logging::global().warn(&message, &[]);
            return StdExitCode::from(3);
        }
        Err(error) => {
            logging::global().error(&format!("Falha ao adquirir o lock: {error}"), &[]);
            return StdExitCode::from(1);
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Falha ao iniciar o runtime tokio: {error}");
            return StdExitCode::from(1);
        }
    };

    runtime
        .block_on(async {
            let session_options = SessionOptions {
                base_dir: Some(base_dir.clone()),
                session_path: Some(account.session_path.clone()),
                encrypt_local_session: Some(config.encrypt_local_session),
                ..SessionOptions::default()
            };
            let loaded = load_session_files(&session_options, &base_dir, &env)
                .map_err(|error| format!("Falha ao carregar a sessão: {error}"))?;
            let Some(session_data) = loaded.session_data else {
                return Err("Nenhuma sessão em disco; rode `ali-coins checkin` antes.".to_string());
            };
            let validation = validate_session(
                &session_data,
                loaded.meta_data.as_ref(),
                Some(&account.user),
            );
            if !validation.valid {
                return Err(format!(
                    "Sessão inválida ({}); rode `ali-coins checkin` antes.",
                    validation.reason.unwrap_or_default()
                ));
            }

            let args = build_chromium_args(&ChromiumArgsInput {
                env: &env,
                is_root: false,
                dev_shm_small: true,
                force_no_sandbox: false,
                low_memory: None,
            });
            let driver = CdpDriver::new();
            let launch_options = LaunchOptions {
                headless: config.headless,
                args,
                executable_path: std::env::var("ALI_COINS_CHROME")
                    .ok()
                    .map(std::path::PathBuf::from),
                user_data_dir: Some(fresh_profile_dir()),
                env: Vec::new(),
            };
            let browser = driver
                .launch(&launch_options)
                .await
                .map_err(|error| error.to_string())?;
            let page = browser
                .new_page()
                .await
                .map_err(|error| error.to_string())?;
            page.enable_resource_blocking(config.allow_media)
                .await
                .map_err(|error| error.to_string())?;
            // A gaveta de tarefas vive na página mobile; emula o Pixel 7 como o
            // fluxo unificado do oráculo faz (check-in -> tarefas reaproveitam a página).
            page.set_device_profile(&ali_coins_browser::launch::pixel7_profile())
                .await
                .map_err(|error| error.to_string())?;
            // Pré-navegação para a origem do painel: permite semear o localStorage
            // e validar os cookies (Network.getCookies não lista nada em about:blank).
            let _ = page
                .goto(
                    ali_coins_flows::tasks_runner::DESKTOP_COIN_URL,
                    &ali_coins_browser::driver::NavOptions::default(),
                )
                .await;
            page.seed_storage_state(&session_data)
                .await
                .map_err(|error| error.to_string())?;
            if !has_auth_cookies(&*page)
                .await
                .map_err(|error| error.to_string())?
            {
                return Err(
                    "Sessão sem cookies de autenticação; rode `ali-coins checkin`.".to_string(),
                );
            }

            let started = Instant::now();
            let options = TasksOptions {
                max_actions: u32::try_from(config.task_max_actions).unwrap_or(25),
                max_attempts: u32::try_from(config.task_max_attempts).unwrap_or(4),
                scroll_wait: Duration::from_secs(config.scroll_wait_seconds),
                skip_app_only: config.skip_app_only_tasks,
                search_query: ali_coins_flows::tasks::SEARCH_QUERY.to_string(),
                nav_timeout: Duration::from_millis(config.nav_timeout),
            };
            let run = run_tasks(&*page, &options)
                .await
                .map_err(|error| error.to_string())?;
            let duration = ali_coins_core::time::format_duration(
                i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
            );

            // Persiste a sessão (cookies renovados pelas navegações).
            let final_state = page.storage_state().await.ok();
            if let Some(state) = &final_state {
                let _ = save_session(
                    &session_options,
                    &base_dir,
                    &env,
                    state.clone(),
                    &account.user,
                );
            }

            // Extrato desktop: saldo final + ganhos reais das tarefas.
            let desktop = ali_coins_flows::desktop::read_desktop_report(
                &*browser,
                final_state.as_ref(),
                Duration::from_millis(config.nav_timeout_short),
            )
            .await;
            let missions_from_ledger = desktop.as_ref().and_then(|data| data.today_missions_coins);
            let final_balance = desktop.as_ref().and_then(|data| data.total_balance.clone());

            let results: Vec<serde_json::Value> = run
                .results
                .iter()
                .map(|outcome| {
                    serde_json::json!({ "title": outcome.title, "status": outcome.status })
                })
                .collect();
            #[allow(clippy::cast_precision_loss)]
            let tasks_coins = missions_from_ledger.map(|value| value as f64);
            let tasks_input = TasksInput {
                results: Some(results),
                coins_gained: tasks_coins,
                coins_from_ledger: Some(missions_from_ledger.is_some()),
                final_balance: final_balance
                    .as_deref()
                    .map(|value| NumOrText::Text(value.to_string())),
                final_coins: final_balance.map(|value| format!("{value} moedas")),
                duration: Some(duration),
                ..TasksInput::default()
            };
            let payload = build_unified_report_payload(
                None,
                Some(&tasks_input),
                &UnifiedMeta {
                    user: Some(&account.user),
                    ..UnifiedMeta::default()
                },
            );
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
                );
            } else {
                logging::global().info(
                    &format!(
                        "Tarefas processadas: {} (ações: {})",
                        run.results.len(),
                        run.actions
                    ),
                    &[],
                );
            }
            // Notificação rica (best-effort) para execuções avulsas de tarefas.
            if config.telegram_enabled {
                let timeout = Duration::from_millis(config.telegram_timeout_ms);
                if let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) {
                    let host = notify_host(&config);
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
                    let event = if run.results.is_empty() {
                        TelegramEvent::AlreadyCollected
                    } else {
                        TelegramEvent::Success
                    };
                    let message = build_unified_report_message(
                        &payload,
                        &host,
                        env!("CARGO_PKG_VERSION"),
                        event,
                    );
                    match send_telegram(&client, &telegram_config, &message).await {
                        result if result.ok => {
                            logging::global().info("Notificação Telegram enviada.", &[]);
                        }
                        result => {
                            logging::global().warn(
                                &format!(
                                    "Falha ao enviar Telegram: {}",
                                    result
                                        .error
                                        .unwrap_or_else(|| "erro desconhecido".to_string())
                                ),
                                &[],
                            );
                        }
                    }
                }
            }

            if run.results.is_empty() {
                logging::global().warn("Nenhuma tarefa encontrada no painel (sem ação).", &[]);
                Ok(ExitCode::NoAction.as_i32())
            } else {
                Ok(ExitCode::Success.as_i32())
            }
        })
        .map_or_else(
            |error: String| {
                logging::global().error(&format!("Falha nas tarefas: {error}"), &[]);
                StdExitCode::from(u8::try_from(ExitCode::Failure.as_i32()).unwrap_or(1))
            },
            |code| StdExitCode::from(u8::try_from(code).unwrap_or(1)),
        )
}
