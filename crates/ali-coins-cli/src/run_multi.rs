//! Execução multi-conta sequencial (Fase 5) — paridade com o fluxo multi-conta
//! do `all.js`: contas em sequência com lock por conta, backoff/jitter entre
//! contas, relatório `multi_account_report`, notificação consolidada e exit
//! codes agregados.
//!
//! Cada conta roda em um processo filho (`all --account <user> --json`) para
//! manter o isolamento exato da execução de conta única; o processo pai apenas
//! orquestra, agrega e notifica.

use crate::context::bootstrap;
use ali_coins_core::config::{Account, EnvSource};
use ali_coins_core::exit::ExitCode;
use ali_coins_core::notify::telegram::{
    build_multi_account_message_at, detect_imported_session_expired,
};
use ali_coins_core::notify::{
    HeartbeatAction, SafeHttpClient, TelegramConfig, TelegramContext, TelegramEvent, build_message,
    send_telegram,
};
use ali_coins_core::report::{
    AccountRef, AccountResultInput, MultiAccountMeta, build_multi_account_report_payload,
};
use ali_coins_core::{logging, time};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

/// Resultado da execução de uma conta (runner injetável).
#[derive(Debug, Clone, Default)]
pub struct AccountExecution {
    /// Usuário bruto.
    pub user: String,
    /// Usuário mascarado.
    pub masked_user: String,
    /// Código de saída do runner da conta.
    pub exit_code: i32,
    /// Erro (quando houver).
    pub error: Option<String>,
    /// Payload `unified_report` da conta (quando houver).
    pub payload: Option<Value>,
    /// `session_meta.json` da conta aponta sessão importada (pré-execução).
    pub meta_imported: bool,
    /// Início/fim/dduração.
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub duration: String,
}

/// Flags agregadas para decidir evento/exit code (port de `all.js`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct MultiFlags {
    /// Alguma conta obteve sucesso (sem erro).
    pub any_success: bool,
    /// Alguma conta teve ação nova (exit 0).
    pub any_new_action: bool,
    /// Alguma conta com streak quebrado (exit 4).
    pub any_streak_broken: bool,
    /// Alguma conta com 2FA (exit 5).
    pub any_2fa: bool,
    /// Alguma conta com captcha.
    pub any_captcha: bool,
    /// Contas com lock ativo (exit 3).
    pub lock_active_count: u32,
    /// Falhas que não são lock.
    pub non_lock_failures: u32,
}

impl MultiFlags {
    /// Agrega as flags a partir das execuções.
    #[must_use]
    pub fn from_executions(executions: &[AccountExecution]) -> Self {
        let mut flags = Self::default();
        for execution in executions {
            match execution.exit_code {
                0 => {
                    flags.any_success = true;
                    flags.any_new_action = true;
                }
                2 => flags.any_success = true,
                3 => {
                    flags.lock_active_count += 1;
                    continue;
                }
                4 => flags.any_streak_broken = true,
                5 => flags.any_2fa = true,
                _ => {}
            }
            if execution.error.is_some() && execution.exit_code != 3 {
                flags.non_lock_failures += 1;
            }
            if execution
                .error
                .as_deref()
                .is_some_and(|error| error.to_lowercase().contains("captcha"))
            {
                flags.any_captcha = true;
            }
        }
        flags
    }

    /// Todas as contas bloqueadas por lock e nenhuma falha genérica.
    #[must_use]
    pub fn all_accounts_locked(&self) -> bool {
        !self.any_success && self.lock_active_count > 0 && self.non_lock_failures == 0
    }

    /// Evento consolidado do Telegram (port da seleção do `all.js`).
    #[must_use]
    pub fn event(&self) -> TelegramEvent {
        if self.all_accounts_locked() {
            TelegramEvent::LockActive
        } else if self.any_streak_broken {
            TelegramEvent::StreakBreak
        } else if self.any_2fa && !self.any_success {
            TelegramEvent::TwoFactorRequired
        } else if self.any_captcha && !self.any_success {
            TelegramEvent::CaptchaRequired
        } else if !self.any_success || self.non_lock_failures > 0 {
            TelegramEvent::Failure
        } else if !self.any_new_action {
            TelegramEvent::AlreadyCollected
        } else {
            TelegramEvent::Success
        }
    }

