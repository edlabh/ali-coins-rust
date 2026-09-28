//! Carregamento de contas (`accounts.json`, `ALI_USER[_N]`) com paths por conta.

use crate::config::env::EnvSource;
use crate::secure_fs::safe_chmod_600;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Conta configurada, com paths isolados (igual ao `loadAccounts` do oráculo).
#[derive(Debug, Clone)]
pub struct Account {
    /// Índice 1-based exibido no relatório.
    pub index: usize,
    /// Usuário original (e-mail/telefone).
    pub user: String,
    /// Senha resolvida (env/file/inline).
    pub password: String,
    /// Usuário mascarado para logs.
    pub masked_user: String,
    /// Chat ID efetivo (conta > global).
    pub telegram_chat_id: Option<String>,
    /// `session.json` (primária) ou `session_<hash>.json`.
    pub session_path: PathBuf,
    /// `session_meta.json` (primária) ou `session_meta_<hash>.json`.
    pub session_meta_path: PathBuf,
    /// Lock global (primária) ou por conta.
    pub lock_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct AccountFileEntry {
    user: Option<String>,
    password: Option<String>,
    #[serde(rename = "passwordEnv")]
    password_env: Option<String>,
    #[serde(rename = "passwordFile")]
    password_file: Option<String>,
    #[serde(rename = "telegramChatId")]
    telegram_chat_id: Option<String>,
    #[serde(rename = "telegram_chat_id")]
    telegram_chat_id_snake: Option<String>,
}

#[derive(Debug)]
struct RawAccount {
    user: String,
    password: String,
    telegram_chat_id: Option<String>,
}

/// Carrega as contas na mesma ordem/precedência do oráculo.
#[must_use]
pub fn load_accounts(env: &EnvSource, base_dir: &Path) -> Vec<Account> {
    let mut accounts: Vec<RawAccount> = Vec::new();
    let accounts_file = base_dir.join("accounts.json");

    if accounts_file.exists() {
        let _ = safe_chmod_600(&accounts_file);
        if let Ok(raw) = std::fs::read_to_string(&accounts_file) {
            if let Ok(entries) = serde_json::from_str::<Vec<AccountFileEntry>>(&raw) {
                for entry in entries {
                    let Some(user) = entry
                        .user
                        .as_deref()
                        .map(str::trim)
                        .filter(|u| !u.is_empty())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    let chat_id = entry
                        .telegram_chat_id
                        .as_deref()
                        .or(entry.telegram_chat_id_snake.as_deref())
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .map(str::to_string);
                    let Some(password) = resolve_password(&entry, env, base_dir) else {
                        continue;
                    };
                    upsert(&mut accounts, &user, password, chat_id);
                }
            }
        }
    }

    if let (Some(user), Some(password)) = (env.get("ALI_USER"), env.get("ALI_PASSWORD")) {
        let user = user.trim();
        if !user.is_empty() {
            let chat = env
                .get("TELEGRAM_CHAT_ID_1")
                .or_else(|| env.get("TELEGRAM_CHAT_ID"))
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if let Some(existing) = accounts
                .iter_mut()
                .find(|account| account.user.eq_ignore_ascii_case(user))
            {
                if existing.telegram_chat_id.is_none() {
                    existing.telegram_chat_id = chat;
                }
            } else {
                accounts.insert(
                    0,
                    RawAccount {
                        user: user.to_string(),
                        password: password.to_string(),
                        telegram_chat_id: chat,
                    },
                );
            }
        }
    }

    for i in 2..=20 {
        let user = env.get(&format!("ALI_USER_{i}"));
        let password = env.get(&format!("ALI_PASSWORD_{i}"));
        if let (Some(user), Some(password)) = (user, password) {
            let user = user.trim();
            if user.is_empty() {
                continue;
            }
            let chat = env
                .get(&format!("TELEGRAM_CHAT_ID_{i}"))
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            upsert(&mut accounts, user, password.to_string(), chat);
        }
    }

    accounts
        .into_iter()
        .enumerate()
        .map(|(idx, account)| build_account(idx, account, env, base_dir))
        .collect()
}

fn resolve_password(entry: &AccountFileEntry, env: &EnvSource, base_dir: &Path) -> Option<String> {
    if let Some(name) = entry
        .password_env
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return env
            .get(name)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if let Some(file) = entry.password_file.as_deref() {
        let candidate = if Path::new(file).is_absolute() {
            PathBuf::from(file)
        } else {
            base_dir.join(file)
        };
        let real_base = base_dir.canonicalize().ok()?;
        let real_file = candidate.canonicalize().ok()?;
        if !real_file.starts_with(&real_base) {
            return None;
        }
        return std::fs::read_to_string(&real_file)
            .ok()
            .map(|content| content.trim().to_string())
            .filter(|password| !password.is_empty());
    }
    entry
        .password
        .as_deref()
        .filter(|password| !password.is_empty())
        .map(str::to_string)
}

fn upsert(accounts: &mut Vec<RawAccount>, user: &str, password: String, chat_id: Option<String>) {
    if let Some(existing) = accounts
        .iter_mut()
        .find(|account| account.user.eq_ignore_ascii_case(user))
    {
        if existing.telegram_chat_id.is_none() {
            existing.telegram_chat_id = chat_id;
        }
    } else {
        accounts.push(RawAccount {
            user: user.to_string(),
            password,
            telegram_chat_id: chat_id,
        });
    }
}

fn build_account(idx: usize, account: RawAccount, env: &EnvSource, base_dir: &Path) -> Account {
    let hash = sha256_hex(&account.user);
    let short_hash = hash.get(..8).unwrap_or_default().to_string();
    let is_primary = idx == 0;

    let env_chat = if is_primary {
        env.get("TELEGRAM_CHAT_ID_1")
            .or_else(|| env.get("TELEGRAM_CHAT_ID"))
    } else {
        env.get(&format!("TELEGRAM_CHAT_ID_{}", idx + 1))
            .or_else(|| env.get("TELEGRAM_CHAT_ID"))
    };
    let telegram_chat_id = account.telegram_chat_id.or_else(|| {
        env_chat
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
    });

    Account {
        index: idx + 1,
        user: account.user.clone(),
        password: account.password,
        masked_user: mask_user(&account.user),
        telegram_chat_id,
        session_path: if is_primary {
            base_dir.join("session.json")
        } else {
            base_dir.join(format!("session_{short_hash}.json"))
        },
        session_meta_path: if is_primary {
            base_dir.join("session_meta.json")
        } else {
            base_dir.join(format!("session_meta_{short_hash}.json"))
        },
        lock_path: if is_primary {
            base_dir.join(format!("ali-coins-{}.lock", lock_user_suffix()))
        } else {
            base_dir.join(format!(
                "ali-coins-{}-{short_hash}.lock",
                lock_user_suffix()
            ))
        },
    }
}

/// Mascara e-mail/telefone igual ao `maskUser`.
#[must_use]
pub fn mask_user(user: &str) -> String {
    if user.is_empty() {
        return "***".to_string();
    }
    if let Some(at) = user.find('@') {
        let local = &user[..at];
        let domain = &user[at..];
        if local.chars().count() >= 2 {
            let prefix: String = local.chars().take(2).collect();
            return format!("{prefix}***{domain}");
        }
        return format!("***{domain}");
    }
    if user.chars().count() > 4 {
        let prefix: String = user.chars().take(2).collect();
        format!("{prefix}***")
    } else {
        "***".to_string()
    }
}

/// Mascara Chat ID expondo os 4 primeiros dígitos.
#[must_use]
pub fn mask_chat_id(chat_id: &str) -> String {
    let value = chat_id.trim();
    if value.is_empty() {
        return String::new();
    }
    if value.chars().count() <= 4 {
        return "***".to_string();
    }
    let prefix: String = value.chars().take(4).collect();
    format!("{prefix}***")
}

fn lock_user_suffix() -> String {
    #[cfg(unix)]
    {
        format!("u{}", nix::unistd::Uid::current().as_raw())
    }
    #[cfg(not(unix))]
    {
        let name = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        sha256_hex(&name).get(..8).unwrap_or_default().to_string()
    }
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mascara_usuario() {
        assert_eq!(mask_user("ab@exemplo.com"), "ab***@exemplo.com");
        assert_eq!(mask_user("a@exemplo.com"), "***@exemplo.com");
        assert_eq!(mask_user("5511999999999"), "55***");
        assert_eq!(mask_user("1234"), "***");
        assert_eq!(mask_user(""), "***");
    }

    #[test]
    fn mascara_chat_id() {
        assert_eq!(mask_chat_id("123456789"), "1234***");
        assert_eq!(mask_chat_id("1234"), "***");
        assert_eq!(mask_chat_id(""), "");
    }

    #[test]
    fn hash_de_conta_estavel() {
        let env = EnvSource::from_pairs([("ALI_USER", "user@example.com"), ("ALI_PASSWORD", "pw")]);
        let accounts = load_accounts(&env, Path::new("/tmp"));
        assert_eq!(accounts.len(), 1);
        let expected = sha256_hex("user@example.com");
        assert!(
            accounts[0]
                .session_path
                .to_string_lossy()
                .starts_with("/tmp/session.json")
        );
        assert_eq!(accounts[0].index, 1);
        assert!(!expected.is_empty());
    }

    #[test]
    fn multi_conta_ordem_e_hash() {
        let env = EnvSource::from_pairs([
            ("ALI_USER", "one@example.com"),
            ("ALI_PASSWORD", "pw1"),
            ("ALI_USER_2", "two@example.com"),
            ("ALI_PASSWORD_2", "pw2"),
            ("ALI_USER_2", "two@example.com"),
        ]);
        let accounts = load_accounts(&env, Path::new("/tmp"));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].index, 1);
        assert_eq!(accounts[1].index, 2);
        let hash = sha256_hex("two@example.com");
        let expected = format!("/tmp/session_{}.json", &hash[..8]);
        assert_eq!(accounts[1].session_path.to_string_lossy(), expected);
    }

    #[test]
    fn dedup_case_insensitive_preserva_primeira() {
        let env = EnvSource::from_pairs([
            ("ALI_USER", "User@Example.com"),
            ("ALI_PASSWORD", "pw1"),
            ("ALI_USER_2", "user@example.com"),
            ("ALI_PASSWORD_2", "pw2"),
        ]);
        let accounts = load_accounts(&env, Path::new("/tmp"));
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].password, "pw1");
    }
}
