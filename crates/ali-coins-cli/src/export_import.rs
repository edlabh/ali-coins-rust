//! Subcomandos `export-session` e `import-session` (equivalente às CLIs do oráculo).

use crate::context::bootstrap;
use ali_coins_core::secure_fs::safe_write_file;
use ali_coins_core::session::{
    SessionOptions, export_session_token, import_session_token, migrate_legacy_session,
    rotate_session_secret,
};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAX_TOKEN_BYTES: u64 = 2 * 1024 * 1024;

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

/// Valor de um argumento no formato `--flag=valor`.
fn flag_prefix<'a>(args: &'a [String], prefix: &str) -> Option<&'a str> {
    args.iter().find_map(|arg| arg.strip_prefix(prefix))
}

/// Seleção de contas para `--rotate`/`--migrate` (padrão: todas as contas).
fn selected_accounts<'a>(
    args: &[String],
    accounts: &'a [ali_coins_core::config::Account],
) -> Result<Vec<&'a ali_coins_core::config::Account>, ExitCode> {
    if let Some(selector) = flag_value(args, "--account") {
        if let Some(account) = resolve_account(accounts, selector) {
            return Ok(vec![account]);
        }
        eprintln!("Conta '{selector}' não encontrada.");
        return Err(ExitCode::from(1));
    }
    Ok(accounts.iter().collect())
}

fn resolve_account<'a>(
    accounts: &'a [ali_coins_core::config::Account],
    selector: &str,
) -> Option<&'a ali_coins_core::config::Account> {
    if let Ok(index) = selector.parse::<usize>() {
        return accounts.iter().find(|account| account.index == index);
    }
    accounts
        .iter()
        .find(|account| account.user.eq_ignore_ascii_case(selector))
}

fn account_options(
    base_dir: &Path,
    account: &ali_coins_core::config::Account,
    plaintext: bool,
) -> SessionOptions {
    let mut options = SessionOptions::with_base_dir(base_dir.to_path_buf());
    options.session_path = Some(account.session_path.clone());
    if plaintext {
        options.encrypt_local_session = Some(false);
    }
    options
}

/// `ali-coins export-session [--all] [--account <id>] [--show-token]`
pub fn run_export(args: &[String]) -> ExitCode {
    if has_flag(args, "--rotate") {
        return run_rotate(args);
    }
    let Some(ctx) = bootstrap() else {
        return ExitCode::from(1);
    };
    run_export_with_context(args, ctx)
}

/// Núcleo do `export-session` (contexto injetável nos testes).
pub(crate) fn run_export_with_context(
    args: &[String],
    ctx: crate::context::CliContext,
) -> ExitCode {
    let crate::context::CliContext {
        base_dir,
        env,
        config: _config,
        accounts,
    } = ctx;
    let show_token = has_flag(args, "--show-token");

    let selected: Vec<&ali_coins_core::config::Account> = if has_flag(args, "--all") {
        accounts
            .iter()
            .filter(|account| {
                account.session_path.exists()
                    || PathBuf::from(format!("{}.enc", account.session_path.to_string_lossy()))
                        .exists()
            })
            .collect()
    } else if let Some(selector) = flag_value(args, "--account") {
        if let Some(account) = resolve_account(&accounts, selector) {
            vec![account]
        } else {
            eprintln!("Conta '{selector}' não encontrada.");
            return ExitCode::from(1);
        }
    } else {
        vec![&accounts[0]]
    };

    let mut exported = 0;
    for account in selected {
        let options = account_options(&base_dir, account, false);
        match export_session_token(&options, &base_dir, &env, &account.user) {
            Ok(token) => {
                let file_name = if account.index == 1 {
                    "session_token.txt".to_string()
                } else {
                    format!("session_token_{}.txt", account.index)
                };
                let file_path = base_dir.join(&file_name);
                if let Err(error) = safe_write_file(&file_path, token.as_bytes()) {
                    eprintln!(
                        "Conta {}: falha ao gravar {file_name}: {error}",
                        account.masked_user
                    );
                    continue;
                }
                println!(
                    "Conta {}: token exportado em {file_name} (0o600).",
                    account.masked_user
                );
                if show_token {
                    println!("{token}");
                }
                exported += 1;
            }
            Err(error) => {
                eprintln!("Conta {}: {error}", account.masked_user);
            }
        }
    }

    if exported > 0 {
        ExitCode::SUCCESS
    } else {
        eprintln!("Nenhuma sessão exportada.");
        ExitCode::from(1)
    }
}

