//! Leitura/escrita/limpeza dos arquivos de sessão (equivalente ao núcleo de `libs/session.js`).

use super::model::{SessionMeta, has_auth_cookies};
use super::prune::{is_within, prune_session_backups};
use super::{
    SessionError, SessionOptions, backup_timestamp, encryption_config, io_err, iso_timestamp,
};
use crate::config::EnvSource;
use crate::crypto::EncryptOptions;
use crate::secure_fs::{safe_chmod_600, safe_write_file};
use crate::session::storage_filter::{filter_storage_state, should_filter_storage};
use chrono::Utc;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Resultado da leitura dos arquivos de sessão.
#[derive(Debug, Clone, Default)]
pub struct LoadedSession {
    /// Payload (storage state) ou `None`.
    pub session_data: Option<Value>,
    /// Metadados ou `None`.
    pub meta_data: Option<SessionMeta>,
    /// Meta ilegível (JSON malformado).
    pub meta_corrupted: bool,
    /// Token legado (v1/v2) migrado para v3.
    pub migrated_legacy_token: bool,
    /// Sessão recuperada com `SESSION_SECRET_OLD` e re-cifrada.
    pub rotated_with_old_secret: bool,
}

/// Resultado de uma gravação.
#[derive(Debug, Clone)]
pub struct SavedSession {
    /// Payload persistido (após filtro de storage).
    pub payload: Value,
    /// Metadados gravados.
    pub meta: SessionMeta,
}

/// Estado do cooldown pós-captcha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptchaCooldown {
    /// Janela ativa?
    pub active: bool,
    /// Horas configuradas.
    pub hours: u64,
    /// Último desafio registrado.
    pub last_captcha_at: Option<String>,
    /// Fim estimado da janela.
    pub until: Option<String>,
}

