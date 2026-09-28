//! Parser tolerante dos tokens de sessão v1/v2/v3 (mesmas regras do oráculo).

use super::base64_lenient;
use super::params::{
    LEGACY_SCRYPT, ScryptConfig, js_parse_int, sanitize_scrypt_params, v3_fallback_config,
};
use super::{
    APP_SCRYPT_SALT_V1, CryptoError, TOKEN_IV_LENGTH, TOKEN_SALT_LENGTH, TOKEN_TAG_LENGTH,
};

/// Versão do token identificada no prefixo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenVersion {
    /// `v1:` — salt fixo.
    V1,
    /// `v2:` — salt aleatório e scrypt legado.
    V2,
    /// `v3:` — parâmetros embutidos ou compacto.
    V3,
}

/// Token parseado, com buffers decodificados e parâmetros sanitizados.
#[derive(Debug, Clone)]
pub struct ParsedToken {
    /// Versão do envelope.
    pub version: TokenVersion,
    /// Parâmetros scrypt derivados/sanitizados.
    pub config: ScryptConfig,
    /// Salt (fixo em v1).
    pub salt: Vec<u8>,
    /// Vetor de inicialização do GCM.
    pub iv: Vec<u8>,
    /// Tag de autenticação do GCM.
    pub tag: Vec<u8>,
    /// Texto cifrado.
    pub ciphertext: Vec<u8>,
}

/// Parseia o token aplicando as mesmas regras (e mensagens) do oráculo.
pub fn parse_session_token(token: &str) -> Result<ParsedToken, CryptoError> {
    let trimmed = token.trim();
    let parts: Vec<&str> = trimmed.split(':').collect();
    let version_tag = parts.first().copied().unwrap_or("");

    let (version, config, salt, iv, tag, ciphertext) = match version_tag {
        "v3" => {
            if parts.len() >= 8 {
                let raw_n = js_parse_int(parts[1])
                    .filter(|v| *v > 0)
                    .and_then(|v| u64::try_from(v).ok());
                let raw_r = js_parse_int(parts[2])
                    .filter(|v| *v > 0)
                    .and_then(|v| u64::try_from(v).ok());
                let raw_p = js_parse_int(parts[3])
                    .filter(|v| *v > 0)
                    .and_then(|v| u64::try_from(v).ok());
                let config = sanitize_scrypt_params(raw_n, raw_r, raw_p, v3_fallback_config());
                (
                    TokenVersion::V3,
                    config,
                    decode(parts[4]),
                    decode(parts[5]),
                    decode(parts[6]),
                    decode(parts[7]),
                )
            } else if parts.len() >= 5 {
                (
                    TokenVersion::V3,
                    v3_fallback_config(),
                    decode(parts[1]),
                    decode(parts[2]),
                    decode(parts[3]),
                    decode(parts[4]),
                )
            } else {
                return Err(CryptoError::TokenFormat(
                    "Formato de token v3 inválido. O token deve possuir blocos \
                     v3:N:r:p:salt:iv:tag:ciphertext:base64."
                        .to_owned(),
                ));
            }
        }
        "v2" => {
            if parts.len() < 5 {
                return Err(CryptoError::TokenFormat(
                    "Formato de token v2 inválido. O token deve possuir blocos \
                     v2:salt:iv:tag:ciphertext:base64."
                        .to_owned(),
                ));
            }
            (
                TokenVersion::V2,
                LEGACY_SCRYPT,
                decode(parts[1]),
                decode(parts[2]),
                decode(parts[3]),
                decode(parts[4]),
            )
        }
        "v1" => {
            if parts.len() < 4 {
                return Err(CryptoError::TokenFormat(
                    "Formato de token v1 inválido. O token deve possuir blocos \
                     v1:iv:tag:ciphertext:base64."
                        .to_owned(),
                ));
            }
            (
                TokenVersion::V1,
                LEGACY_SCRYPT,
                APP_SCRYPT_SALT_V1.to_vec(),
                decode(parts[1]),
                decode(parts[2]),
                decode(parts[3]),
            )
        }
        _ => {
            return Err(CryptoError::TokenFormat(
                "Formato de token de sessão inválido. O token deve iniciar com \"v1:\", \"v2:\" ou \"v3:\"."
                    .to_owned(),
            ));
        }
    };

    assert_token_buffer_lengths(version, &salt, &iv, &tag)?;

    Ok(ParsedToken {
        version,
        config,
        salt,
        iv,
        tag,
        ciphertext,
    })
}