    /// Exit code agregado (port da seleção do `all.js`).
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        if self.all_accounts_locked() {
            ExitCode::LockActive
        } else if self.any_streak_broken {
            ExitCode::StreakBroken
        } else if self.any_2fa && !self.any_success {
            ExitCode::TwoFactor
        } else if !self.any_success || self.non_lock_failures > 0 {
            ExitCode::Failure
        } else if !self.any_new_action {
            ExitCode::NoAction
        } else {
            ExitCode::Success
        }
    }
}

/// Espera entre contas: backoff de falha composto com a pausa aleatória (maior).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn account_wait_ms(
    consecutive_failures: u32,
    delay_min_ms: u64,
    delay_max_ms: u64,
    random_fraction: f64,
    env: &EnvSource,
) -> u64 {
    let backoff = if consecutive_failures > 0 {
        time::calculate_account_backoff(
            i64::from(consecutive_failures.saturating_sub(1)),
            None,
            60_000,
            random_fraction,
            env,
        )
    } else {
        0
    };
    let delay = if delay_max_ms > 0 {
        time::pick_pause_ms(delay_min_ms as f64, delay_max_ms as f64, random_fraction)
    } else {
        0
    };
    time::compose_account_wait_ms(backoff as f64, delay as f64)
}

/// Extrai o `unified_report` do stdout de um filho (logs + JSON no mesmo fluxo).
#[must_use]
pub fn extract_report(stdout: &str) -> Option<Value> {
    let marker = "\"type\": \"unified_report\"";
    let marker_at = stdout.find(marker)?;
    // Candidato: último `{` no início de linha antes do marcador.
    let start = stdout[..marker_at].rfind("\n{").map(|index| index + 1)?;
    let end = json_object_end(stdout, start)?;
    let parsed: Value = serde_json::from_str(&stdout[start..end]).ok()?;
    (parsed.get("type").and_then(Value::as_str) == Some("unified_report")).then_some(parsed)
}

/// Fim do objeto JSON iniciado em `start` (contando chaves, ciente de strings).
fn json_object_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes.iter().enumerate().skip(start) {
        let character = *byte;
        if in_string {
            if escaped {
                escaped = false;
            } else if character == b'\\' {
                escaped = true;
            } else if character == b'"' {
                in_string = false;
            }
            continue;
        }
        match character {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn random_fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos());
    f64::from(nanos) / 1_000_000_000.0
}

/// Executa o filho `all` para uma conta (produção).
fn run_account_child(account: &Account, force: bool) -> AccountExecution {
    let start = Utc::now();
    // Lido antes do filho: o flag de sessão importada é do estado inicial da conta.
    let meta_imported =
        ali_coins_core::session::session_meta_is_imported(&account.session_meta_path);
    let mut command = match std::env::current_exe() {
        Ok(exe) => Command::new(exe),
        Err(error) => {
            return AccountExecution {
                user: account.user.clone(),
                masked_user: account.masked_user.clone(),
                exit_code: 1,
                error: Some(format!("Falha ao localizar o executável: {error}")),
                start_time: start,
                end_time: Utc::now(),
                duration: "0s".to_string(),
                ..AccountExecution::default()
            };
        }
    };
    command
        .arg("all")
        .arg("--account")
        .arg(&account.user)
        .arg("--json");
    if force {
        command.arg("--force");
    }
    let output = command.output();
    let end = Utc::now();
    let duration = time::format_duration((end - start).num_milliseconds());
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(1);
            let payload = extract_report(&stdout);
            let error = if exit_code == 0 || exit_code == 2 {
                None
            } else if exit_code == 3 {
                Some("Lock ativo por outro processo".to_string())
            } else {
                let snippet = if stderr.trim().is_empty() {
                    stdout
                } else {
                    stderr
                };
                Some(
                    snippet
                        .trim()
                        .lines()
                        .last()
                        .unwrap_or("Falha na conta")
                        .to_string(),
                )
            };
            AccountExecution {
                user: account.user.clone(),
                masked_user: account.masked_user.clone(),
                exit_code,
                error,
                payload,
                meta_imported,
                start_time: start,
                end_time: end,
                duration,
            }
        }
        Err(error) => AccountExecution {
            user: account.user.clone(),
            masked_user: account.masked_user.clone(),
            exit_code: 1,
            error: Some(format!("Falha ao executar a conta: {error}")),
            payload: None,
            meta_imported,
            start_time: start,
            end_time: end,
            duration,
        },
    }
}

