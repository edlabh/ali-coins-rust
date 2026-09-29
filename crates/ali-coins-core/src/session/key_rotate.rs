//! Rotação de chave at-rest e migração de sessão legada — port de
//! `libs/session.js` (`rotateSessionSecret` e `migrateLegacySession`).

use super::paths::resolve_session_paths;
use super::store::read_meta;
use super::{SessionError, SessionOptions, backup_timestamp, encryption_config, iso_timestamp};
use crate::config::EnvSource;
use crate::crypto::{EncryptOptions, decrypt_session, encrypt_session};
use crate::secure_fs::{safe_chmod_600, safe_write_file};
use chrono::Utc;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Resultado da rotação de chave de uma conta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateOutcome {
    /// Rotação concluída.
    pub success: bool,
    /// Usuário da sessão (pode ser `None`).
    pub user: Option<String>,
    /// Backup versionado criado antes da re-criptografia.
    pub backup_path: PathBuf,
}

/// Resultado da migração de sessão legada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateOutcome {
    /// Migração concluída.
    pub migrated: bool,
    /// Usuário da sessão.
    pub user: Option<String>,
    /// Quantidade de cookies migrados.
    pub cookies_count: Option<usize>,
    /// A sessão ficou cifrada.
    pub encrypted: Option<bool>,
}

fn crypto_err(message: impl Into<String>) -> SessionError {
    SessionError::Crypto(message.into())
}

#[allow(clippy::needless_pass_by_value)]
fn json_err(error: serde_json::Error) -> SessionError {
    SessionError::Json(error.to_string())
}

#[allow(clippy::needless_pass_by_value)]
fn io_err(error: std::io::Error) -> SessionError {
    SessionError::Io(error.to_string())
}

fn resolve_secrets(options: &SessionOptions, env: &EnvSource) -> (Option<String>, Option<String>) {
    let old = options
        .old_secret
        .clone()
        .or_else(|| env.get("SESSION_SECRET_OLD").map(str::to_string))
        .or_else(|| options.secret.clone())
        .or_else(|| env.get("SESSION_SECRET").map(str::to_string));

    let new = options
        .new_secret
        .clone()
        .or_else(|| env.get("SESSION_SECRET_NEW").map(str::to_string))
        .or_else(|| {
            env.get("SESSION_SECRET_OLD").and_then(|_| {
                options
                    .secret
                    .clone()
                    .or_else(|| env.get("SESSION_SECRET").map(str::to_string))
            })
        });

    (old, new)
}

