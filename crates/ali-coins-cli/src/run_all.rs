//! Subcomando `all`: check-in + tarefas numa única execução, com um só relatório
//! e uma única notificação (paridade com `all.js` do oráculo).

use crate::export_import::bootstrap;
use ali_coins_browser::cdp::CdpDriver;
use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions, NavOptions};
use ali_coins_browser::launch::{ChromiumArgsInput, build_chromium_args, pixel7_profile};
use ali_coins_core::lock::{LockError, LockOptions, acquire};
use ali_coins_core::notify::{
    SafeHttpClient, TelegramConfig, TelegramContext, TelegramEvent, build_message,
    build_unified_report_message, send_telegram,
};
use ali_coins_core::report::{
    CheckinInput, NumOrText, TasksInput, UnifiedMeta, build_unified_report_payload,
    checkin_coins_from_streak,
};
use ali_coins_core::session::{SessionOptions, load_session_files, save_session, validate_session};
use ali_coins_core::{exit::ExitCode, logging};
use ali_coins_flows::checkin::{CheckinOptions, parse_total_balance, run_checkin};
use ali_coins_flows::login::{LoginOptions, has_auth_cookies};
use ali_coins_flows::tasks_runner::{DESKTOP_COIN_URL, TasksOptions, run_tasks};
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

/// O atraso inicial deve ser aplicado? (paridade `shouldApplyStartDelay` do oráculo)
fn should_apply_start_delay(no_delay: bool, max_ms: u64) -> bool {
    !no_delay && max_ms > 0
}

/// Fração aleatória simples (nanossegundos do relógio).
fn random_fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos());
    f64::from(nanos) / 1_000_000_000.0
}

/// Rótulo de host exibido nas notificações (`NOTIFY_HOST_LABEL` > hostname).
fn notify_host(config: &ali_coins_core::config::Config) -> String {
    if config.notify_host_label.trim().is_empty() {
        ali_coins_core::lock::hostname()
    } else {
        config.notify_host_label.clone()
    }
}

/// Duração formatada desde o início do cronômetro.
fn duration_since(started: &Instant) -> String {
    ali_coins_core::time::format_duration(
        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    )
}

/// Número do saldo exibido (`1.234 moedas` → `1234`).
fn parse_balance_number(text: &str) -> Option<f64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<f64>().ok()
}

/// Diferença de saldo entre duas leituras (negativa/ilegível → `None`).
fn balance_diff(initial: Option<&str>, final_value: Option<&str>) -> Option<f64> {
    let initial = parse_balance_number(initial?)?;
    let final_value = parse_balance_number(final_value?)?;
    (final_value >= initial).then_some(final_value - initial)
}

/// Notificação única do relatório unificado (best-effort).
async fn send_unified_notification(
    config: &ali_coins_core::config::Config,
    account: &ali_coins_core::config::Account,
    payload: &serde_json::Value,
    event: TelegramEvent,
) {
    let timeout = Duration::from_millis(config.telegram_timeout_ms);
    let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) else {
        return;
    };
    let host = notify_host(config);
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
    let message = build_unified_report_message(payload, &host, env!("CARGO_PKG_VERSION"), event);
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