/// `ali-coins export-session --rotate [--all] [--account <id>] [--new-secret-from-env=VAR]`
fn run_rotate(args: &[String]) -> ExitCode {
    let Some(ctx) = bootstrap() else {
        return ExitCode::from(1);
    };
    run_rotate_with_context(args, ctx)
}

/// Núcleo do `--rotate` (contexto injetável nos testes).
pub(crate) fn run_rotate_with_context(
    args: &[String],
    ctx: crate::context::CliContext,
) -> ExitCode {
    let crate::context::CliContext {
        base_dir,
        env,
        config: _config,
        accounts,
    } = ctx;
    let selected = match selected_accounts(args, &accounts) {
        Ok(selected) => selected,
        Err(code) => return code,
    };

    let new_var = flag_prefix(args, "--new-secret-from-env=")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("SESSION_SECRET_NEW");
    let old_secret = env
        .get("SESSION_SECRET_OLD")
        .or_else(|| env.get("SESSION_SECRET"))
        .map(str::to_string);
    let new_secret = env.get(new_var).map(str::to_string).or_else(|| {
        env.get("SESSION_SECRET_OLD")
            .and_then(|_| env.get("SESSION_SECRET").map(str::to_string))
    });

    let mut rotated = 0_u32;
    let mut failed = 0_u32;
    for account in selected {
        let enc_path = PathBuf::from(format!("{}.enc", account.session_path.display()));
        if !account.session_path.exists() && !enc_path.exists() {
            ali_coins_core::logging::global().warn(
                &format!(
                    "[{}] Nenhuma sessão ativa encontrada. Pulando...",
                    account.masked_user
                ),
                &[],
            );
            continue;
        }
        let mut options = account_options(&base_dir, account, false);
        options.old_secret.clone_from(&old_secret);
        options.new_secret.clone_from(&new_secret);
        match rotate_session_secret(&options, &base_dir, &env) {
            Ok(outcome) => {
                rotated += 1;
                ali_coins_core::logging::global().info(
                    &format!(
                        "[{}] chave rotacionada (backup: {})",
                        account.masked_user,
                        outcome.backup_path.display()
                    ),
                    &[],
                );
            }
            Err(error) => {
                failed += 1;
                ali_coins_core::logging::global().error(
                    &format!("[{}] falha na rotação: {error}", account.masked_user),
                    &[],
                );
            }
        }
    }

    if rotated == 0 {
        ali_coins_core::logging::global().error("Nenhuma conta foi rotacionada com sucesso.", &[]);
        return ExitCode::from(1);
    }
    if failed > 0 {
        ali_coins_core::logging::global().warn(
            "[PARCIAL] Rotação concluída com falhas em algumas contas.",
            &[],
        );
        return ExitCode::from(1);
    }
    ali_coins_core::logging::global().info(
        "[SUCESSO] Rotação de chave concluída para todas as contas.",
        &[],
    );
    ExitCode::SUCCESS
}

/// `ali-coins import-session --migrate [--all] [--account <id>] [--json]`
fn run_migrate(args: &[String]) -> ExitCode {
    let Some(ctx) = bootstrap() else {
        return ExitCode::from(1);
    };
    run_migrate_with_context(args, ctx)
}