/// Rotaciona a chave at-rest da sessão (port de `rotateSessionSecret`).
///
/// Lê a sessão atual (`.enc` com a chave antiga ou `session.json` legado),
/// cria backup cifrado versionado, re-cifra com a nova chave, valida o
/// round-trip, grava em `.enc` (0600) e atualiza os metadados.
pub fn rotate_session_secret(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
) -> Result<RotateOutcome, SessionError> {
    let paths = resolve_session_paths(options, default_base);
    let (old_secret, new_secret) = resolve_secrets(options, env);

    let old_secret = old_secret
        .filter(|value| value.chars().count() >= 32)
        .ok_or_else(|| {
            crypto_err(
                "SESSION_SECRET_OLD é obrigatório e deve ter no mínimo 32 caracteres para descriptografar a sessão atual durante a rotação.",
            )
        })?;
    let new_secret = new_secret
        .filter(|value| value.chars().count() >= 32)
        .ok_or_else(|| {
            crypto_err(
                "SESSION_SECRET_NEW (ou nova SESSION_SECRET) é obrigatório e deve ter no mínimo 32 caracteres para re-criptografar a sessão.",
            )
        })?;
    if old_secret == new_secret {
        return Err(crypto_err(
            "A nova chave de sessão deve ser diferente da chave atual para efetuar a rotação.",
        ));
    }

    let (raw_json, session_data) = if paths.enc_path.exists() {
        let encrypted = std::fs::read_to_string(&paths.enc_path).map_err(io_err)?;
        let raw = decrypt_session(&encrypted, &old_secret).map_err(|error| {
            crypto_err(format!(
                "Falha ao descriptografar session.json.enc com SESSION_SECRET_OLD: {error}"
            ))
        })?;
        let data: Value = serde_json::from_str(&raw).map_err(json_err)?;
        (raw, data)
    } else if paths.s_path.exists() {
        let raw = std::fs::read_to_string(&paths.s_path).map_err(io_err)?;
        let data: Value = serde_json::from_str(&raw).map_err(|_| {
            SessionError::Json("Falha ao ler session.json legado: JSON malformado.".to_string())
        })?;
        (raw, data)
    } else {
        return Err(crypto_err(
            "Nenhum arquivo de sessão existente (session.json ou session.json.enc) encontrado para rotação.",
        ));
    };
    if !session_data.get("cookies").is_some_and(Value::is_array) {
        return Err(crypto_err(
            "Estrutura de sessão inválida: array de cookies ausente.",
        ));
    }

    // Backup versionado da sessão antiga (re-cifrado com a chave antiga).
    std::fs::create_dir_all(&paths.scratch_dir).map_err(io_err)?;
    let backup_path = paths.scratch_dir.join(format!(
        "session.bak-{}.json.enc",
        backup_timestamp(Utc::now())
    ));
    let backup = encrypt_session(&raw_json, &old_secret, &EncryptOptions::default())
        .map_err(|error| crypto_err(error.to_string()))?;
    safe_write_file(&backup_path, backup.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(&backup_path);

    // Re-cifra com a nova chave e valida o round-trip ANTES de gravar.
    let pretty = serde_json::to_string_pretty(&session_data).map_err(json_err)?;
    let encrypted_new = encrypt_session(&pretty, &new_secret, &EncryptOptions::default())
        .map_err(|error| crypto_err(error.to_string()))?;
    let verified = decrypt_session(&encrypted_new, &new_secret).map_err(|error| {
        crypto_err(format!(
            "Falha na validação roundtrip pós-criptografia com nova chave: {error}"
        ))
    })?;
    let verified_json: Value = serde_json::from_str(&verified).map_err(json_err)?;
    if !verified_json.get("cookies").is_some_and(Value::is_array) {
        return Err(crypto_err(
            "Verificação roundtrip falhou: cookies inválidos.",
        ));
    }

    safe_write_file(&paths.enc_path, encrypted_new.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(&paths.enc_path);
    if paths.s_path.exists() {
        let _ = std::fs::remove_file(&paths.s_path);
    }

    let mut meta = read_meta(&paths.m_path).unwrap_or_default();
    meta.last_rotated_at = Some(iso_timestamp(Utc::now()));
    meta.encrypted = Some(true);
    let meta_json = serde_json::to_string_pretty(&meta).map_err(json_err)?;
    safe_write_file(&paths.m_path, meta_json.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(&paths.m_path);

    let _ = super::prune_session_backups(options, default_base, env);

    crate::logging::global().info("[ROTAÇÃO] Chave de sessão rotacionada com sucesso.", &[]);
    Ok(RotateOutcome {
        success: true,
        user: meta.user,
        backup_path,
    })
}

/// Migra `session.json` legado (texto claro) para `session.json.enc`.
///
/// Nunca sobrescreve um `.enc` existente sem backup; só remove o texto claro
/// após a gravação e a verificação do round-trip com criptografia ativa.
pub fn migrate_legacy_session(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
) -> Result<MigrateOutcome, SessionError> {
    let paths = resolve_session_paths(options, default_base);
    let config = encryption_config(env, options);

    let secret = config
        .secret
        .clone()
        .filter(|value| value.chars().count() >= 32)
        .ok_or_else(|| {
            crypto_err(
                "SESSION_SECRET é obrigatório e deve ter no mínimo 32 caracteres para migração segura.",
            )
        })?;
    if !config.should_encrypt {
        return Err(crypto_err(
            "ENCRYPT_LOCAL_SESSION está desligado; a migração para .enc requer criptografia ativa. Remova o opt-out ou use --plaintext no import.",
        ));
    }
    if !paths.s_path.exists() {
        return Err(crypto_err(format!(
            "Arquivo legado session.json não encontrado em: \"{}\"",
            paths.s_path.display()
        )));
    }

    let content = std::fs::read_to_string(&paths.s_path).map_err(io_err)?;
    let parsed: Value = serde_json::from_str(&content).map_err(|_| {
        SessionError::Json(
            "Conteúdo do session.json legado é inválido (JSON malformado).".to_string(),
        )
    })?;
    let session_data = parsed.get("session").cloned().unwrap_or(parsed);
    if !session_data.get("cookies").is_some_and(Value::is_array) {
        return Err(crypto_err(
            "Payload de sessão inválido: array de cookies ausente.",
        ));
    }
    let cookies_count = session_data
        .get("cookies")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let mut meta = read_meta(&paths.m_path).unwrap_or_default();
    if meta.user.is_none() {
        meta.user = Some("legado".to_string());
    }
    meta.encrypted = Some(true);
    meta.migrated_at = Some(iso_timestamp(Utc::now()));
    meta.saved_at = Some(iso_timestamp(Utc::now()));

    if paths.enc_path.exists() {
        let backup = PathBuf::from(format!(
            "{}.bak-{}",
            paths.enc_path.display(),
            Utc::now().timestamp_millis()
        ));
        std::fs::rename(&paths.enc_path, &backup).map_err(io_err)?;
        let _ = safe_chmod_600(&backup);
        crate::logging::global().warn(
            "session.json.enc existente preservado como backup antes da migração do texto claro.",
            &[],
        );
    }

    let pretty = serde_json::to_string_pretty(&session_data).map_err(json_err)?;
    let encrypted = encrypt_session(&pretty, &secret, &EncryptOptions::default())
        .map_err(|error| crypto_err(error.to_string()))?;
    safe_write_file(&paths.enc_path, encrypted.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(&paths.enc_path);

    let Ok(verified) = decrypt_session(&encrypted, &secret) else {
        return Err(crypto_err("Falha na verificação da migração para .enc."));
    };
    let Ok(verified_json) = serde_json::from_str::<Value>(&verified) else {
        return Err(crypto_err("Falha na verificação da migração para .enc."));
    };
    if !verified_json.get("cookies").is_some_and(Value::is_array) {
        return Err(crypto_err("Falha na verificação da migração para .enc."));
    }

    let _ = std::fs::remove_file(&paths.s_path);
    let meta_json = serde_json::to_string_pretty(&meta).map_err(json_err)?;
    safe_write_file(&paths.m_path, meta_json.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(&paths.m_path);

    crate::logging::global().info(
        "Sessão legada migrada com sucesso para formato criptografado at-rest.",
        &[],
    );
    Ok(MigrateOutcome {
        migrated: true,
        user: meta.user,
        cookies_count: Some(cookies_count),
        encrypted: Some(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const OLD_SECRET: &str = "chave-antiga-com-mais-de-32-caracteres-000";
    const NEW_SECRET: &str = "chave-nova-com-mais-de-32-caracteres-1111";

    fn session_payload() -> Value {
        serde_json::json!({ "cookies": [{ "name": "xman_us_t", "value": "abc" }], "origins": [] })
    }

    fn options_for(dir: &Path) -> SessionOptions {
        SessionOptions {
            base_dir: Some(dir.to_path_buf()),
            ..SessionOptions::default()
        }
    }

    fn env_with(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_pairs(pairs.iter().map(|(key, value)| (*key, *value)))
    }

    #[test]
    fn rotaciona_sessao_cifrada() {
        let dir = TempDir::new().unwrap();
        let options = options_for(dir.path());
        // Sessão escrita com a chave antiga pelo fluxo normal.
        let env_old = env_with(&[("SESSION_SECRET", OLD_SECRET)]);
        let saved = super::super::save_session(
            &options,
            dir.path(),
            &env_old,
            session_payload(),
            "user@example.com",
        )
        .unwrap()
        .expect("sessão salva");
        assert_eq!(saved.meta.encrypted, Some(true));

        let env_rotate = env_with(&[
            ("SESSION_SECRET_OLD", OLD_SECRET),
            ("SESSION_SECRET", NEW_SECRET),
        ]);
        let outcome = rotate_session_secret(&options, dir.path(), &env_rotate).unwrap();
        assert!(outcome.success);
        assert!(outcome.backup_path.exists());

        let enc = std::fs::read_to_string(dir.path().join("session.json.enc")).unwrap();
        let decrypted = decrypt_session(&enc, NEW_SECRET).unwrap();
        assert!(decrypted.contains("xman_us_t"));
        assert!(!dir.path().join("session.json").exists());

        let meta = read_meta(&dir.path().join("session_meta.json")).unwrap();
        assert!(meta.last_rotated_at.is_some());
    }

    #[test]
    fn rotacao_exige_chaves_validas_e_diferentes() {
        let dir = TempDir::new().unwrap();
        let options = options_for(dir.path());
        let env = env_with(&[("SESSION_SECRET", OLD_SECRET)]);
        super::super::save_session(
            &options,
            dir.path(),
            &env,
            session_payload(),
            "user@example.com",
        )
        .unwrap();

        let same = env_with(&[
            ("SESSION_SECRET_OLD", OLD_SECRET),
            ("SESSION_SECRET_NEW", OLD_SECRET),
        ]);
        assert!(rotate_session_secret(&options, dir.path(), &same).is_err());

        let missing_new = env_with(&[("SESSION_SECRET", OLD_SECRET)]);
        assert!(rotate_session_secret(&options, dir.path(), &missing_new).is_err());
    }

    #[test]
    fn migra_sessao_legada_em_texto_claro() {
        let dir = TempDir::new().unwrap();
        let options = options_for(dir.path());
        let legacy = serde_json::to_string_pretty(&session_payload()).unwrap();
        std::fs::write(dir.path().join("session.json"), legacy).unwrap();

        let env = env_with(&[("SESSION_SECRET", OLD_SECRET)]);
        let outcome = migrate_legacy_session(&options, dir.path(), &env).unwrap();
        assert!(outcome.migrated);
        assert_eq!(outcome.cookies_count, Some(1));
        assert_eq!(outcome.encrypted, Some(true));

        let enc = std::fs::read_to_string(dir.path().join("session.json.enc")).unwrap();
        let decrypted = decrypt_session(&enc, OLD_SECRET).unwrap();
        assert!(decrypted.contains("xman_us_t"));
        assert!(!dir.path().join("session.json").exists());

        let meta = read_meta(&dir.path().join("session_meta.json")).unwrap();
        assert!(meta.migrated_at.is_some());
        assert_eq!(meta.user.as_deref(), Some("legado"));
    }

    #[test]
    fn migracao_falha_sem_texto_claro() {
        let dir = TempDir::new().unwrap();
        let options = options_for(dir.path());
        let env = env_with(&[("SESSION_SECRET", OLD_SECRET)]);
        assert!(migrate_legacy_session(&options, dir.path(), &env).is_err());
    }
}
