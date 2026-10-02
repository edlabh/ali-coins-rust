//! Subcomando `checkin`: executa o check-in diário com o browser real.
//!
//! Fluxo: config → lock (exit 3) → sessão (load/seed) → `CdpDriver` (política de
//! launch + Pixel 7 + bloqueio de recursos) → `run_checkin` → salvar sessão →
//! relatório `unified_report` (JSON ou texto) → exit codes 0/2/5.

use crate::export_import::bootstrap;
use ali_coins_browser::cdp::CdpDriver;
use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
use ali_coins_browser::launch::{ChromiumArgsInput, build_chromium_args, pixel7_profile};
use ali_coins_core::lock::{LockError, LockOptions, acquire};
use ali_coins_core::notify::{
    SafeHttpClient, TelegramConfig, TelegramContext, TelegramEvent, build_message,
    build_unified_report_message, send_telegram,
};
use ali_coins_core::report::{
    CheckinInput, UnifiedMeta, build_unified_report_payload, checkin_coins_from_streak,
};
use ali_coins_core::session::{SessionOptions, load_session_files, save_session, validate_session};
use ali_coins_core::{exit::ExitCode, logging};
use ali_coins_flows::checkin::{CheckinOptions, run_checkin};
use ali_coins_flows::login::LoginOptions;
use std::io::IsTerminal as _;
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

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