/// Núcleo do `--migrate` (contexto injetável nos testes).
pub(crate) fn run_migrate_with_context(
    args: &[String],
    ctx: crate::context::CliContext,
) -> ExitCode {
    let crate::context::CliContext {
        base_dir,
        env,
        config: _config,
        accounts,
    } = ctx;
    let json = has_flag(args, "--json");
    let selected = match selected_accounts(args, &accounts) {
        Ok(selected) => selected,
        Err(code) => return code,
    };

    let mut migrated = 0_u32;
    let mut results: Vec<serde_json::Value> = Vec::new();
    for account in selected {
        let options = account_options(&base_dir, account, false);
        match migrate_legacy_session(&options, &base_dir, &env) {
            Ok(outcome) => {
                migrated += 1;
                results.push(serde_json::json!({
                    "user": account.masked_user,
                    "cookiesCount": outcome.cookies_count,
                    "migrated": outcome.migrated,
                    "encrypted": outcome.encrypted,
                    "sessionPath": account.session_path,
                }));
            }
            Err(error) => {
                results.push(serde_json::json!({
                    "user": account.masked_user,
                    "migrated": false,
                    "error": error.to_string(),
                    "sessionPath": account.session_path,
                }));
                ali_coins_core::logging::global().error(
                    &format!("[{}] falha na migração: {error}", account.masked_user),
                    &[],
                );
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string())
        );
    } else if migrated == 0 {
        ali_coins_core::logging::global().error(
            "Nenhuma sessão legada foi migrada (verifique session.json e ENCRYPT_LOCAL_SESSION).",
            &[],
        );
    } else {
        ali_coins_core::logging::global().info(
            &format!("{migrated} sessão(ões) legada(s) migrada(s) para .enc."),
            &[],
        );
    }

    if migrated == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// `ali-coins import-session [--all] [--account <id>] [--from-file <path>] [--plaintext] [--keep-tokens]`
pub fn run_import(args: &[String]) -> ExitCode {
    if has_flag(args, "--migrate") {
        return run_migrate(args);
    }
    let Some(ctx) = bootstrap() else {
        return ExitCode::from(1);
    };
    run_import_with_context(args, ctx)
}

/// Núcleo do `import-session` (contexto injetável nos testes).
pub(crate) fn run_import_with_context(
    args: &[String],
    ctx: crate::context::CliContext,
) -> ExitCode {
    let crate::context::CliContext {
        base_dir,
        env,
        config: _config,
        accounts,
    } = ctx;
    let plaintext = has_flag(args, "--plaintext");
    let keep_tokens = has_flag(args, "--keep-tokens");
    let expected_user = flag_value(args, "--account")
        .map(|selector| resolve_account(&accounts, selector).map(|account| account.user.clone()));
    let expected_user = match expected_user {
        Some(Some(user)) => Some(user),
        Some(None) => {
            eprintln!(
                "Conta '{}' não encontrada.",
                flag_value(args, "--account").unwrap_or_default()
            );
            return ExitCode::from(1);
        }
        None => None,
    };

    if has_flag(args, "--all") {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&base_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.file_name().is_some_and(|name| {
                            let name = name.to_string_lossy();
                            name.starts_with("session_token") && name.ends_with(".txt")
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();

        let mut imported = 0;
        for file in files {
            match read_token_file(&file) {
                Ok(token) => {
                    let options = import_options(&base_dir, &accounts, plaintext);
                    match import_session_token(
                        &options,
                        &base_dir,
                        &env,
                        &token,
                        expected_user.as_deref(),
                        &accounts,
                    ) {
                        Ok(outcome) => {
                            println!(
                                "Importada sessão de {} ({}).",
                                ali_coins_core::config::mask_user(&outcome.user),
                                if outcome.encrypted {
                                    "cifrada"
                                } else {
                                    "texto puro"
                                }
                            );
                            if !keep_tokens {
                                let _ = std::fs::remove_file(&file);
                            }
                            imported += 1;
                        }
                        Err(error) => eprintln!("Falha ao importar {}: {error}", file.display()),
                    }
                }
                Err(error) => eprintln!("{error}"),
            }
        }
        return if imported > 0 {
            ExitCode::SUCCESS
        } else {
            eprintln!("Nenhuma sessão importada.");
            ExitCode::from(1)
        };
    }

    let token = if let Some(path) = flag_value(args, "--from-file") {
        match read_token_file(Path::new(path)) {
            Ok(token) => token,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(1);
            }
        }
    } else {
        let mut buffer = Vec::new();
        if let Err(error) = std::io::stdin()
            .take(MAX_TOKEN_BYTES + 1)
            .read_to_end(&mut buffer)
        {
            eprintln!("Falha ao ler o token da entrada padrão: {error}");
            return ExitCode::from(1);
        }
        if buffer.len() as u64 > MAX_TOKEN_BYTES {
            eprintln!("Token acima do limite de 2 MiB.");
            return ExitCode::from(1);
        }
        String::from_utf8_lossy(&buffer).to_string()
    };

    let options = import_options(&base_dir, &accounts, plaintext);
    match import_session_token(
        &options,
        &base_dir,
        &env,
        &token,
        expected_user.as_deref(),
        &accounts,
    ) {
        Ok(outcome) => {
            println!(
                "Sessão importada para {} ({}).",
                ali_coins_core::config::mask_user(&outcome.user),
                outcome.session_path.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Falha ao importar sessão: {error}");
            ExitCode::from(1)
        }
    }
}

fn import_options(
    base_dir: &Path,
    accounts: &[ali_coins_core::config::Account],
    plaintext: bool,
) -> SessionOptions {
    // A sessão é gravada no caminho da conta resolvida pelo token; quando o
    // auto-roteamento escolher outra conta, o `save_session` usa o caminho padrão
    // (session.json) — comportamento equivalente ao importador do oráculo.
    let mut options = SessionOptions::with_base_dir(base_dir.to_path_buf());
    if let Some(first) = accounts.first() {
        options.session_path = Some(first.session_path.clone());
    }
    if plaintext {
        options.encrypt_local_session = Some(false);
    }
    options
}

fn read_token_file(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Falha ao ler {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} não é um arquivo regular.", path.display()));
    }
    if metadata.len() > MAX_TOKEN_BYTES {
        return Err(format!("{} excede o limite de 2 MiB.", path.display()));
    }
    std::fs::read_to_string(path)
        .map_err(|error| format!("Falha ao ler {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_core::config::EnvSource;
    use ali_coins_core::session::{
        SessionOptions, save_session, storage_filter::storage_state_json,
    };

    const SECRET: &str = "parity-test-secret-0123456789abcdef";
    const USER: &str = "fulano@example.com";

    fn ctx(dir: &std::path::Path) -> crate::context::CliContext {
        crate::context::context_from(
            dir.to_path_buf(),
            EnvSource::from_pairs([
                ("ALI_USER", USER),
                ("ALI_PASSWORD", "senha"),
                ("SESSION_SECRET", SECRET),
                ("TELEGRAM_ENABLED", "false"),
            ]),
        )
        .expect("contexto")
    }

    fn grava_sessao(dir: &std::path::Path) {
        let options = SessionOptions::with_base_dir(dir.to_path_buf());
        let env = EnvSource::from_pairs([("SESSION_SECRET", SECRET)]);
        save_session(
            &options,
            dir,
            &env,
            storage_state_json(&[("xman_us_t", "auth-value")], &[]),
            USER,
        )
        .expect("save")
        .expect("gravou");
    }

    #[test]
    fn export_sem_sessao_falha() {
        let dir = tempfile::tempdir().expect("tempdir");
        let code = run_export_with_context(&[], ctx(dir.path()));
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn export_e_import_roundtrip_pelo_cli() {
        let dir_a = tempfile::tempdir().expect("dir a");
        grava_sessao(dir_a.path());
        let code = run_export_with_context(&["--show-token".to_string()], ctx(dir_a.path()));
        assert_eq!(code, ExitCode::SUCCESS);
        let token_path = dir_a.path().join("session_token.txt");
        assert!(token_path.exists(), "token exportado");

        let dir_b = tempfile::tempdir().expect("dir b");
        let code = run_import_with_context(
            &[
                "--from-file".to_string(),
                token_path.to_string_lossy().to_string(),
            ],
            ctx(dir_b.path()),
        );
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(dir_b.path().join("session.json.enc").exists());
    }

    #[test]
    fn import_token_invalido_falha() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bad = dir.path().join("bad.txt");
        std::fs::write(&bad, "v3:token-que-nao-decifra").expect("token");
        let code = run_import_with_context(
            &["--from-file".to_string(), bad.to_string_lossy().to_string()],
            ctx(dir.path()),
        );
        assert_eq!(code, ExitCode::from(1));
    }
}