/// `ali-coins all [--account <id>] [--json] [--force]`
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

    // Atraso inicial aleatório (anti-detecção; paridade `all.js`): fica ANTES do
    // lock/navegador, nunca roda com `--no-delay` e é desligado com teto 0.
    if should_apply_start_delay(has_flag(args, "--no-delay"), config.start_delay_max_ms) {
        #[allow(clippy::cast_precision_loss)]
        let delay_ms = ali_coins_core::time::pick_pause_ms(
            config.start_delay_min_ms as f64,
            config.start_delay_max_ms as f64,
            random_fraction(),
        );
        if delay_ms > 0 {
            let target = chrono::Utc::now()
                + chrono::Duration::milliseconds(i64::try_from(delay_ms).unwrap_or(i64::MAX));
            logging::global().info(
                &format!(
                    "Início atrasado em {}s (janela {}–{}s; início previsto às {})",
                    delay_ms / 1000,
                    config.start_delay_min_ms / 1000,
                    config.start_delay_max_ms / 1000,
                    crate::checkin_parity::format_target_time(target)
                ),
                &[],
            );
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }

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

            // Um único browser para as duas etapas (como o `all.js`).
            let chrome_args = build_chromium_args(&ChromiumArgsInput {
                env: &env,
                is_root: false,
                dev_shm_small: true,
                force_no_sandbox: false,
                low_memory: None,
            });
            let driver = CdpDriver::new();
            let launch_options = LaunchOptions {
                headless: config.headless,
                args: chrome_args,
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

            let main_started = Instant::now();
            let element_timeout = Duration::from_millis(config.element_timeout);
            let selector_timeout = Duration::from_millis(config.selector_timeout);

            // ----- ETAPA 1/2: check-in diário -----
            let step1_started = Instant::now();
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
            let result = run_checkin(&*page, &account.user, &account.password, &checkin_options)
                .await
                .map_err(|error| format!("{error}"))?;
            let step1_duration = duration_since(&step1_started);
            logging::global().info(
                &format!("Etapa 1/2 (check-in) concluída em {step1_duration}."),
                &[],
            );

            let just_collected = result.collected;
            let state_after_checkin = page.storage_state().await.ok();

            // Coleta de água da Fazenda Mágica (trecho do collect.js).
            let _ = crate::checkin_parity::collect_water(&*page).await;

            // Saldo/streak/extrato após o check-in (reusa a checagem inicial quando
            // nada foi coletado, como o oráculo).
            let desktop_after_checkin = if crate::checkin_parity::should_reuse_early_desktop(
                just_collected,
                early_desktop.as_ref(),
            ) {
                logging::global().info(
                    "Saldo/streak reutilizados da checagem inicial do desktop (sem nova leitura).",
                    &[],
                );
                early_desktop.clone()
            } else {
                ali_coins_flows::desktop::read_desktop_report(
                    &*browser,
                    state_after_checkin.as_ref(),
                    Duration::from_millis(config.nav_timeout_short),
                )
                .await
            };
            let was_already_collected_today = desktop_after_checkin
                .as_ref()
                .is_some_and(|data| data.has_checkin_today);
            let bonus_from_ledger = desktop_after_checkin
                .as_ref()
                .and_then(|data| data.today_bonus_coins);
            let checkin_coins_from_ledger = bonus_from_ledger.is_some_and(|value| value > 0);
            let already_collected =
                (result.already_collected || was_already_collected_today) && !just_collected;
            let confirmed_by_ledger = crate::checkin_parity::should_confirm_checkin_by_ledger(
                just_collected,
                already_collected,
                checkin_coins_from_ledger,
            );
            let resolved = crate::checkin_parity::resolve_streak(
                result.streak_days,
                previous_streak_days,
                early_desktop.as_ref().and_then(|data| data.desktop_streak),
                just_collected,
                already_collected,
                confirmed_by_ledger,
                desktop_after_checkin
                    .as_ref()
                    .and_then(|data| data.desktop_streak),
            );
            let streak_days = crate::checkin_parity::resolved_streak_number(&resolved);
            let streak_value = resolved.streak_days.clone();
            let checkin_coins = if let Some(bonus) = bonus_from_ledger {
                Some(bonus)
            } else if !already_collected && (just_collected || confirmed_by_ledger) {
                Some(checkin_coins_from_streak(Some(&streak_value)))
            } else {
                None
            };
            let mobile_balance = page
                .content()
                .await
                .ok()
                .and_then(|content| parse_total_balance(&content));
            let raw_balance = desktop_after_checkin
                .as_ref()
                .and_then(|data| data.total_balance.clone())
                .or_else(|| result.total_balance.clone())
                .or(mobile_balance);
            let balance_after_checkin = crate::checkin_parity::sync_balance_after_checkin(
                just_collected,
                checkin_coins,
                raw_balance.as_deref(),
                early_desktop
                    .as_ref()
                    .and_then(|data| data.total_balance.as_deref()),
            )
            .or(raw_balance);

            // Salva a sessão (cookies renovados + streak resolvido).
            if let Some(state) = &state_after_checkin {
                let mut save_options = session_options.clone();
                save_options.streak_days = streak_days;
                if save_session(&save_options, &base_dir, &env, state.clone(), &account.user)
                    .is_ok()
                {
                    logging::global().info("Sessão atualizada em disco.", &[]);
                }
            }

            // ----- ETAPA 2/2: tarefas diárias -----
            let step2_started = Instant::now();
            let _ = page.goto(DESKTOP_COIN_URL, &NavOptions::default()).await;
            if let Some(state) = &state_after_checkin {
                let _ = page.seed_storage_state(state).await;
            }
            let tasks_outcome = if has_auth_cookies(&*page).await.unwrap_or(false) {
                let options = TasksOptions::from_config(&config);
                run_tasks(&*page, &*browser, &options)
                    .await
                    .map_err(|error| error.to_string())
            } else {
                Err("Sessão sem cookies de autenticação após o check-in.".to_string())
            };
            let step2_duration = duration_since(&step2_started);
            let (tasks_run, tasks_error) = match tasks_outcome {
                Ok(run) => (Some(run), None),
                Err(error) => (None, Some(error)),
            };
            if let Some(error) = &tasks_error {
                logging::global().warn(&format!("Aviso na etapa de tarefas: {error}"), &[]);
            } else {
                logging::global().info(
                    &format!("Etapa 2/2 (tarefas) concluída em {step2_duration}."),
                    &[],
                );
            }

            // Salva a sessão novamente após as tarefas.
            let state_after_tasks = if tasks_run.is_some() {
                let state = page.storage_state().await.ok();
                if let Some(state) = &state {
                    let mut save_options = session_options.clone();
                    save_options.streak_days = streak_days;
                    let _ =
                        save_session(&save_options, &base_dir, &env, state.clone(), &account.user);
                }
                state
            } else {
                None
            };

            // Extrato desktop após as tarefas: saldo final + ganhos reais.
            let desktop_after_tasks = ali_coins_flows::desktop::read_desktop_report(
                &*browser,
                state_after_tasks.as_ref(),
                Duration::from_millis(config.nav_timeout_short),
            )
            .await;
            let mobile_balance = page
                .content()
                .await
                .ok()
                .and_then(|content| parse_total_balance(&content));
            let balance_after_tasks = desktop_after_tasks
                .as_ref()
                .and_then(|data| data.total_balance.clone())
                .or(mobile_balance);
            let missions_from_ledger = desktop_after_tasks
                .as_ref()
                .and_then(|data| data.today_missions_coins);
            let tasks_coins_from_ledger = missions_from_ledger.is_some();
            #[allow(clippy::cast_precision_loss)]
            let tasks_coins = missions_from_ledger.map(|value| value as f64).or_else(|| {
                balance_diff(
                    balance_after_checkin.as_deref(),
                    balance_after_tasks.as_deref(),
                )
            });

            // ----- ETAPA 3/3: relatório consolidado + notificação única -----
            let checkin_input = CheckinInput {
                already_collected: Some(already_collected),
                coins_gained_today: checkin_coins.map(|value| value.to_string()),
                streak_days: Some(streak_value),
                total_balance: balance_after_checkin.clone(),
                duration: Some(step1_duration.clone()),
                checkin_coins_from_ledger: Some(checkin_coins_from_ledger),
                ..CheckinInput::default()
            };
            let tasks_input = tasks_run.as_ref().map(|run| TasksInput {
                results: Some(
                    run.results
                        .iter()
                        .map(|outcome| {
                            serde_json::json!({
                                "title": outcome.title,
                                "status": outcome.status
                            })
                        })
                        .collect(),
                ),
                coins_gained: tasks_coins,
                coins_from_ledger: Some(tasks_coins_from_ledger),
                initial_balance: balance_after_checkin
                    .as_deref()
                    .map(|value| NumOrText::Text(value.to_string())),
                final_balance: balance_after_tasks
                    .as_deref()
                    .map(|value| NumOrText::Text(value.to_string())),
                final_coins: balance_after_tasks.as_deref().map(|value| {
                    if value.contains("moedas") {
                        value.to_string()
                    } else {
                        format!("{value} moedas")
                    }
                }),
                duration: Some(step2_duration.clone()),
                ..TasksInput::default()
            });
            let total_duration = duration_since(&main_started);
            let payload = build_unified_report_payload(
                Some(&checkin_input),
                tasks_input.as_ref(),
                &UnifiedMeta {
                    user: Some(&account.user),
                    total_duration: Some(total_duration.as_str()),
                    step1_duration: Some(step1_duration.as_str()),
                    step2_duration: Some(step2_duration.as_str()),
                    tasks_error: tasks_error.as_deref(),
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
                        "Execução concluída: check-in {} | tarefas {} | duração {}",
                        if just_collected || confirmed_by_ledger {
                            "coletado"
                        } else if already_collected {
                            "já coletado"
                        } else {
                            "sem ação"
                        },
                        tasks_run.as_ref().map_or_else(
                            || "falhou".to_string(),
                            |run| format!("{} tarefas ({} ações)", run.results.len(), run.actions)
                        ),
                        total_duration
                    ),
                    &[],
                );
            }

            let had_new_checkin = !already_collected || checkin_coins_from_ledger;
            let had_task_actions = tasks_run.as_ref().is_some_and(|run| run.actions > 0);
            let event = if !had_new_checkin && !had_task_actions {
                TelegramEvent::AlreadyCollected
            } else {
                TelegramEvent::Success
            };
            if config.telegram_enabled {
                send_unified_notification(&config, account, &payload, event).await;
            }

            if had_new_checkin || had_task_actions {
                Ok(ExitCode::Success.as_i32())
            } else {
                Ok(ExitCode::NoAction.as_i32())
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
                    let context = TelegramContext {
                        user: Some(account.masked_user.as_str()),
                        error: Some(error.as_str()),
                        host: Some(host.as_str()),
                        version: Some(env!("CARGO_PKG_VERSION")),
                        ..TelegramContext::default()
                    };
                    let message = build_message(TelegramEvent::Failure, &context);
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
                    logging::global().error(&format!("Falha na execução: {error}"), &[]);
                    StdExitCode::from(u8::try_from(ExitCode::Failure.as_i32()).unwrap_or(1))
                }
            },
            |code| StdExitCode::from(u8::try_from(code).unwrap_or(1)),
        )
}

#[cfg(test)]
mod tests {
    use super::should_apply_start_delay;

    #[test]
    fn atraso_inicial_so_com_max_positivo_e_sem_no_delay() {
        assert!(!should_apply_start_delay(false, 0));
        assert!(!should_apply_start_delay(true, 5000));
        assert!(should_apply_start_delay(false, 5000));
    }
}
