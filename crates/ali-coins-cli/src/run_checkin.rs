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
use ali_coins_core::report::{
    CheckinInput, StreakValue, UnifiedMeta, build_unified_report_payload, checkin_coins_from_streak,
};
use ali_coins_core::session::{SessionOptions, load_session_files, save_session, validate_session};
use ali_coins_core::{exit::ExitCode, logging};
use ali_coins_flows::checkin::{CheckinOptions, run_checkin};
use ali_coins_flows::login::LoginOptions;
use std::io::IsTerminal as _;
use std::process::ExitCode as StdExitCode;
use std::time::Duration;

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
            if let Ok(loaded) = load_session_files(&session_options, &base_dir, &env) {
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
                executable_path: std::env::var("ALI_COINS_CHROME")
                    .ok()
                    .map(std::path::PathBuf::from),
                user_data_dir: None,
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

            let checkin_options = CheckinOptions {
                login: LoginOptions {
                    interactive: std::io::stdin().is_terminal(),
                    ..LoginOptions::default()
                },
                confirm_timeout: Duration::from_secs(5),
                detect_timeout: Duration::from_millis(800),
            };
            let result = run_checkin(&*page, &account.user, &account.password, &checkin_options)
                .await
                .map_err(|error| format!("{error}"))?;

            // Persiste a sessão renovada (cookies + localStorage filtrado).
            if let Ok(state) = page.storage_state().await {
                let streak_days = result.streak_days;
                let mut save_options = session_options.clone();
                save_options.streak_days = streak_days;
                if save_session(&save_options, &base_dir, &env, state, &account.user).is_ok() {
                    logging::global().info("Sessão atualizada em disco.", &[]);
                }
            }

            // Relatório unificado (C-09).
            let streak_value = result
                .streak_days
                .map_or_else(|| StreakValue::Text("N/D".to_string()), StreakValue::Number);
            let coins = if !result.already_collected && result.collected {
                Some(checkin_coins_from_streak(Some(&streak_value)))
            } else {
                None
            };
            let checkin = CheckinInput {
                already_collected: Some(result.already_collected),
                coins_gained_today: coins.map(|value| value.to_string()),
                streak_days: Some(streak_value),
                total_balance: result.total_balance.clone(),
                duration: None,
                ..CheckinInput::default()
            };
            let payload = build_unified_report_payload(
                Some(&checkin),
                None,
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
                        "Check-in: {} | streak {} | saldo {}",
                        if result.already_collected {
                            "já coletado"
                        } else if result.collected {
                            "coletado"
                        } else {
                            "não coletado"
                        },
                        result
                            .streak_days
                            .map_or_else(|| "N/D".to_string(), |value| value.to_string()),
                        result.total_balance.as_deref().unwrap_or("N/D")
                    ),
                    &[],
                );
            }

            if result.already_collected {
                Ok(ExitCode::NoAction.as_i32())
            } else if result.collected {
                Ok(ExitCode::Success.as_i32())
            } else {
                Ok(ExitCode::Failure.as_i32())
            }
        })
        .map_or_else(
            |error: String| {
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