/// `ali-coins checkin [--account <id>] [--json] [--force]`
pub fn run(args: &[String]) -> StdExitCode {
    let json = has_flag(args, "--json");
    let Some((base_dir, env, config, accounts)) = bootstrap() else {
        return StdExitCode::from(1);
    };
    let account = if let Some(selector) = flag_value(args, "--account") {
        let parsed_index = selector.parse::<usize>().ok();
        let found = accounts.iter().find(|account| {
            parsed_index.map_or_else(
                || account.user.eq_ignore_ascii_case(selector),
                |index| account.index == index,
            )
        });
        if let Some(account) = found {
            account
        } else {
            eprintln!("Conta '{selector}' não encontrada.");
            return StdExitCode::from(1);
        }
    } else {
        &accounts[0]
    };

    // Lock inter-processo (exit 3 quando ativo).
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

    // Estado de sessão importada para o alerta de falha (fallback do oráculo).
    let meta_imported =
        ali_coins_core::session::session_meta_is_imported(&account.session_meta_path);

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

            // Sessão existente (se válida, evita login).
            let mut storage_state = None;
            let mut previous_streak_days = None;
            if let Ok(loaded) = load_session_files(&session_options, &base_dir, &env) {
                previous_streak_days = loaded
                    .meta_data
                    .as_ref()
                    .and_then(|meta| meta.last_streak_days);
                if let Some(session_data) = &loaded.session_data {
                    if validate_session(
                        session_data,
                        loaded.meta_data.as_ref(),
                        Some(&account.user),
                    )
                    .valid
                    {
                        storage_state = Some(session_data.clone());
                    }
                }
            }

            // Browser com a política de launch portada.
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
                executable_path: ali_coins_browser::launch::resolve_chromium_path(&env),
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
            page.set_device_profile(&pixel7_profile())
                .await
                .map_err(|error| error.to_string())?;
            page.enable_resource_blocking(config.allow_media)
                .await
                .map_err(|error| error.to_string())?;
            if let Some(state) = &storage_state {
                let _ = page.seed_storage_state(state).await;
            }

            // Pré-checagem desktop (D-08): saldo/streak/extrato antes do mobile.
            let early_desktop = crate::checkin_parity::read_early_desktop(
                &*browser,
                storage_state.as_ref(),
                Duration::from_millis(config.nav_timeout_short),
            )
            .await;

            let element_timeout = Duration::from_millis(config.element_timeout);
            let selector_timeout = Duration::from_millis(config.selector_timeout);
            let checkin_options = CheckinOptions {
                login: LoginOptions {
                    interactive: std::io::stdin().is_terminal(),
                    detect_timeout: element_timeout,
                    password_wait_timeout: selector_timeout,
                    ..LoginOptions::default()
                },
                confirm_timeout: Duration::from_secs(config.scroll_wait_seconds.max(3)),
                detect_timeout: element_timeout,
                nav_timeout: Duration::from_millis(config.nav_timeout),
            };
            let step_started = Instant::now();
            let step_start_time = chrono::Utc::now();
            let result = run_checkin(&*page, &account.user, &account.password, &checkin_options)
                .await
                .map_err(|error| format!("{error}"))?;
            let step_end_time = chrono::Utc::now();
            let step_duration = ali_coins_core::time::format_duration(
                i64::try_from(step_started.elapsed().as_millis()).unwrap_or(i64::MAX),
            );

            let just_collected = result.collected;

            // Coleta de água da Fazenda Mágica (trecho do collect.js).
            let _ = crate::checkin_parity::collect_water(&*page).await;

            // 5. Confirmar resultado e saldo no desktop (reusa a checagem inicial
            // quando nada foi coletado, como o oráculo).
            let mut desktop = if crate::checkin_parity::should_reuse_early_desktop(
                just_collected,
                early_desktop.as_ref(),
            ) {
                logging::global().info(
                    "Saldo/streak reutilizados da checagem inicial do desktop (sem nova leitura).",
                    &[],
                );
                early_desktop.clone()
            } else {
                let state = page.storage_state().await.ok();
                crate::checkin_parity::read_early_desktop(
                    &*browser,
                    state.as_ref(),
                    Duration::from_millis(config.nav_timeout_short),
                )
                .await
            };
            // Fonte de verdade do extrato: se a leitura escolhida não trouxe a
            // seção do dia (ledger), tenta uma leitura fresca única para exibir
            // o valor real do check-in e das tarefas.
            if desktop.as_ref().is_some_and(|data| !data.ledger_available) {
                let state = page.storage_state().await.ok();
                if let Some(fresh) = crate::checkin_parity::read_early_desktop(
                    &*browser,
                    state.as_ref(),
                    Duration::from_millis(config.nav_timeout_short),
                )
                .await
                {
                    if fresh.ledger_available {
                        ali_coins_core::logging::global().info(
                            "Extrato do dia obtido em leitura fresca (ledger indisponível na anterior).",
                            &[],
                        );
                        desktop = Some(fresh);
                    }
                }
            }
            let desktop = desktop;
            let was_already_collected_today =
                desktop.as_ref().is_some_and(|data| data.has_checkin_today);
            let bonus_from_ledger = desktop.as_ref().and_then(|data| data.today_bonus_coins);
            let checkin_coins_from_ledger = bonus_from_ledger.is_some_and(|value| value > 0);
            let already_collected =
                (result.already_collected || was_already_collected_today) && !just_collected;
            let confirmed_by_ledger = crate::checkin_parity::should_confirm_checkin_by_ledger(
                just_collected,
                already_collected,
                checkin_coins_from_ledger,
            );
            // Confirmação da quebra de streak pelo extrato (leitura fresca quando
            // a tela lê 1 com histórico anterior > 1 e o extrato ainda não veio).
            let mut statement_streak = desktop
                .as_ref()
                .and_then(|data| data.desktop_streak);
            if crate::checkin_parity::should_confirm_streak_by_statement(
                result.streak_days,
                previous_streak_days,
                statement_streak,
                already_collected,
            ) {{
                logging::global().warn(
                    "Leitura de streak = 1 com histórico anterior > 1; confirmando a quebra pelo extrato desktop...",
                    &[],
                );
                let state = page.storage_state().await.ok();
                if let Some(confirm_read) = crate::checkin_parity::read_early_desktop(
                    &*browser,
                    state.as_ref(),
                    Duration::from_millis(config.nav_timeout_short),
                )
                .await
                {{
                    if confirm_read.desktop_streak.is_some() {{
                        statement_streak = confirm_read.desktop_streak;
                    }}
                    if statement_streak.is_some_and(|value| value > 1) {{
                        logging::global().info(
                            "Extrato desmente a quebra (sequência do extrato > 1); preservando o streak real.",
                            &[],
                        );
                    }} else {{
                        logging::global().warn(
                            "Extrato não desmente a quebra (sequência do extrato <= 1 ou indisponível).",
                            &[],
                        );
                    }}
                }} else {{
                    logging::global().warn(
                        "Falha ao confirmar a quebra de streak pelo extrato; mantendo a leitura da tela.",
                        &[],
                    );
                }}
            }}

            let resolved = crate::checkin_parity::resolve_streak(
                result.streak_days,
                previous_streak_days,
                early_desktop.as_ref().and_then(|data| data.desktop_streak),
                just_collected,
                already_collected,
                confirmed_by_ledger,
                statement_streak,
            );
            let streak_days = crate::checkin_parity::resolved_streak_number(&resolved);
            let streak_value = resolved.streak_days.clone();
            let coins = if let Some(bonus) = bonus_from_ledger {
                Some(bonus)
            } else if !already_collected && (just_collected || confirmed_by_ledger) {
                Some(checkin_coins_from_streak(Some(&streak_value)))
            } else {
                None
            };
            let raw_balance = desktop
                .as_ref()
                .and_then(|data| data.total_balance.clone())
                .or_else(|| result.total_balance.clone());
            let total_balance = crate::checkin_parity::sync_balance_after_checkin(
                just_collected,
                coins,
                raw_balance.as_deref(),
                early_desktop
                    .as_ref()
                    .and_then(|data| data.total_balance.as_deref()),
            )
            .or(raw_balance);

            // Persiste a sessão renovada (cookies + localStorage filtrado).
            let final_state = page.storage_state().await.ok();
            if let Some(state) = &final_state {
                let mut save_options = session_options.clone();
                save_options.streak_days = streak_days;
                if save_session(&save_options, &base_dir, &env, state.clone(), &account.user)
                    .is_ok()
                {
                    logging::global().info("Sessão atualizada em disco.", &[]);
                }
            }

            // Relatório unificado (C-09).
            let checkin = CheckinInput {
                already_collected: Some(already_collected),
                coins_gained_today: coins.map(|value| value.to_string()),
                streak_days: Some(streak_value),
                total_balance: total_balance.clone(),
                duration: Some(step_duration.clone()),
                start_time: Some(step_start_time.to_rfc3339()),
                end_time: Some(step_end_time.to_rfc3339()),
                checkin_coins_from_ledger: Some(checkin_coins_from_ledger),
                ..CheckinInput::default()
            };
            let payload = build_unified_report_payload(
                Some(&checkin),
                None,
                &UnifiedMeta {
                    user: Some(&account.user),
                    total_duration: Some(step_duration.as_str()),
                    step1_duration: Some(step_duration.as_str()),
                    main_start_time: Some(step_start_time),
                    main_end_time: Some(step_end_time),
                    ..UnifiedMeta::default()
                },
            );
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
                );
            } else {
                crate::report_render::render_checkin(&checkin);
            }

            // Notificação Telegram (best-effort; nunca falha o fluxo).
            if config.telegram_enabled {
                let timeout = Duration::from_millis(config.telegram_timeout_ms);
                if let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) {
                    let event =
                        if just_collected || checkin_coins_from_ledger || confirmed_by_ledger {
                            TelegramEvent::Success
                        } else if already_collected {
                            TelegramEvent::AlreadyCollected
                        } else {
                            TelegramEvent::Failure
                        };
                    let streak_display =
                        streak_days.map_or_else(|| "N/D".to_string(), |value| value.to_string());
                    let host = notify_host(&config);
                    let chat_id = account
                        .telegram_chat_id
                        .clone()
                        .unwrap_or_else(|| config.telegram_chat_id.clone());
                    let context = TelegramContext {
                        user: Some(account.masked_user.as_str()),
                        total_balance: total_balance.as_deref(),
                        coins_gained: coins,
                        streak_days: Some(streak_display.as_str()),
                        previous_streak_days: None,
                        duration: None,
                        error: None,
                        host: Some(host.as_str()),
                        version: Some(env!("CARGO_PKG_VERSION")),
                        imported_session_expired: false,
                    };
                    let telegram_config = TelegramConfig {
                        enabled: true,
                        bot_token: config.telegram_bot_token.clone(),
                        chat_id,
                        silent: config.telegram_silent,
                        timeout_ms: config.telegram_timeout_ms,
                        api_base: String::new(),
                    };
                    let message = match event {
                        TelegramEvent::Success | TelegramEvent::AlreadyCollected => {
                            build_unified_report_message(
                                &payload,
                                &host,
                                env!("CARGO_PKG_VERSION"),
                                event,
                            )
                        }
                        _ => build_message(event, &context),
                    };
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

            if just_collected || checkin_coins_from_ledger || confirmed_by_ledger {
                Ok(ExitCode::Success.as_i32())
            } else if already_collected {
                Ok(ExitCode::NoAction.as_i32())
            } else {
                Ok(ExitCode::Failure.as_i32())
            }
        })
        .map_or_else(
            |error: String| {
                // Notifica a falha (best-effort) antes de sair.
                if config.telegram_enabled {
                    let timeout = Duration::from_millis(config.telegram_timeout_ms);
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
                    let imported_session_expired =
                        ali_coins_core::notify::telegram::detect_imported_session_expired(
                            Some(error.as_str()),
                            meta_imported,
                        );
                    if imported_session_expired {
                        logging::global().error(
                            "[Sessão Remota Expirada] A execução falhou e a sessão ativa foi importada de outro host. Gere uma nova sessão executando \"node export_session.js\" no servidor de origem e importe-a com \"node import_session.js\".",
                            &[],
                        );
                    }
                    let context = TelegramContext {
                        user: Some(account.masked_user.as_str()),
                        error: Some(error.as_str()),
                        host: Some(host.as_str()),
                        version: Some(env!("CARGO_PKG_VERSION")),
                        imported_session_expired,
                        ..TelegramContext::default()
                    };
                    // Eventos dedicados como no oráculo (2FA/captcha/falha genérica).
                    let telegram_event = if error.contains("2FA") || error.contains("não-interativa")
                    {
                        TelegramEvent::TwoFactorRequired
                    } else if error.to_lowercase().contains("captcha") {
                        TelegramEvent::CaptchaRequired
                    } else {
                        TelegramEvent::Failure
                    };
                    let message = build_message(telegram_event, &context);
                    runtime.block_on(async {
                        if let Ok(client) =
                            SafeHttpClient::new(config.allow_private_webhooks, timeout)
                        {
                            let _ = send_telegram(&client, &telegram_config, &message).await;
                        }
                    });
                }
                if error.contains("2FA") || error.contains("não-interativa") {
                    logging::global().error(&error, &[]);
                    StdExitCode::from(u8::try_from(ExitCode::TwoFactor.as_i32()).unwrap_or(1))
                } else {
                    logging::global().error(&format!("Falha no check-in: {error}"), &[]);
                    StdExitCode::from(u8::try_from(ExitCode::Failure.as_i32()).unwrap_or(1))
                }
            },
            |code| StdExitCode::from(u8::try_from(code).unwrap_or(1)),
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