/// Loop sequencial com runner injetável (testes usam fakes; produção usa o filho).
pub fn run_accounts<F>(
    accounts: &[Account],
    env: &EnvSource,
    delay_min_ms: u64,
    delay_max_ms: u64,
    mut runner: F,
) -> Vec<AccountExecution>
where
    F: FnMut(&Account) -> AccountExecution,
{
    let mut executions = Vec::with_capacity(accounts.len());
    let mut consecutive_failures = 0_u32;
    for (index, account) in accounts.iter().enumerate() {
        logging::global().info(
            &format!(
                "\n>>> [CONTA {}/{}] Iniciando execução para: {}",
                index + 1,
                accounts.len(),
                account.masked_user
            ),
            &[],
        );
        let execution = runner(account);
        if execution.error.is_some() || execution.exit_code == 1 {
            consecutive_failures += 1;
        } else if execution.exit_code != 3 {
            consecutive_failures = 0;
        }
        if index + 1 < accounts.len() {
            let wait = account_wait_ms(
                consecutive_failures,
                delay_min_ms,
                delay_max_ms,
                random_fraction(),
                env,
            );
            if wait > 0 {
                logging::global().info(
                    &format!("Aguardando {wait}ms antes da próxima conta..."),
                    &[],
                );
                std::thread::sleep(Duration::from_millis(wait));
            }
        }
        executions.push(execution);
    }
    executions
}

/// Monta o payload `multi_account_report` a partir das execuções.
#[must_use]
pub fn build_multi_payload(
    executions: &[AccountExecution],
    main_start: DateTime<Utc>,
    main_end: DateTime<Utc>,
) -> Value {
    let results: Vec<AccountResultInput> = executions
        .iter()
        .map(|execution| {
            let checkin_result = execution
                .payload
                .as_ref()
                .and_then(|payload| payload.get("checkin"))
                .filter(|value| !value.is_null())
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            let tasks_result = execution
                .payload
                .as_ref()
                .and_then(|payload| payload.get("tasks"))
                .filter(|value| !value.is_null())
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            AccountResultInput {
                account: Some(AccountRef {
                    masked_user: Some(execution.masked_user.clone()),
                }),
                user: Some(execution.user.clone()),
                checkin_result,
                tasks_result,
                error: execution.error.clone(),
                is_imported_session_expired: Some(detect_imported_session_expired(
                    execution.error.as_deref(),
                    execution.meta_imported,
                )),
                start_time: Some(execution.start_time.to_rfc3339()),
                end_time: Some(execution.end_time.to_rfc3339()),
                duration: Some(execution.duration.clone()),
                ..AccountResultInput::default()
            }
        })
        .collect();
    let total_duration = time::format_duration((main_end - main_start).num_milliseconds());
    build_multi_account_report_payload(
        &results,
        &MultiAccountMeta {
            total_duration: Some(&total_duration),
            main_start_time: Some(main_start),
            main_end_time: Some(main_end),
            _marker: std::marker::PhantomData,
        },
    )
}

