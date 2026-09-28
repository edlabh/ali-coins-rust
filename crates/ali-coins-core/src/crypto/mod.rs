//! Criptografia de sessão compatível com o oráculo Node (`security.js`, commit `c05bf07`).
//!
//! Formatos suportados (parser tolerante, igual ao oráculo):
//! - v1: `v1:iv:tag:ciphertext[:base64]` — salt fixo legado, scrypt N=16384.
//! - v2: `v2:salt:iv:tag:ciphertext[:base64]` — salt aleatório, scrypt N=16384.
//! - v3: `v3:N:r:p:salt:iv:tag:ciphertext[:base64]` e compacto
//!   `v3:salt:iv:tag:ciphertext[:base64]` (usa os parâmetros efetivos da máquina).
//!
//! Regras de fidelidade (ADR-0004):
//! - AES-256-GCM sem AAD, nonce 12 B, tag 16 B, salt 16 B.
//! - Encoder base64 padrão (com padding); decoder leniente na leitura.
//! - Qualquer falha de scrypt/GCM/UTF-8 vira a mesma mensagem pública de
//!   autenticação do oráculo; tokens malformados nunca causam panic.

mod base64_lenient;
mod params;
mod token;

pub use params::{LEGACY_SCRYPT, ScryptConfig, effective_default_scrypt_n, sanitize_scrypt_params};
pub use token::{ParsedToken, TokenVersion, parse_session_token};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use rand::RngCore as _;
use zeroize::Zeroizing;

/// Salt fixo dos tokens v1 (mesmo literal do oráculo).
pub const APP_SCRYPT_SALT_V1: &[u8] = b"ali-coins-session-encryption-v1-scrypt-salt";

/// Piso mínimo de segurança para o custo do scrypt (2^14).
pub const SCRYPT_MIN_N: u32 = 16_384;
/// Default seguro moderno (2^17).
pub const SCRYPT_DEFAULT_N: u32 = 131_072;
/// Default reduzido em hosts com pouca RAM (2^15).
pub const SCRYPT_LOW_MEMORY_DEFAULT_N: u32 = 32_768;
/// Limiar de RAM (bytes) para o default reduzido (1,5 GiB).
pub const SCRYPT_LOW_MEMORY_TOTAL_BYTES: u64 = 1_610_612_736;
/// Teto defensivo para N embutido em token não confiável (2^20).
pub const SCRYPT_MAX_N: u32 = 1_048_576;
/// Teto defensivo para r/p de tokens não confiáveis.
pub const SCRYPT_MAX_R: u32 = 16;
/// Teto defensivo para p de tokens não confiáveis.
pub const SCRYPT_MAX_P: u32 = 16;
/// Teto de memória combinada do scrypt (256 MB).
pub const SCRYPT_MEMORY_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// IV canônico do GCM.
pub const TOKEN_IV_LENGTH: usize = 12;
/// Tag canônica do GCM.
pub const TOKEN_TAG_LENGTH: usize = 16;
/// Salt canônico de v2/v3.
pub const TOKEN_SALT_LENGTH: usize = 16;
/// Tamanho da chave derivada.
pub const KEY_LENGTH: usize = 32;

/// Mensagem pública de falha de autenticação/descriptografia (contrato).
pub const AUTH_ERROR_MESSAGE: &str =
    "Falha na autenticação/descriptografia do token. Verifique se o SESSION_SECRET está correto.";

/// Erros do módulo de cripto.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Segredo ausente/curto (mensagem específica por operação).
    #[error("{0}")]
    InvalidSecret(String),
    /// Token com estrutura inválida (mensagem específica por versão).
    #[error("{0}")]
    TokenFormat(String),
    /// Token vazio/ausente.
    #[error("Token de sessão não fornecido ou inválido.")]
    MissingToken,
    /// Falha de autenticação/descriptografia (mensagem genérica do oráculo).
    #[error("{AUTH_ERROR_MESSAGE}")]
    Authentication,
    /// Parâmetros inválidos na criptografia (uso interno; nunca em entrada externa).
    #[error("{0}")]
    InvalidParams(String),
}

/// Versão usada ao cifrar (o oráculo só produz v2/v3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptVersion {
    /// Salt aleatório, scrypt legado (N=16384).
    V2,
    /// Envelope com marcadores N:r:p (default).
    V3,
}

/// Opções de criptografia equivalentes a `{ version, N, r, p }` do oráculo.
#[derive(Debug, Clone, Copy, Default)]
pub struct EncryptOptions {
    /// Versão do token; `None` = v3.
    pub version: Option<EncryptVersion>,
    /// N explícito (coagido/sanitizado como `options.N`).
    pub n: Option<u64>,
    /// r explícito.
    pub r: Option<u64>,
    /// p explícito.
    pub p: Option<u64>,
}

/// Valida o segredo com a mesma regra do oráculo (`trim().length >= 32`).
///
/// Assim como no Node, a derivação usa a string **original**, não a aparada.
pub fn assert_valid_secret(secret: &str, operation: &str) -> Result<(), CryptoError> {
    if secret.trim().len() < 32 {
        return Err(CryptoError::InvalidSecret(format!(
            "SESSION_SECRET é obrigatório e deve ter no mínimo 32 caracteres para {operation}."
        )));
    }
    Ok(())
}

