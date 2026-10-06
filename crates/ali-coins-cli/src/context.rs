//! Contexto de execução da CLI (bootstrap testável).
//!
//! `context_from` resolve a partir de um diretório/ambiente já prontos (sem
//! estado global — usado pelos testes); `bootstrap` é o caminho de produção:
//! diretório atual (ou `ALI_COINS_BASE_DIR`), `credentials.env` via dotenvy e
//! ambiente do processo.

use ali_coins_core::config::{Account, Config, EnvSource, load_accounts};
use std::path::PathBuf;

/// Contexto resolvido (base dir, ambiente, config e contas).
pub(crate) struct CliContext {
    /// Diretório base (credentials.env, sessões, relatórios).
    pub base_dir: PathBuf,
    /// Ambiente (processo + credentials.env).
    pub env: EnvSource,
    /// Configuração validada.
    pub config: Config,
    /// Contas carregadas (pelo menos uma).
    pub accounts: Vec<Account>,
}

/// Resolve o contexto a partir de diretório/ambiente explícitos.
pub(crate) fn context_from(base_dir: PathBuf, env: EnvSource) -> Option<CliContext> {
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
    Some(CliContext {
        base_dir,
        env,
        config,
        accounts,
    })
}

/// Bootstrap de produção (cwd ou `ALI_COINS_BASE_DIR`, com dotenv).
pub(crate) fn bootstrap() -> Option<CliContext> {
    let base_dir = std::env::var("ALI_COINS_BASE_DIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            PathBuf::from,
        );
    let credentials = base_dir.join("credentials.env");
    if credentials.exists() {
        let _ = dotenvy::from_path(&credentials);
    }
    let env = EnvSource::from_current_process();
    context_from(base_dir, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_ok() -> EnvSource {
        EnvSource::from_pairs([
            ("ALI_USER", "user@example.com"),
            ("ALI_PASSWORD", "senha"),
            ("SESSION_SECRET", "0123456789abcdef0123456789abcdef"),
            ("TELEGRAM_ENABLED", "false"),
        ])
    }

    #[test]
    fn contexto_invalido_retorna_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(context_from(dir.path().to_path_buf(), EnvSource::default()).is_none());
    }

    #[test]
    fn contexto_valido_resolve_contas() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = context_from(dir.path().to_path_buf(), env_ok()).expect("contexto");
        assert_eq!(ctx.accounts.len(), 1);
        assert_eq!(ctx.accounts[0].user, "user@example.com");
        assert!(
            ctx.base_dir
                .ends_with(dir.path().file_name().expect("nome"))
        );
    }
}
