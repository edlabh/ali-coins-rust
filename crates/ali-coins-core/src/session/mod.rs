//! Persistência de sessão compatível com `libs/session.js` do oráculo.
//!
//! Cobre: resolução de paths, configuração de criptografia at-rest, leitura
//! (com migração v1/v2→v3 e rotação via `SESSION_SECRET_OLD`), escrita atômica
//! 0600 com filtro de storage, metadados, streak, cooldown de captcha, backup
//! e poda de artefatos.

pub mod key_rotate;
pub mod model;
pub mod paths;
pub mod prune;
pub mod storage_filter;
pub mod store;

pub use key_rotate::{
    MigrateOutcome, RotateOutcome, migrate_legacy_session, rotate_session_secret,
};
pub use model::{
    SessionMeta, Validation, is_captcha_cooldown_active, is_cookie_expired, is_imported_session,
    validate_session, validate_session_payload,
};
pub use paths::{SessionPaths, resolve_session_paths};
pub use prune::{clean_orphan_tmp_files, is_prunable_artifact, prune_session_backups};
pub use storage_filter::{filter_storage_state, is_allowed_storage_key, should_filter_storage};
pub use store::{
    ImportOutcome, LoadedSession, SavedSession, clear_session, export_session_token,
    get_captcha_cooldown, import_session_token, load_session_files, mark_session_imported,
    record_captcha_challenge, save_session, session_meta_is_imported, update_session_streak,
};

use std::path::PathBuf;

/// Opções compartilhadas de sessão (equivalente ao objeto `options` do oráculo).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    /// Diretório base do projeto (default: diretório atual).
    pub base_dir: Option<PathBuf>,
    /// Caminho customizado da sessão (pode terminar em `.enc`).
    pub session_path: Option<PathBuf>,
    /// Caminho customizado do meta.
    pub session_meta_path: Option<PathBuf>,
    /// Diretório de scratch/backups.
    pub scratch_dir: Option<PathBuf>,
    /// `SESSION_SECRET` explícito (senão usa env).
    pub secret: Option<String>,
    /// `SESSION_SECRET_OLD` explícito.
    pub old_secret: Option<String>,
    /// `SESSION_SECRET_NEW` explícito (rotação de chave).
    pub new_secret: Option<String>,
    /// `ENCRYPT_LOCAL_SESSION` explícito.
    pub encrypt_local_session: Option<bool>,
    /// `SESSION_STRICT_STORAGE` explícito.
    pub filter_storage: Option<bool>,
    /// Login novo: limpa marcadores de import/captcha.
    pub fresh_login: bool,
    /// Dias de streak a persistir (`None` = não altera).
    pub streak_days: Option<i64>,
    /// Pula a leitura da sessão (apenas meta).
    pub skip_session: bool,
    /// Desliga a migração automática texto→.enc.
    pub auto_migrate: bool,
    /// Dias de retenção de artefatos.
    pub retention_days: Option<f64>,
    /// Não remove arquivos (apenas reporta).
    pub dry_run: bool,
}

impl SessionOptions {
    /// Opções com base dir definido.
    #[must_use]
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self {
            base_dir: Some(base_dir),
            auto_migrate: true,
            ..Self::default()
        }
    }
}

/// Erros do módulo de sessão.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Falha de I/O.
    #[error("{0}")]
    Io(String),
    /// Falha de criptografia.
    #[error("{0}")]
    Crypto(String),
    /// JSON inválido.
    #[error("{0}")]
    Json(String),
}

/// Configuração efetiva de criptografia at-rest.
#[derive(Debug, Clone)]
pub struct EncryptionConfig {
    /// Segredo atual (env/opções).
    pub secret: Option<String>,
    /// Segredo antigo (rotação).
    pub old_secret: Option<String>,
    /// Deve cifrar ao gravar/ler?
    pub should_encrypt: bool,
    /// `ENCRYPT_LOCAL_SESSION` efetivo.
    pub encrypt_local: bool,
}

/// Resolve a configuração de criptografia (equivalente a `getEncryptionConfig`).
#[must_use]
pub fn encryption_config(
    env: &crate::config::EnvSource,
    options: &SessionOptions,
) -> EncryptionConfig {
    let secret = options
        .secret
        .clone()
        .or_else(|| env.get("SESSION_SECRET").map(str::to_string));
    let old_secret = options
        .old_secret
        .clone()
        .or_else(|| env.get("SESSION_SECRET_OLD").map(str::to_string));

    let encrypt_local = options.encrypt_local_session.unwrap_or_else(|| {
        let raw = env.get("ENCRYPT_LOCAL_SESSION");
        crate::config::env::bool_encrypt_local(raw)
    });

    let should_encrypt = secret
        .as_deref()
        .is_some_and(|value| value.chars().count() >= 32)
        && encrypt_local;

    EncryptionConfig {
        secret,
        old_secret,
        should_encrypt,
        encrypt_local,
    }
}

/// Timestamp ISO no formato usado em backups (`: .` → `-`).
#[must_use]
pub fn backup_timestamp(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
        .replace([':', '.'], "-")
}

/// Timestamp ISO canônico (`toISOString`).
#[must_use]
pub fn iso_timestamp(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Erro de I/O convertido para o tipo do módulo.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn io_err(err: std::io::Error) -> SessionError {
    SessionError::Io(err.to_string())
}
