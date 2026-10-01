//! Subcomando `all`: check-in + tarefas numa única execução, com um só relatório
//! e uma única notificação (paridade com `all.js` do oráculo).

use crate::export_import::bootstrap;
use ali_coins_browser::cdp::CdpDriver;
use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions, NavOptions};
use ali_coins_browser::launch::{ChromiumArgsInput, build_chromium_args, pixel7_profile};
use ali_coins_core::lock::{LockError, LockOptions, acquire};
use ali_coins_core::notify::{
    HeartbeatAction, HeartbeatConfig, SafeHttpClient, TelegramConfig, TelegramContext,
    TelegramEvent, build_message, build_unified_report_message, send_heartbeat, send_telegram,
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
pub(crate) fn should_apply_start_delay(no_delay: bool, max_ms: u64) -> bool {
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
pub(crate) fn notify_host(config: &ali_coins_core::config::Config) -> String {
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

/// Envia uma mensagem já montada (best-effort).
pub(crate) async fn send_message(
    config: &ali_coins_core::config::Config,
    account: &ali_coins_core::config::Account,
    message: &str,
) {
    let timeout = Duration::from_millis(config.telegram_timeout_ms);
    let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) else {
        return;
    };
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
    match send_telegram(&client, &telegram_config, message).await {
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

/// Notificação do relatório unificado (best-effort).
async fn send_unified_notification(
    config: &ali_coins_core::config::Config,
    account: &ali_coins_core::config::Account,
    payload: &serde_json::Value,
    event: TelegramEvent,
) {
    let host = notify_host(config);
    let message = build_unified_report_message(payload, &host, env!("CARGO_PKG_VERSION"), event);
    send_message(config, account, &message).await;
}

/// Envia um heartbeat (best-effort), como o `all.js`.
pub(crate) async fn send_heartbeat_action(
    config: &ali_coins_core::config::Config,
    action: HeartbeatAction,
    payload: Option<&str>,
) {
    if !config.heartbeat_enabled || config.heartbeat_url.trim().is_empty() {
        return;
    }
    let timeout = Duration::from_millis(config.heartbeat_timeout_ms);
    let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) else {
        return;
    };
    let heartbeat = HeartbeatConfig {
        enabled: true,
        url: config.heartbeat_url.clone(),
        timeout_ms: config.heartbeat_timeout_ms,
    };
    let host = notify_host(config);
    let result = send_heartbeat(&client, action, &heartbeat, &host, payload).await;
    if result.ok {
        logging::global().info("Heartbeat enviado.", &[]);
    } else if !result.skipped {
        logging::global().warn(
            &format!(
                "Falha no heartbeat: {}",
                result
                    .error
                    .unwrap_or_else(|| "erro desconhecido".to_string())
            ),
            &[],
        );
    }
}

/// Falha de lock: heartbeat + Telegram (`lock_active`/`failure`), como o `all.js`.
fn notify_lock_failure(
    config: &ali_coins_core::config::Config,
    account: &ali_coins_core::config::Account,
    error: &str,
    active: bool,
) {
    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        return;
    };
    runtime.block_on(async {
        send_heartbeat_action(config, HeartbeatAction::Fail, Some(error)).await;
        let host = notify_host(config);
        let context = TelegramContext {
            user: Some(account.masked_user.as_str()),
            error: Some(error),
            host: Some(host.as_str()),
            version: Some(env!("CARGO_PKG_VERSION")),
            ..TelegramContext::default()
        };
        let event = if active {
            TelegramEvent::LockActive
        } else {
            TelegramEvent::Failure
        };
        let message = build_message(event, &context);
        send_message(config, account, &message).await;
    });
}

/// `ali-coins all [--account <id>] [--json] [--force]`
pub fn run(args: &[String]) -> StdExitCode {
    let json = has_flag(args, "--json");
    let Some((base_dir, env, config, accounts)) = bootstrap() else {
        return StdExitCode::from(1);
    };
    // Modo multi-conta (Fase 5): mais de uma conta configurada ou `--all` explícito.
    if accounts.len() > 1 || has_flag(args, "--all") {
        return crate::run_multi::run(args);
    }
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
            notify_lock_failure(&config, account, &message, true);
            return StdExitCode::from(3);
        }
        Err(error) => {
            let message = format!("Falha ao adquirir o lock: {error}");
            logging::global().error(&message, &[]);
            notify_lock_failure(&config, account, &message, false);
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
            // Dead man's switch: sinal de início (paridade all.js).
            send_heartbeat_action(&config, HeartbeatAction::Start, None).await;

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

            // D-03: trace CDP (opt-in; desligado por padrão em host de baixa memória).
            let trace_mode = ali_coins_browser::trace::resolve_trace_mode(&env);
            let trace_dir = ali_coins_browser::trace::diagnostics_dir(&env);
            let (trace_should_start, _) = ali_coins_browser::trace::decide_trace(trace_mode, false);
            let trace_started =
                trace_should_start && page.start_trace().await.unwrap_or(false);
            if trace_started {
                logging::global().info("Trace CDP iniciado (PW_TRACE).", &[]);
            }

            let flow_result: Result<i32, String> = async {

            // Pré-checagem desktop (D-08): saldo/streak/extrato antes do mobile.
            let early_desktop = crate::checkin_parity::read_early_desktop(
                &*browser,
                storage_state.as_ref(),
                Duration::from_millis(config.nav_timeout_short),
            )
            .await;

            let main_started = Instant::now();
            let main_start_time = chrono::Utc::now();
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
            let bonus_from_ledger_checkin = desktop_after_checkin
                .as_ref()
                .and_then(|data| data.today_bonus_coins);
            let checkin_coins_from_ledger_checkin =
                bonus_from_ledger_checkin.is_some_and(|value| value > 0);
            let already_collected =
                (result.already_collected || was_already_collected_today) && !just_collected;
            let confirmed_by_ledger = crate::checkin_parity::should_confirm_checkin_by_ledger(
                just_collected,
                already_collected,
                checkin_coins_from_ledger_checkin,
            );
            // Confirmação da quebra de streak pelo extrato (leitura fresca quando
            // a tela lê 1 com histórico anterior > 1 e o extrato ainda não veio).
            let mut statement_streak = desktop_after_checkin
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
            let checkin_coins_estimate = if let Some(bonus) = bonus_from_ledger_checkin {
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
                checkin_coins_estimate,
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
                .filter(|data| data.today_missions_count > 0)
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
            // Fonte de verdade do extrato desktop: o bônus do check-in pode não
            // ter vindo na leitura pós-check-in (reutilizada); a leitura final
            // (após as tarefas) completa o valor creditado hoje.
            let bonus_from_ledger = bonus_from_ledger_checkin.or_else(|| {
                desktop_after_tasks
                    .as_ref()
                    .and_then(|data| data.today_bonus_coins)
            });
            let checkin_coins_from_ledger = bonus_from_ledger.is_some_and(|value| value > 0);
            let checkin_coins = if let Some(bonus) = bonus_from_ledger {
                Some(bonus)
            } else {
                checkin_coins_estimate
            };
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
                                "status": outcome.status,
                                "coins": outcome.coins
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
                    main_start_time: Some(main_start_time),
                    main_end_time: Some(chrono::Utc::now()),
                    tasks_error: tasks_error.as_deref(),
                },
            );

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
                );
            } else {
                crate::report_render::render_unified(
                    &payload,
                    Some(&checkin_input),
                    &account.masked_user,
                );
            }

            let had_new_checkin = !already_collected || checkin_coins_from_ledger;
            let had_task_actions = tasks_run.as_ref().is_some_and(|run| run.actions > 0);

            // Alerta crítico de streak quebrado (paridade all.js): evento dedicado
            // e exit 4 (perda irreversível após dias de sequência).
            #[allow(clippy::cast_precision_loss)]
            let streak_broken = ali_coins_core::report::is_streak_break(
                streak_days.map(|value| value as f64),
                previous_streak_days.map(|value| value as f64),
                already_collected,
            );
            if streak_broken {
                logging::global().error(
                    &format!(
                        "🚨 ALERTA CRÍTICO: Streak quebrado! A sequência diária de check-in foi interrompida ou resetada (ontem {previous_streak_days:?} -> hoje {streak_days:?})."
                    ),
                    &[],
                );
            }

            let event = if streak_broken {
                TelegramEvent::StreakBreak
            } else if !had_new_checkin && !had_task_actions {
                TelegramEvent::AlreadyCollected
            } else {
                TelegramEvent::Success
            };
            if config.telegram_enabled {
                if streak_broken {
                    let host = notify_host(&config);
                    let previous_display = previous_streak_days
                        .map_or_else(|| "N/D".to_string(), |value| value.to_string());
                    let streak_display =
                        streak_days.map_or_else(|| "N/D".to_string(), |value| value.to_string());
                    let balance_display = payload
                        .get("meta")
                        .and_then(|meta| meta.get("finalBalance"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("N/D")
                        .to_string();
                    let context = TelegramContext {
                        user: Some(account.masked_user.as_str()),
                        total_balance: Some(balance_display.as_str()),
                        streak_days: Some(streak_display.as_str()),
                        previous_streak_days: Some(previous_display.as_str()),
                        host: Some(host.as_str()),
                        version: Some(env!("CARGO_PKG_VERSION")),
                        ..TelegramContext::default()
                    };
                    let message = build_message(TelegramEvent::StreakBreak, &context);
                    send_message(&config, account, &message).await;
                } else {
                    send_unified_notification(&config, account, &payload, event).await;
                }
            }

            if streak_broken {
                send_heartbeat_action(
                    &config,
                    HeartbeatAction::Fail,
                    Some(&format!(
                        "Streak quebrado: ontem {previous_streak_days:?} dias -> hoje {streak_days:?} dias"
                    )),
                )
                .await;
                Ok(ExitCode::StreakBroken.as_i32())
            } else {
                let payload_json = serde_json::to_string(&payload).unwrap_or_default();
                send_heartbeat_action(&config, HeartbeatAction::Success, Some(&payload_json)).await;
                if had_new_checkin || had_task_actions {
                    Ok(ExitCode::Success.as_i32())
                } else {
                    Ok(ExitCode::NoAction.as_i32())
                }
            }
        }
        .await;

        let (_, trace_keep) =
            ali_coins_browser::trace::decide_trace(trace_mode, flow_result.is_err());
        if trace_started {
            match page.stop_trace(&trace_dir, "all", trace_keep).await {
                Ok(Some(path)) => logging::global().warn(
                    &format!("Trace CDP salvo em {}", path.display()),
                    &[],
                ),
                Ok(None) => {}
                Err(error) => logging::global().debug(
                    &format!("Falha ao finalizar o trace CDP: {error}"),
                    &[],
                ),
            }
        }
        flow_result
        })
        .map_or_else(
            |error: String| {
                // Notifica a falha (best-effort) antes de sair.
                runtime.block_on(async {
                    send_heartbeat_action(&config, HeartbeatAction::Fail, Some(&error)).await;
                });
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
                            "[Sessão Remota Expirada] Falha durante a execução: a sessão importada expirou ou foi invalidada pelo AliExpress. Gere uma nova sessão executando \"node export_session.js\" no servidor de origem e importe-a com \"node import_session.js\".",
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
