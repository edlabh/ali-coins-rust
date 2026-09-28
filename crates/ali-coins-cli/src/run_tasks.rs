//! Subcomando `tasks`: executa o painel "Ganhe mais moedas" (runner conservador).

use crate::export_import::bootstrap;
use ali_coins_browser::cdp::CdpDriver;
use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
use ali_coins_browser::launch::{ChromiumArgsInput, build_chromium_args};
use ali_coins_core::lock::{LockError, LockOptions, acquire};
use ali_coins_core::report::{TasksInput, UnifiedMeta, build_unified_report_payload};
use ali_coins_core::session::{SessionOptions, load_session_files, save_session, validate_session};
use ali_coins_core::{exit::ExitCode, logging};
use ali_coins_flows::login::has_auth_cookies;
use ali_coins_flows::tasks_runner::{TasksOptions, run_tasks};
use std::process::ExitCode as StdExitCode;
use std::time::{Duration, Instant};

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
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
            page.enable_resource_blocking(config.allow_media)
                .await
                .map_err(|error| error.to_string())?;
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
            };
            let run = run_tasks(&*page, &options)
                .await
                .map_err(|error| error.to_string())?;
            let duration = ali_coins_core::time::format_duration(
                i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
            );

            // Persiste a sessão (cookies renovados pelas navegações).
            if let Ok(state) = page.storage_state().await {
                let _ = save_session(&session_options, &base_dir, &env, state, &account.user);
            }

            let results: Vec<serde_json::Value> = run
            .results
            .iter()
            .map(|outcome| {
                serde_json::json!({ "title": outcome.title, "status": outcome.status })
            })
            .collect();
            let tasks_input = TasksInput {
                results: Some(results),
                final_coins: Some("N/D".to_string()),
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
            Ok(ExitCode::Success.as_i32())
        })
        .map_or_else(
            |error: String| {
                logging::global().error(&format!("Falha nas tarefas: {error}"), &[]);
                StdExitCode::from(u8::try_from(ExitCode::Failure.as_i32()).unwrap_or(1))
            },
            |code| StdExitCode::from(u8::try_from(code).unwrap_or(1)),
        )
}