/// Lê `.enc`/`.json` + meta, aplicando migrações e rotação de chave.
pub fn load_session_files(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
) -> Result<LoadedSession, SessionError> {
    let paths = super::resolve_session_paths(options, default_base);
    let config = encryption_config(env, options);
    let mut loaded = LoadedSession::default();

    if !options.skip_session && paths.enc_path.exists() {
        if let Ok(encrypted) = std::fs::read_to_string(&paths.enc_path) {
            let secret = config
                .secret
                .as_deref()
                .filter(|value| value.chars().count() >= 32)
                .map(str::to_string);

            if let Some(secret) = secret {
                match crate::crypto::decrypt_session(&encrypted, &secret) {
                    Ok(decrypted) => {
                        loaded.session_data = Some(parse_json(&decrypted)?);
                        let version: String = encrypted.trim().chars().take(3).collect();
                        if config.should_encrypt && (version == "v1:" || version == "v2:") {
                            if let Some(value) = &loaded.session_data {
                                if reencrypt(&paths.enc_path, value, &secret, true).is_ok() {
                                    loaded.migrated_legacy_token = true;
                                }
                            }
                        }
                    }
                    Err(_) => {
                        if let Some(old_secret) = config
                            .old_secret
                            .as_deref()
                            .filter(|value| value.chars().count() >= 32)
                        {
                            if let Ok(decrypted_old) =
                                crate::crypto::decrypt_session(&encrypted, old_secret)
                            {
                                let value = parse_json(&decrypted_old)?;
                                if reencrypt(&paths.enc_path, &value, &secret, true).is_ok() {
                                    loaded.rotated_with_old_secret = true;
                                }
                                loaded.session_data = Some(value);
                            }
                        }
                    }
                }
            }
        }
    }

    if !options.skip_session && loaded.session_data.is_none() && paths.s_path.exists() {
        if let Ok(plain) = std::fs::read_to_string(&paths.s_path) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&plain) {
                loaded.session_data = Some(parsed.clone());
                if config.should_encrypt && options.auto_migrate {
                    if let Some(secret) = config.secret.as_deref() {
                        // Preserva um .enc existente como backup (M5 do oráculo).
                        let mut can_migrate = true;
                        if paths.enc_path.exists() {
                            let backup = PathBuf::from(format!(
                                "{}.bak-{}",
                                paths.enc_path.to_string_lossy(),
                                Utc::now().timestamp_millis()
                            ));
                            if std::fs::rename(&paths.enc_path, &backup).is_ok() {
                                let _ = safe_chmod_600(&backup);
                            } else {
                                can_migrate = false;
                            }
                        }
                        if can_migrate {
                            if let Ok(token) = encrypt_value_pretty(&parsed, secret) {
                                let wrote =
                                    safe_write_file(&paths.enc_path, token.as_bytes()).is_ok();
                                let verified = wrote
                                    && crate::crypto::decrypt_session(token.trim(), secret)
                                        .ok()
                                        .and_then(|decrypted| {
                                            serde_json::from_str::<Value>(&decrypted).ok()
                                        })
                                        .and_then(|value| value.get("cookies").map(Value::is_array))
                                        .is_some_and(|is_array| is_array);
                                if verified {
                                    let _ = std::fs::remove_file(&paths.s_path);
                                    let _ = safe_chmod_600(&paths.enc_path);
                                    loaded.migrated_legacy_token = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if paths.m_path.exists() {
        if let Ok(raw) = std::fs::read_to_string(&paths.m_path) {
            match serde_json::from_str::<SessionMeta>(&raw) {
                Ok(meta) => loaded.meta_data = Some(meta),
                Err(_) => loaded.meta_corrupted = true,
            }
        }
    }

    for path in [&paths.enc_path, &paths.s_path, &paths.m_path] {
        if path.exists() {
            let _ = safe_chmod_600(path);
        }
    }

    Ok(loaded)
}

/// Grava a sessão (cifrada ou em texto puro) e os metadados.
///
/// Retorna `Ok(None)` quando o oráculo ignoraria a gravação: sem cookies,
/// sem cookie de autenticação ou com criptografia exigida sem segredo válido.
pub fn save_session(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
    payload: Value,
    user: &str,
) -> Result<Option<SavedSession>, SessionError> {
    let has_cookies_array = payload.get("cookies").is_some_and(Value::is_array);
    if !has_cookies_array || !has_auth_cookies(&payload) {
        return Ok(None);
    }

    let config = encryption_config(env, options);
    if config.encrypt_local && !config.should_encrypt {
        return Ok(None);
    }

    let paths = super::resolve_session_paths(options, default_base);
    let mut meta = read_meta(&paths.m_path).unwrap_or_default();
    meta.user = Some(user.to_string());
    meta.saved_at = Some(iso_timestamp(Utc::now()));
    meta.encrypted = Some(config.should_encrypt);
    if options.fresh_login {
        meta.is_imported = None;
        meta.imported_at = None;
        meta.last_captcha_at = None;
    }
    if let Some(days) = options.streak_days.filter(|days| *days >= 0) {
        meta.last_streak_days = Some(days);
        meta.last_checkin_date = Some(iso_timestamp(Utc::now()));
    }

    let persisted = if should_filter_storage(options, env) {
        filter_storage_state(payload)
    } else {
        payload
    };
    let payload_str = serde_json::to_string(&persisted).map_err(json_err)?;

    if config.should_encrypt {
        let secret = config.secret.as_deref().unwrap_or_default();
        let token =
            crate::crypto::encrypt_session(&payload_str, secret, &EncryptOptions::default())
                .map_err(|err| SessionError::Crypto(err.to_string()))?;
        safe_write_file(&paths.enc_path, token.as_bytes()).map_err(io_err)?;
        let _ = safe_chmod_600(&paths.enc_path);
        if paths.s_path.exists() {
            let _ = std::fs::remove_file(&paths.s_path);
        }
    } else {
        safe_write_file(&paths.s_path, payload_str.as_bytes()).map_err(io_err)?;
        let _ = safe_chmod_600(&paths.s_path);
        if paths.enc_path.exists() {
            let _ = std::fs::remove_file(&paths.enc_path);
        }
    }

    // Sessão primeiro, meta depois (contrato do oráculo).
    let meta_str = serde_json::to_string(&meta).map_err(json_err)?;
    safe_write_file(&paths.m_path, meta_str.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(&paths.m_path);

    let _ = prune_session_backups(options, default_base, env);

    Ok(Some(SavedSession {
        payload: persisted,
        meta,
    }))
}

/// Remove os arquivos de sessão, criando backup versionado em `scratch/`.
#[must_use]
pub fn clear_session(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
) -> Option<PathBuf> {
    let paths = super::resolve_session_paths(options, default_base);
    let config = encryption_config(env, options);
    let allowed_root = options
        .base_dir
        .clone()
        .unwrap_or_else(|| default_base.to_path_buf())
        .join("scratch");
    let target = options
        .scratch_dir
        .clone()
        .unwrap_or_else(|| paths.scratch_dir.clone());

    let mut backup = None;
    if is_within(&allowed_root, &target) {
        let timestamp = backup_timestamp(Utc::now());
        let base_name = paths
            .s_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let tag = if base_name == "session" {
            String::new()
        } else {
            format!("-{base_name}")
        };
        let _ = std::fs::create_dir_all(&target);

        if paths.enc_path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&paths.enc_path) {
                let bak = target.join(format!("session.bak{tag}-{timestamp}.json.enc"));
                if safe_write_file(&bak, raw.as_bytes()).is_ok() {
                    let _ = safe_chmod_600(&bak);
                    backup = Some(bak);
                }
            }
        } else if paths.s_path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&paths.s_path) {
                let secret_ok = config
                    .secret
                    .as_deref()
                    .is_some_and(|value| value.chars().count() >= 32);
                if secret_ok {
                    let secret = config.secret.as_deref().unwrap_or_default();
                    if let Ok(token) =
                        crate::crypto::encrypt_session(&raw, secret, &EncryptOptions::default())
                    {
                        let bak = target.join(format!("session.bak{tag}-{timestamp}.json.enc"));
                        if safe_write_file(&bak, token.as_bytes()).is_ok() {
                            let _ = safe_chmod_600(&bak);
                            backup = Some(bak);
                        }
                    }
                } else {
                    let bak = target.join(format!("session.bak{tag}-{timestamp}.json"));
                    if safe_write_file(&bak, raw.as_bytes()).is_ok() {
                        let _ = safe_chmod_600(&bak);
                        backup = Some(bak);
                    }
                }
            }
        }
    }

    let _ = std::fs::remove_file(&paths.enc_path);
    let _ = std::fs::remove_file(&paths.s_path);
    let _ = std::fs::remove_file(&paths.m_path);

    let _ = prune_session_backups(options, default_base, env);
    backup
}

/// Atualiza `lastStreakDays`/`lastCheckinDate` no meta (0o600).
#[must_use]
pub fn update_session_streak(
    options: &SessionOptions,
    default_base: &Path,
    streak_days: i64,
) -> Option<SessionMeta> {
    if streak_days < 0 {
        return None;
    }
    let paths = super::resolve_session_paths(options, default_base);
    if !paths.m_path.exists() {
        return None;
    }
    let mut meta = read_meta(&paths.m_path)?;
    meta.last_streak_days = Some(streak_days);
    meta.last_checkin_date = Some(iso_timestamp(Utc::now()));
    write_meta_pretty(&paths.m_path, &meta).ok()?;
    Some(meta)
}

/// Registra um desafio anti-bot no meta (`lastCaptchaAt`).
#[must_use]
pub fn record_captcha_challenge(
    options: &SessionOptions,
    default_base: &Path,
    when_iso: &str,
    account_user: Option<&str>,
) -> Option<SessionMeta> {
    let paths = super::resolve_session_paths(options, default_base);
    let mut meta = read_meta(&paths.m_path).unwrap_or_default();
    meta.last_captcha_at = Some(when_iso.to_string());
    if meta.user.is_none() {
        if let Some(user) = account_user {
            meta.user = Some(user.to_string());
        }
    }
    write_meta_pretty(&paths.m_path, &meta).ok()?;
    Some(meta)
}

/// Remove o marcador de captcha do meta, preservando os demais campos.
#[must_use]
pub fn clear_captcha_challenge(
    options: &SessionOptions,
    default_base: &Path,
) -> Option<SessionMeta> {
    let paths = super::resolve_session_paths(options, default_base);
    if !paths.m_path.exists() {
        return None;
    }
    let mut meta = read_meta(&paths.m_path)?;
    if meta.last_captcha_at.is_none() {
        return Some(meta);
    }
    meta.last_captcha_at = None;
    write_meta_pretty(&paths.m_path, &meta).ok()?;
    Some(meta)
}

/// Lê o cooldown pós-captcha da conta.
#[must_use]
pub fn get_captcha_cooldown(
    options: &SessionOptions,
    default_base: &Path,
    hours: u64,
    now_ms: i64,
) -> CaptchaCooldown {
    let paths = super::resolve_session_paths(options, default_base);
    let last_captcha_at = std::fs::read_to_string(&paths.m_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("lastCaptchaAt")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let active =
        super::model::is_captcha_cooldown_active(last_captcha_at.as_deref(), hours, now_ms);
    let until = if active {
        last_captcha_at
            .as_deref()
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .map(|parsed| {
                let until_ms = parsed.timestamp_millis()
                    + i64::try_from(hours).unwrap_or(i64::MAX) * 60 * 60 * 1000;
                chrono::DateTime::from_timestamp_millis(until_ms)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            })
    } else {
        None
    };
    CaptchaCooldown {
        active,
        hours,
        last_captcha_at,
        until,
    }
}

/// Resultado de uma importação.
#[derive(Debug, Clone)]
pub struct ImportOutcome {
    /// Conta de destino.
    pub user: String,
    /// Caminho da sessão gravada.
    pub session_path: PathBuf,
    /// Se ficou cifrada at-rest.
    pub encrypted: bool,
}

/// Exporta um token v3 portável `{ session, meta }` para a conta.
///
/// Reutiliza a sessão em disco, valida contra a conta, aplica o filtro de
/// storage e cifra com o `SESSION_SECRET` (mínimo 32 caracteres).
pub fn export_session_token(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
    account_user: &str,
) -> Result<String, SessionError> {
    let loaded = load_session_files(options, default_base, env)?;
    let Some(session_data) = loaded.session_data else {
        return Err(SessionError::Io(
            "Nenhuma sessão ativa encontrada em disco.".to_string(),
        ));
    };
    let validation =
        super::validate_session(&session_data, loaded.meta_data.as_ref(), Some(account_user));
    if !validation.valid {
        return Err(SessionError::Io(
            validation
                .reason
                .unwrap_or_else(|| "sessão inválida".to_string()),
        ));
    }

    let config = encryption_config(env, options);
    let secret = config.secret.as_deref().ok_or_else(|| {
        SessionError::Crypto("SESSION_SECRET é obrigatório para exportar a sessão.".to_string())
    })?;
    if secret.chars().count() < 32 {
        return Err(SessionError::Crypto(
            "SESSION_SECRET deve conter no mínimo 32 caracteres.".to_string(),
        ));
    }

    let filtered = if should_filter_storage(options, env) {
        filter_storage_state(session_data)
    } else {
        session_data
    };

    let now = Utc::now();
    let mut meta = serde_json::Map::new();
    meta.insert(
        "user".to_string(),
        serde_json::Value::String(account_user.to_string()),
    );
    meta.insert(
        "exportedAt".to_string(),
        serde_json::Value::String(iso_timestamp(now)),
    );
    meta.insert(
        "expiresAt".to_string(),
        serde_json::Value::String(iso_timestamp(now + chrono::Duration::days(90))),
    );
    meta.insert(
        "exportedFrom".to_string(),
        serde_json::Value::String(crate::lock::hostname()),
    );

    let mut payload = serde_json::Map::new();
    payload.insert("session".to_string(), filtered);
    payload.insert("meta".to_string(), serde_json::Value::Object(meta));
    let body =
        serde_json::to_string_pretty(&serde_json::Value::Object(payload)).map_err(json_err)?;

    crate::crypto::encrypt_session(&body, secret, &EncryptOptions::default())
        .map_err(|err| SessionError::Crypto(err.to_string()))
}

/// Importa um token para a conta correspondente (auto-roteamento por `meta.user`).
pub fn import_session_token(
    options: &SessionOptions,
    default_base: &Path,
    env: &EnvSource,
    token: &str,
    expected_user: Option<&str>,
    accounts: &[crate::config::Account],
) -> Result<ImportOutcome, SessionError> {
    let known_users: Vec<String> = accounts
        .iter()
        .map(|account| account.user.clone())
        .collect();
    let config = encryption_config(env, options);
    let secret = config.secret.as_deref().ok_or_else(|| {
        SessionError::Crypto("SESSION_SECRET é obrigatório para importar a sessão.".to_string())
    })?;
    if secret.chars().count() < 32 {
        return Err(SessionError::Crypto(
            "SESSION_SECRET deve conter no mínimo 32 caracteres.".to_string(),
        ));
    }
    let decrypted = crate::crypto::decrypt_session(token.trim(), secret)
        .map_err(|err| SessionError::Crypto(err.to_string()))?;
    let payload: serde_json::Value = parse_json(&decrypted)?;
    super::validate_session_payload(&payload).map_err(SessionError::Json)?;

    let token_user = payload
        .get("meta")
        .and_then(|meta| meta.get("user"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let target_user = match expected_user {
        Some(expected) => {
            if let Some(token_user) = token_user.as_deref() {
                if !token_user.eq_ignore_ascii_case(expected) {
                    return Err(SessionError::Io(format!(
                        "O e-mail do token ({token_user}) não corresponde à conta selecionada ({expected})."
                    )));
                }
            }
            expected.to_string()
        }
        None => {
            if known_users.len() == 1 {
                known_users[0].clone()
            } else {
                let Some(token_user) = token_user else {
                    return Err(SessionError::Io(
                        "Token sem identificação de conta; use --account para escolher o destino."
                            .to_string(),
                    ));
                };
                known_users
                    .iter()
                    .find(|user| user.eq_ignore_ascii_case(&token_user))
                    .cloned()
                    .ok_or_else(|| {
                        SessionError::Io(format!(
                            "O e-mail do token ({token_user}) não corresponde a nenhuma conta configurada."
                        ))
                    })?
            }
        }
    };

    let session_value = payload
        .get("session")
        .cloned()
        .ok_or_else(|| SessionError::Json("Payload sem campo session.".to_string()))?;

    let mut save_options = options.clone();
    if let Some(account) = accounts
        .iter()
        .find(|account| account.user.eq_ignore_ascii_case(&target_user))
    {
        save_options.session_path = Some(account.session_path.clone());
    }
    let saved = save_session(
        &save_options,
        default_base,
        env,
        session_value,
        &target_user,
    )?
    .ok_or_else(|| {
        SessionError::Io(
            "Sessão recusada na gravação (sem cookie de autenticação ou criptografia exigida sem segredo)."
                .to_string(),
        )
    })?;

    // Marcadores de importação preservados do token.
    let token_meta = payload.get("meta").cloned().unwrap_or_default();
    let exported_at = token_meta
        .get("exportedAt")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let expires_at = token_meta
        .get("expiresAt")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let exported_from = token_meta
        .get("exportedFrom")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let _ = mark_session_imported(
        &save_options,
        default_base,
        &target_user,
        exported_at,
        expires_at,
        exported_from,
    );

    let paths = super::resolve_session_paths(&save_options, default_base);
    Ok(ImportOutcome {
        user: target_user,
        session_path: paths.s_path,
        encrypted: saved.meta.encrypted.unwrap_or(false),
    })
}

/// Marca a sessão como importada no meta (preserva os demais campos).
#[must_use]
pub fn mark_session_imported(
    options: &SessionOptions,
    default_base: &Path,
    user: &str,
    exported_at: Option<String>,
    expires_at: Option<String>,
    exported_from: Option<String>,
) -> Option<SessionMeta> {
    let paths = super::resolve_session_paths(options, default_base);
    let mut meta = read_meta(&paths.m_path).unwrap_or_default();
    meta.user = Some(user.to_string());
    meta.is_imported = Some(true);
    meta.imported_at = Some(iso_timestamp(Utc::now()));
    if exported_at.is_some() {
        meta.exported_at = exported_at;
    }
    if expires_at.is_some() {
        meta.expires_at = expires_at;
    }
    if exported_from.is_some() {
        meta.exported_from = exported_from;
    }
    write_meta_pretty(&paths.m_path, &meta).ok()?;
    Some(meta)
}

fn reencrypt(
    enc_path: &Path,
    value: &Value,
    secret: &str,
    pretty: bool,
) -> Result<(), SessionError> {
    let plaintext = if pretty {
        serde_json::to_string_pretty(value).map_err(json_err)?
    } else {
        serde_json::to_string(value).map_err(json_err)?
    };
    let token = crate::crypto::encrypt_session(&plaintext, secret, &EncryptOptions::default())
        .map_err(|err| SessionError::Crypto(err.to_string()))?;
    safe_write_file(enc_path, token.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(enc_path);
    Ok(())
}

pub(crate) fn read_meta(path: &Path) -> Option<SessionMeta> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_meta_pretty(path: &Path, meta: &SessionMeta) -> Result<(), SessionError> {
    let raw = serde_json::to_string_pretty(meta).map_err(json_err)?;
    safe_write_file(path, raw.as_bytes()).map_err(io_err)?;
    let _ = safe_chmod_600(path);
    Ok(())
}

fn parse_json(raw: &str) -> Result<Value, SessionError> {
    serde_json::from_str(raw).map_err(json_err)
}

#[allow(clippy::needless_pass_by_value)]
fn json_err(err: serde_json::Error) -> SessionError {
    SessionError::Json(err.to_string())
}

/// Serializa um `Value` com indentação e cifra em v3.
fn encrypt_value_pretty(value: &Value, secret: &str) -> Result<String, SessionError> {
    let plaintext = serde_json::to_string_pretty(value).map_err(json_err)?;
    crate::crypto::encrypt_session(&plaintext, secret, &EncryptOptions::default())
        .map_err(|err| SessionError::Crypto(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::EncryptVersion;
    use crate::session::storage_filter::storage_state_json;

    fn env_plain() -> EnvSource {
        // Sem criptografia at-rest (equivalente a ENCRYPT_LOCAL_SESSION=false).
        EnvSource::from_pairs([("ENCRYPT_LOCAL_SESSION", "false")])
    }

    fn env_encrypted() -> EnvSource {
        EnvSource::from_pairs([("SESSION_SECRET", "parity-test-secret-0123456789abcdef")])
    }

    fn sample_state() -> Value {
        storage_state_json(
            &[("xman_us_t", "auth-value")],
            &[(
                "https://www.aliexpress.com",
                &[("user_info", "1"), ("aegis", "ruido")],
            )],
        )
    }

    #[test]
    fn grava_e_le_em_texto_puro_com_filtro() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        let saved = save_session(
            &options,
            dir.path(),
            &env_plain(),
            sample_state(),
            "u@e.com",
        )
        .unwrap()
        .expect("gravou");
        assert_eq!(saved.meta.user.as_deref(), Some("u@e.com"));

        let loaded = load_session_files(&options, dir.path(), &env_plain()).unwrap();
        let data = loaded.session_data.expect("sessão");
        let entries = data["origins"][0]["localStorage"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "user_info");
        assert_eq!(loaded.meta_data.unwrap().user.as_deref(), Some("u@e.com"));
    }

    #[test]
    fn criptografia_at_rest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        save_session(
            &options,
            dir.path(),
            &env_encrypted(),
            sample_state(),
            "u@e.com",
        )
        .unwrap()
        .expect("gravou");
        assert!(dir.path().join("session.json.enc").exists());
        assert!(!dir.path().join("session.json").exists());

        let loaded = load_session_files(&options, dir.path(), &env_encrypted()).unwrap();
        assert!(loaded.session_data.is_some());
    }

    #[test]
    fn ignora_storage_sem_cookie_de_auth() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        let state = storage_state_json(&[("outro", "x")], &[]);
        let saved = save_session(&options, dir.path(), &env_plain(), state, "u@e.com").unwrap();
        assert!(saved.is_none());
    }

    #[test]
    fn recusa_gravar_sem_secret_quando_cripto_ligada() {
        let dir = tempfile::tempdir().unwrap();
        let mut options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        options.encrypt_local_session = Some(true);
        let saved = save_session(
            &options,
            dir.path(),
            &env_plain(),
            sample_state(),
            "u@e.com",
        )
        .unwrap();
        assert!(saved.is_none());
        assert!(!dir.path().join("session.json").exists());
    }

    #[test]
    fn migra_token_v2_para_v3_na_leitura() {
        let dir = tempfile::tempdir().unwrap();
        let secret = "parity-test-secret-0123456789abcdef";
        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        let state_str = serde_json::to_string_pretty(&sample_state()).unwrap();
        let v2 = crate::crypto::encrypt_session(
            &state_str,
            secret,
            &EncryptOptions {
                version: Some(EncryptVersion::V2),
                ..EncryptOptions::default()
            },
        )
        .unwrap();
        std::fs::write(dir.path().join("session.json.enc"), &v2).unwrap();

        let loaded = load_session_files(&options, dir.path(), &env_encrypted()).unwrap();
        assert!(loaded.migrated_legacy_token);
        let on_disk = std::fs::read_to_string(dir.path().join("session.json.enc")).unwrap();
        assert!(on_disk.starts_with("v3:"));
    }

    #[test]
    fn rotaciona_chave_com_secret_old() {
        let dir = tempfile::tempdir().unwrap();
        let old_secret = "old-secret-0123456789abcdefghijklmn";
        let new_secret = "new-secret-0123456789abcdefghijklmn";
        let state_str = serde_json::to_string(&sample_state()).unwrap();
        let token =
            crate::crypto::encrypt_session(&state_str, old_secret, &EncryptOptions::default())
                .unwrap();
        std::fs::write(dir.path().join("session.json.enc"), &token).unwrap();

        let mut options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        options.secret = Some(new_secret.to_string());
        options.old_secret = Some(old_secret.to_string());
        let env = env_plain();
        let loaded = load_session_files(&options, dir.path(), &env).unwrap();
        assert!(loaded.rotated_with_old_secret);
        assert!(loaded.session_data.is_some());
        let on_disk = std::fs::read_to_string(dir.path().join("session.json.enc")).unwrap();
        assert!(crate::crypto::decrypt_session(&on_disk, new_secret).is_ok());
    }

    #[test]
    fn clear_cria_backup_e_remove_arquivos() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        save_session(
            &options,
            dir.path(),
            &env_encrypted(),
            sample_state(),
            "u@e.com",
        )
        .unwrap()
        .expect("gravou");

        let backup = clear_session(&options, dir.path(), &env_encrypted());
        assert!(backup.is_some());
        let backup = backup.unwrap();
        assert!(backup.starts_with(dir.path().join("scratch")));
        assert!(backup.exists());
        assert!(!dir.path().join("session.json.enc").exists());
        assert!(!dir.path().join("session_meta.json").exists());
    }

    #[test]
    fn streak_e_captcha_no_meta() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
        save_session(
            &options,
            dir.path(),
            &env_plain(),
            sample_state(),
            "u@e.com",
        )
        .unwrap()
        .expect("gravou");

        let updated = update_session_streak(&options, dir.path(), 42).expect("streak");
        assert_eq!(updated.last_streak_days, Some(42));
        assert!(updated.last_checkin_date.is_some());

        let when = chrono::Utc::now().to_rfc3339();
        let meta = record_captcha_challenge(&options, dir.path(), &when, Some("u@e.com"))
            .expect("captcha");
        assert_eq!(meta.last_captcha_at.as_deref(), Some(when.as_str()));

        let cooldown = get_captcha_cooldown(
            &options,
            dir.path(),
            12,
            chrono::Utc::now().timestamp_millis(),
        );
        assert!(cooldown.active);
        assert!(cooldown.until.is_some());

        let cleared = clear_captcha_challenge(&options, dir.path()).expect("clear");
        assert!(cleared.last_captcha_at.is_none());
    }
}