/// `ali-coins all --all [--json] [--force]` em modo multi-conta.
pub fn run(args: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode as StdExitCode;
    let json = args.iter().any(|arg| arg == "--json");
    let force = args.iter().any(|arg| arg == "--force" || arg == "-f");
    let Some(crate::context::CliContext {
        env,
        config,
        accounts,
        ..
    }) = bootstrap()
    else {
        return StdExitCode::from(1);
    };
    if accounts.is_empty() {
        logging::global().error("Nenhuma conta configurada para o modo multi-conta.", &[]);
        return StdExitCode::from(1);
    }

    let main_start = Utc::now();
    let executions = run_accounts(
        &accounts,
        &env,
        config.account_delay_min_ms,
        config.account_delay_max_ms,
        |account| run_account_child(account, force),
    );
    let main_end = Utc::now();
    let payload = build_multi_payload(&executions, main_start, main_end);
    let flags = MultiFlags::from_executions(&executions);
    let event = flags.event();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        let summary = payload
            .get("meta")
            .and_then(|meta| meta.get("successfulAccounts"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        logging::global().info(
            &format!(
                "Multi-conta: {summary}/{} contas com sucesso | evento {event:?}",
                accounts.len()
            ),
            &[],
        );
    }

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Falha ao iniciar o runtime tokio: {error}");
            return StdExitCode::from(1);
        }
    };
    runtime.block_on(async {
        // Notificação consolidada + heartbeat (best-effort).
        if config.heartbeat_enabled && !config.heartbeat_url.trim().is_empty() {
            let timeout = Duration::from_millis(config.heartbeat_timeout_ms);
            if let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) {
                let heartbeat = ali_coins_core::notify::HeartbeatConfig {
                    enabled: true,
                    url: config.heartbeat_url.clone(),
                    timeout_ms: config.heartbeat_timeout_ms,
                };
                let host = crate::run_all::notify_host(&config);
                let action = if matches!(
                    event,
                    TelegramEvent::LockActive
                        | TelegramEvent::StreakBreak
                        | TelegramEvent::TwoFactorRequired
                        | TelegramEvent::Failure
                ) {
                    HeartbeatAction::Fail
                } else {
                    HeartbeatAction::Success
                };
                let payload_text = serde_json::to_string(&payload).unwrap_or_default();
                let _ = ali_coins_core::notify::send_heartbeat(
                    &client,
                    action,
                    &heartbeat,
                    &host,
                    Some(&payload_text),
                )
                .await;
            }
        }
        if config.telegram_enabled {
            let timeout = Duration::from_millis(config.telegram_timeout_ms);
            if let Ok(client) = SafeHttpClient::new(config.allow_private_webhooks, timeout) {
                let host = crate::run_all::notify_host(&config);
                let chat_id = accounts[0]
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
                let error_text = executions
                    .iter()
                    .find_map(|execution| execution.error.as_deref());
                let message = match event {
                    TelegramEvent::LockActive
                    | TelegramEvent::StreakBreak
                    | TelegramEvent::TwoFactorRequired => {
                        let context = TelegramContext {
                            user: None,
                            error: error_text,
                            host: Some(host.as_str()),
                            version: Some(env!("CARGO_PKG_VERSION")),
                            ..TelegramContext::default()
                        };
                        build_message(event, &context)
                    }
                    _ => build_multi_account_message_at(
                        &payload,
                        event,
                        error_text,
                        &host,
                        env!("CARGO_PKG_VERSION"),
                        Utc::now(),
                    ),
                };
                let _ = send_telegram(&client, &telegram_config, &message).await;
            }
        }
    });

    let code = flags.exit_code();
    StdExitCode::from(u8::try_from(code.as_i32()).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution(user: &str, exit_code: i32, error: Option<&str>) -> AccountExecution {
        AccountExecution {
            user: user.to_string(),
            masked_user: user.replace("user", "***"),
            exit_code,
            error: error.map(str::to_string),
            ..AccountExecution::default()
        }
    }

    #[test]
    fn flags_e_exit_codes_agregados() {
        let success = vec![execution("user1", 0, None), execution("user2", 2, None)];
        let flags = MultiFlags::from_executions(&success);
        assert!(flags.any_success && flags.any_new_action);
        assert_eq!(flags.event(), TelegramEvent::Success);
        assert_eq!(flags.exit_code(), ExitCode::Success);

        let already = vec![execution("user1", 2, None)];
        let flags = MultiFlags::from_executions(&already);
        assert_eq!(flags.event(), TelegramEvent::AlreadyCollected);
        assert_eq!(flags.exit_code(), ExitCode::NoAction);

        let locked = vec![execution("user1", 3, Some("lock"))];
        let flags = MultiFlags::from_executions(&locked);
        assert!(flags.all_accounts_locked());
        assert_eq!(flags.event(), TelegramEvent::LockActive);
        assert_eq!(flags.exit_code(), ExitCode::LockActive);

        let broken = vec![execution("user1", 0, None), execution("user2", 4, None)];
        let flags = MultiFlags::from_executions(&broken);
        assert_eq!(flags.event(), TelegramEvent::StreakBreak);
        assert_eq!(flags.exit_code(), ExitCode::StreakBroken);

        let failed = vec![
            execution("user1", 0, None),
            execution("user2", 1, Some("erro")),
        ];
        let flags = MultiFlags::from_executions(&failed);
        assert_eq!(flags.event(), TelegramEvent::Failure);
        assert_eq!(flags.exit_code(), ExitCode::Failure);

        let two_factor = vec![execution("user1", 5, Some("2FA"))];
        let flags = MultiFlags::from_executions(&two_factor);
        assert_eq!(flags.event(), TelegramEvent::TwoFactorRequired);
        assert_eq!(flags.exit_code(), ExitCode::TwoFactor);
    }

    #[test]
    fn executa_contas_em_sequencia_com_runner_fake() {
        let env = EnvSource::from_pairs([("ACCOUNT_BACKOFF_BASE_MS", "0")]);
        let accounts: Vec<Account> = ["a@example.com", "b@example.com", "c@example.com"]
            .iter()
            .enumerate()
            .map(|(index, user)| Account {
                index,
                user: (*user).to_string(),
                masked_user: format!("***{index}"),
                ..Account::default()
            })
            .collect();
        let mut order = Vec::new();
        let executions = run_accounts(&accounts, &env, 0, 0, |account| {
            order.push(account.user.clone());
            execution(&account.user, 0, None)
        });
        assert_eq!(
            order,
            vec!["a@example.com", "b@example.com", "c@example.com"]
        );
        assert_eq!(executions.len(), 3);
    }

    #[test]
    fn extrai_relatorio_do_stdout() {
        let stdout = concat!(
            "{\"level\":30,\"msg\":\"log\"}\n",
            "{\n  \"checkin\": {\"alreadyCollected\": true},\n  \"type\": \"unified_report\",\n  \"user\": \"x@y\"\n}\n",
            "{\"level\":30,\"msg\":\"fim\"}\n"
        );
        let report = extract_report(stdout).expect("payload");
        assert_eq!(report["type"], "unified_report");
        assert_eq!(report["user"], "x@y");
        assert!(extract_report("sem payload").is_none());
    }

    #[test]
    fn payload_multi_agrega_contas() {
        let start = Utc::now();
        let end = start + chrono::Duration::seconds(30);
        let executions = vec![
            execution("a@example.com", 0, None),
            execution("b@example.com", 1, Some("falha")),
        ];
        let payload = build_multi_payload(&executions, start, end);
        assert_eq!(payload["type"], "multi_account_report");
        assert_eq!(payload["meta"]["totalAccounts"], 2);
        assert_eq!(payload["meta"]["successfulAccounts"], 1);
        assert_eq!(payload["accounts"][1]["error"], "falha");
    }

    #[test]
    fn payload_multi_marca_sessao_importada_expirada() {
        let start = Utc::now();
        let imported = AccountExecution {
            error: Some(
                "Erro ao efetuar o login: não foi possível obter streak e saldo".to_string(),
            ),
            meta_imported: true,
            ..execution("a@example.com", 1, None)
        };
        let sem_meta = execution("b@example.com", 1, Some("Navigation timeout"));
        let payload = build_multi_payload(&[imported, sem_meta], start, start);
        assert_eq!(payload["accounts"][0]["isImportedSessionExpired"], true);
        assert_eq!(payload["accounts"][1]["isImportedSessionExpired"], false);
    }
}