/// Cifra o payload e devolve o token (`v3:...` ou `v2:...`).
pub fn encrypt_session(
    plaintext: &str,
    secret: &str,
    options: &EncryptOptions,
) -> Result<String, CryptoError> {
    assert_valid_secret(secret, "criptografia segura")?;

    let version = options.version.unwrap_or(EncryptVersion::V3);
    let mut salt = [0_u8; TOKEN_SALT_LENGTH];
    let mut iv = [0_u8; TOKEN_IV_LENGTH];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut iv);

    let config = match version {
        EncryptVersion::V2 => LEGACY_SCRYPT,
        EncryptVersion::V3 => sanitize_scrypt_params(
            options.n,
            options.r,
            options.p,
            params::v3_fallback_config(),
        ),
    };

    let key = derive_key_for_encrypt(secret, &salt, config)?;
    let mut sealed = encrypt_payload(&key, &iv, plaintext.as_bytes())?;
    let tag = sealed.split_off(sealed.len() - TOKEN_TAG_LENGTH);
    let encoder = base64::engine::general_purpose::STANDARD;

    let salt_b64 = encoder.encode(salt);
    let iv_b64 = encoder.encode(iv);
    let tag_b64 = encoder.encode(tag);
    let ct_b64 = encoder.encode(sealed);

    Ok(match version {
        EncryptVersion::V2 => format!("v2:{salt_b64}:{iv_b64}:{tag_b64}:{ct_b64}:base64"),
        EncryptVersion::V3 => format!(
            "v3:{}:{}:{}:{salt_b64}:{iv_b64}:{tag_b64}:{ct_b64}:base64",
            config.n, config.r, config.p
        ),
    })
}

/// Decifra um token v1/v2/v3 e devolve o payload em texto.
///
/// Erros de formato mantêm as mensagens específicas do oráculo; qualquer
/// falha de derivação/autenticação vira [`CryptoError::Authentication`].
pub fn decrypt_session(token: &str, secret: &str) -> Result<String, CryptoError> {
    assert_valid_secret(secret, "descriptografia")?;
    if token.is_empty() {
        return Err(CryptoError::MissingToken);
    }

    let parsed = parse_session_token(token)?;
    let key = derive_key(secret, &parsed.salt, parsed.config)
        .map_err(|()| CryptoError::Authentication)?;
    decrypt_payload(&key, &parsed.iv, &parsed.tag, &parsed.ciphertext)
        .map_err(|()| CryptoError::Authentication)
}

fn derive_key(
    secret: &str,
    salt: &[u8],
    config: ScryptConfig,
) -> Result<Zeroizing<[u8; KEY_LENGTH]>, ()> {
    let scrypt_params = scrypt_params(config)?;
    let mut key = Zeroizing::new([0_u8; KEY_LENGTH]);
    scrypt::scrypt(secret.as_bytes(), salt, &scrypt_params, &mut *key).map_err(|_| ())?;
    Ok(key)
}

fn derive_key_for_encrypt(
    secret: &str,
    salt: &[u8],
    config: ScryptConfig,
) -> Result<Zeroizing<[u8; KEY_LENGTH]>, CryptoError> {
    let scrypt_params = scrypt_params(config).map_err(|()| {
        CryptoError::InvalidParams(format!(
            "Parâmetros scrypt inválidos (N={}, r={}, p={}).",
            config.n, config.r, config.p
        ))
    })?;
    let mut key = Zeroizing::new([0_u8; KEY_LENGTH]);
    scrypt::scrypt(secret.as_bytes(), salt, &scrypt_params, &mut *key).map_err(|_| {
        CryptoError::InvalidParams("Falha ao derivar a chave com scrypt.".to_owned())
    })?;
    Ok(key)
}

fn scrypt_params(config: ScryptConfig) -> Result<scrypt::Params, ()> {
    // O scrypt exige N potência de 2; tokens com N inválido viram falha de auth.
    if !config.n.is_power_of_two() {
        return Err(());
    }
    // O crate não recebe maxmem (removido na 0.11): aplicamos o mesmo teto do
    // oráculo antes de derivar, reproduzindo o comportamento do OpenSSL.
    let memory = 128 * u64::from(config.n) * u64::from(config.r) * u64::from(config.p)
        + 128 * u64::from(config.r) * u64::from(config.p);
    if memory > config.maxmem {
        return Err(());
    }
    let log_n = u8::try_from(config.n.trailing_zeros()).map_err(|_| ())?;
    scrypt::Params::new(log_n, config.r, config.p, KEY_LENGTH).map_err(|_| ())
}

fn encrypt_payload(
    key: &[u8; KEY_LENGTH],
    iv: &[u8; TOKEN_IV_LENGTH],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(&key[..])
        .map_err(|_| CryptoError::InvalidParams("Tamanho de chave inválido.".to_owned()))?;
    cipher
        .encrypt(Nonce::from_slice(iv), plaintext)
        .map_err(|_| CryptoError::InvalidParams("Falha ao cifrar o payload.".to_owned()))
}

fn decrypt_payload(
    key: &[u8; KEY_LENGTH],
    iv: &[u8],
    tag: &[u8],
    ciphertext: &[u8],
) -> Result<String, ()> {
    let cipher = Aes256Gcm::new_from_slice(&key[..]).map_err(|_| ())?;
    let mut sealed = Vec::with_capacity(ciphertext.len() + tag.len());
    sealed.extend_from_slice(ciphertext);
    sealed.extend_from_slice(tag);
    let plaintext = cipher
        .decrypt(Nonce::from_slice(iv), sealed.as_ref())
        .map_err(|_| ())?;
    Ok(String::from_utf8_lossy(&plaintext).into_owned())
}