/// Valida tamanhos canônicos; falha usa a mensagem pública de autenticação.
fn assert_token_buffer_lengths(
    version: TokenVersion,
    salt: &[u8],
    iv: &[u8],
    tag: &[u8],
) -> Result<(), CryptoError> {
    let needs_salt_check = matches!(version, TokenVersion::V2 | TokenVersion::V3);
    let invalid = iv.len() != TOKEN_IV_LENGTH
        || tag.len() != TOKEN_TAG_LENGTH
        || (needs_salt_check && salt.len() != TOKEN_SALT_LENGTH);
    if invalid {
        Err(CryptoError::Authentication)
    } else {
        Ok(())
    }
}

fn decode(part: &str) -> Vec<u8> {
    base64_lenient::decode_lenient(part)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v3_token() -> String {
        // Estrutura válida (conteúdo cifrado pode ser fictício; só o parse é testado).
        format!(
            "v3:16384:8:1:{}:{}:{}:{}:base64",
            "AAAAAAAAAAAAAAAAAAAAAA==", // 16 bytes
            "AAAAAAAAAAAAAAAA",         // 12 bytes
            "AAAAAAAAAAAAAAAAAAAAAA==", // 16 bytes
            "AAAAAA=="                  // 4 bytes
        )
    }

    #[test]
    fn parseia_v3_completo() {
        let parsed = parse_session_token(&v3_token()).expect("v3 válido");
        assert_eq!(parsed.version, TokenVersion::V3);
        assert_eq!(parsed.config.n, 16_384);
        assert_eq!(parsed.salt.len(), TOKEN_SALT_LENGTH);
        assert_eq!(parsed.iv.len(), TOKEN_IV_LENGTH);
        assert_eq!(parsed.tag.len(), TOKEN_TAG_LENGTH);
    }

    #[test]
    fn ignora_campos_extras_no_final() {
        let token = format!("{}:lixo:extra", v3_token());
        assert!(parse_session_token(&token).is_ok());
    }

    #[test]
    fn mensagens_de_formato_iguais_ao_oraculo() {
        let err = parse_session_token("v9:aa:bb").unwrap_err().to_string();
        assert!(err.starts_with("Formato de token de sessão inválido."));

        let err = parse_session_token("v1:").unwrap_err().to_string();
        assert!(err.starts_with("Formato de token v1 inválido."));

        let err = parse_session_token("v2:a:b:c").unwrap_err().to_string();
        assert!(err.starts_with("Formato de token v2 inválido."));

        let err = parse_session_token("v3:a:b:c").unwrap_err().to_string();
        assert!(err.starts_with("Formato de token v3 inválido."));
    }

    #[test]
    fn tamanhos_invalidos_viram_erro_de_autenticacao() {
        let err = parse_session_token("v3:16384:8:1:!!!!:!!!!:!!!!:!!!!:base64").unwrap_err();
        assert!(matches!(err, CryptoError::Authentication));
    }

    #[test]
    fn n_invalido_no_token_cai_no_fallback_sanitizado() {
        let token = format!(
            "v3:abc:xyz:0:{}:{}:{}:{}:base64",
            "AAAAAAAAAAAAAAAAAAAAAA==", "AAAAAAAAAAAAAAAA", "AAAAAAAAAAAAAAAAAAAAAA==", "AAAAAA=="
        );
        let parsed = parse_session_token(&token).expect("parse tolerante");
        assert!(parsed.config.n >= super::super::SCRYPT_MIN_N);
    }
}
