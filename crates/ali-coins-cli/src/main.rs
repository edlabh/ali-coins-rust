//! Binário `ali-coins` (Rust).
//!
//! Fase 1: `--dry-run [--json]` com paridade de contrato C-02/C-03/C-04.
//! Os demais subcomandos entram nas próximas fases (ver `docs/03-roadmap.md`).

#![forbid(unsafe_code)]

mod checkin_parity;
mod export_import;
mod notify_test;
mod run_all;
mod run_checkin;
mod run_tasks;

use ali_coins_core::config::{
    Config, DryRunSummary, EnvSource, load_accounts, mask_chat_id, mask_user,
};
use ali_coins_core::{exit, logging};
use clap::{Arg, ArgAction, Command};
use std::path::PathBuf;
use std::process::ExitCode;

fn command() -> Command {
    Command::new("ali-coins")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Automação de check-in e tarefas de moedas do AliExpress (port Rust).")
        // O oráculo usa `allowUnknownOption(true)`: mantém tolerância a flags futuras.
        .ignore_errors(true)
        .arg(
            Arg::new("dry-run")
                .short('d')
                .long("dry-run")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
        .arg(Arg::new("notify").long("notify").action(ArgAction::SetTrue))
        .arg(
            Arg::new("no-notify")
                .long("no-notify")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("heartbeat").long("heartbeat").action(ArgAction::SetTrue))
        .arg(
            Arg::new("no-heartbeat")
                .long("no-heartbeat")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("force").short('f').long("force").action(ArgAction::SetTrue))
        .arg(
            Arg::new("no-delay")
                .long("no-delay")
                .action(ArgAction::SetTrue),
        )
}

/// Último `--positivo`/`--negativo` vence (semântica do commander).
fn last_boolean_flag(args: &[String], positive: &str, negative: &str) -> Option<bool> {
    let mut result = None;
    for arg in args {
        if arg == positive {
            result = Some(true);
        } else if arg == negative {
            result = Some(false);
        }
    }
    result
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();

    // Subcomandos de sessão (compatíveis com os scripts do oráculo).
    match raw.get(1).map(String::as_str) {
        Some("export-session") => return export_import::run_export(&raw[2..]),
        Some("import-session") => return export_import::run_import(&raw[2..]),
        Some("all") => {
            if raw.iter().any(|arg| arg == "--dry-run") {
                return run_dry_run(&raw, raw.iter().any(|arg| arg == "--json"));
            }
            return run_all::run(&raw[2..]);
        }
        Some("checkin") => {
            if raw.iter().any(|arg| arg == "--dry-run") {
                return run_dry_run(&raw, raw.iter().any(|arg| arg == "--json"));
            }
            return run_checkin::run(&raw[2..]);
        }
        Some("notify-test") => return notify_test::run(&raw[2..]),
        Some("tasks") => {
            if raw.iter().any(|arg| arg == "--dry-run") {
                return run_dry_run(&raw, raw.iter().any(|arg| arg == "--json"));
            }
            return run_tasks::run(&raw[2..]);
        }
        _ => {}
    }

    let matches = command().get_matches_from(&raw);
    let json_mode = matches.get_flag("json");

    // Crash handler global: exit code 6 com cleanup best-effort.
    exit::install_panic_handler(|| {});

    if !matches.get_flag("dry-run") {
        eprintln!(
            "ali-coins-rust {} — use --dry-run, all, checkin, tasks, export-session ou import-session.",
            env!("CARGO_PKG_VERSION")
        );
        return ExitCode::from(1);
    }

    run_dry_run(&raw, json_mode)
}

/// `--dry-run [--json]` (também aceito após subcomandos: `all --dry-run`).
pub(crate) fn run_dry_run(raw: &[String], json_mode: bool) -> ExitCode {
    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // Mesma ordem do oráculo: credentials.env (sem sobrescrever env já definidas).
    let credentials = base_dir.join("credentials.env");
    if credentials.exists() {
        let _ = dotenvy::from_path(&credentials);
    }
    let env = EnvSource::from_current_process();

    let notify = last_boolean_flag(raw, "--notify", "--no-notify");
    let heartbeat = last_boolean_flag(raw, "--heartbeat", "--no-heartbeat");

    let config = match Config::load(&env, &base_dir, true, notify, heartbeat) {
        Ok(config) => config,
        Err(error) => {
            let details = error
                .issues()
                .iter()
                .map(|issue| format!(" • {}: {}", issue.path, issue.message))
                .collect::<Vec<_>>()
                .join("\n");
            logging::global().error(
                &format!("Falha na validação do arquivo credentials.env:\n{details}"),
                &[],
            );
            return ExitCode::from(1);
        }
    };
    let _ = logging::init(config.log_level, json_mode);

    let accounts = load_accounts(&env, &base_dir);

    if json_mode {
        println!(
            "{}",
            DryRunSummary::build(&config, &accounts).to_pretty_json()
        );
    } else {
        print_human_dry_run(&config, &accounts);
    }
    ExitCode::SUCCESS
}

fn print_human_dry_run(config: &Config, accounts: &[ali_coins_core::config::Account]) {
    println!("===============================================================");
    println!("                 MODO DE VALIDAÇÃO (DRY-RUN)");
    println!("===============================================================");
    println!("Configuração validada com sucesso:");
    if accounts.len() > 1 {
        println!(" • Contas detectadas ({}):", accounts.len());
        for account in accounts {
            println!("    - [Conta {}] {}", account.index, account.masked_user);
        }
    } else {
        println!(" • Usuário: {}", mask_user(&config.ali_user));
        println!(" • Senha: [CONFIGURADA]");
    }
    println!(" • Bloqueio de mídia (ALLOW_MEDIA): {}", config.allow_media);
    println!(" • Modo Headless: {}", config.headless);
    println!(" • Nível de Log: {}", config.log_level.as_str());
    println!(
        " • Cooldown pós-captcha (CAPTCHA_COOLDOWN_HOURS): {}",
        if config.captcha_cooldown_hours > 0 {
            format!("{}h", config.captcha_cooldown_hours)
        } else {
            "desligado".to_string()
        }
    );
    println!(
        " • Sandbox Chromium: {}",
        if config.no_sandbox {
            "Desativado (--no-sandbox)"
        } else {
            "Ativado"
        }
    );
    println!(
        " • Timeout de Navegação (NAV_TIMEOUT): {}ms",
        config.nav_timeout
    );
    println!(
        " • SESSION_SECRET: {}",
        if config.session_secret.is_some() {
            "[CONFIGURADO]"
        } else {
            "[NÃO CONFIGURADO]"
        }
    );
    println!(
        " • Webhook de notificação: {}",
        if config.notify_webhook_url.is_empty() {
            "[NÃO CONFIGURADO]".to_string()
        } else if config.allow_private_webhooks {
            "[CONFIGURADO] (ALLOW_PRIVATE_WEBHOOKS ativo)".to_string()
        } else {
            "[CONFIGURADO]".to_string()
        }
    );
    println!(
        " • Notificações Telegram: {}",
        if config.telegram_enabled {
            format!(
                "Ativado (Chat ID: {}, Token: [CONFIGURADO])",
                mask_chat_id(&config.telegram_chat_id)
            )
        } else {
            "Desativado".to_string()
        }
    );
    println!(
        " • Dead Man's Switch (Heartbeat): {}",
        if config.heartbeat_enabled {
            format!("Ativado (timeout: {}ms)", config.heartbeat_timeout_ms)
        } else {
            "Desativado".to_string()
        }
    );
    println!("===============================================================");
}
