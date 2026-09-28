//! Subcomandos `export-session` e `import-session` (equivalente às CLIs do oráculo).

use ali_coins_core::config::{Config, EnvSource, load_accounts};
use ali_coins_core::secure_fs::safe_write_file;
use ali_coins_core::session::{SessionOptions, export_session_token, import_session_token};
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

pub(crate) fn bootstrap() -> Option<(
    PathBuf,
    EnvSource,
    Config,
    Vec<ali_coins_core::config::Account>,
)> {
    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let credentials = base_dir.join("credentials.env");
    if credentials.exists() {
        let _ = dotenvy::from_path(&credentials);
    }
    let env = EnvSource::from_current_process();
    let config = match Config::load(&env, &base_dir, true, None, None) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Falha na validação das variáveis de configuração em credentials.env:");
            for issue in error.issues() {
                eprintln!(" • {}: {}", issue.path, issue.message);
            }
            return None;
        }
    };
    let accounts = load_accounts(&env, &base_dir);
    if accounts.is_empty() {
        eprintln!("Nenhuma conta configurada (ALI_USER/ALI_USER_N ou accounts.json).");
        return None;
    }
    Some((base_dir, env, config, accounts))
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
    let Some((base_dir, env, _config, accounts)) = bootstrap() else {
        return ExitCode::from(1);
    };
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

/// `ali-coins import-session [--all] [--account <id>] [--from-file <path>] [--plaintext] [--keep-tokens]`
pub fn run_import(args: &[String]) -> ExitCode {
    let Some((base_dir, env, _config, accounts)) = bootstrap() else {
        return ExitCode::from(1);
    };
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
