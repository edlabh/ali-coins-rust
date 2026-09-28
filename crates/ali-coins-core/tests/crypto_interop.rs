//! Testes de interoperabilidade com fixtures geradas pelo oráculo Node.
//!
//! Fixture: `tools/parity/fixtures/crypto/tokens.json`
//! (gere com `./tools/parity/generate-fixtures.sh`).

use ali_coins_core::crypto::{
    CryptoError, EncryptOptions, EncryptVersion, decrypt_session, effective_default_scrypt_n,
    encrypt_session, parse_session_token,
};
use serde_json::Value;

const SECRET: &str = "parity-test-secret-0123456789abcdef";

fn fixtures() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/parity/fixtures/crypto/tokens.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "fixture não encontrada em {}: {err}\nRode: ./tools/parity/generate-fixtures.sh",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("fixture JSON válida")
}

#[test]
fn decifra_tokens_gerados_pelo_oraculo_em_todas_as_versoes() {
    let doc = fixtures();
    let secret = doc["secret"].as_str().expect("secret");
    let plaintext = doc["plaintext"].as_str().expect("plaintext");

    for name in ["v1", "v2", "v3", "v3default"] {
        let token = doc["tokens"][name]
            .as_str()
            .unwrap_or_else(|| panic!("{name}"));
        let out = decrypt_session(token, secret).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(out, plaintext, "payload divergente em {name}");
    }
}

#[test]
fn decifra_token_v3_compacto_quando_o_default_da_maquina_confere() {
    let doc = fixtures();
    let fixture_n = u32::try_from(doc["scryptDefaultN"].as_u64().expect("scryptDefaultN"))
        .expect("scryptDefaultN cabe em u32");
    if fixture_n != effective_default_scrypt_n() {
        eprintln!(
            "pulando v3 compacto: fixture N={fixture_n}, máquina N={}",
            effective_default_scrypt_n()
        );
        return;
    }
    let secret = doc["secret"].as_str().expect("secret");
    let plaintext = doc["plaintext"].as_str().expect("plaintext");
    let token = doc["tokens"]["v3compact"].as_str().expect("v3compact");
    let out = decrypt_session(token, secret).expect("v3 compacto");
    assert_eq!(out, plaintext);
}

#[test]
fn tokens_malformados_falham_com_erro_tipado_sem_panic() {
    let doc = fixtures();
    let secret = doc["secret"].as_str().expect("secret");
    let valid_v3 = doc["tokens"]["v3"].as_str().expect("v3");
    let malformed = doc["malformed"].as_array().expect("malformed");

    for token in malformed {
        let token = token.as_str().expect("string");
        match decrypt_session(token, secret) {
            // O parser do oráculo ignora campos extras após o ciphertext.
            Ok(_out) => assert!(
                token.starts_with(&format!("{valid_v3}:")),
                "token inesperadamente aceito: {token}"
            ),
            Err(err) => assert!(
                matches!(
                    err,
                    CryptoError::TokenFormat(_)
                        | CryptoError::Authentication
                        | CryptoError::MissingToken
                ),
                "erro inesperado para {token:?}: {err}"
            ),
        }
    }
}

#[test]
fn segredo_invalido_e_token_vazio_tem_mensagens_do_oraculo() {
    let err = decrypt_session("v3:aa", "curto").unwrap_err().to_string();
    assert!(err.contains("no mínimo 32 caracteres para descriptografia"));

    let err = encrypt_session("{}", "curto", &EncryptOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("no mínimo 32 caracteres para criptografia segura"));

    let err = decrypt_session("", SECRET).unwrap_err();
    assert!(matches!(err, CryptoError::MissingToken));

    let err = decrypt_session(
        "v1:aa:bb:cc:base64",
        "outro-segredo-com-mais-de-32-caracteres!",
    )
    .unwrap_err();
    assert!(matches!(err, CryptoError::Authentication));
}

#[test]
fn roundtrip_rust_v2_v3_e_compacto() {
    let plaintext = r#"{"cookies":[{"name":"a","value":"b"}],"origins":[]}"#;

    for options in [
        EncryptOptions {
            version: Some(EncryptVersion::V2),
            ..EncryptOptions::default()
        },
        EncryptOptions {
            n: Some(16_384),
            r: Some(8),
            p: Some(1),
            ..EncryptOptions::default()
        },
        EncryptOptions::default(),
    ] {
        let token = encrypt_session(plaintext, SECRET, &options).expect("encrypt");
        assert_eq!(decrypt_session(&token, SECRET).expect("decrypt"), plaintext);
    }

    // Formato compacto: mesmos buffers, sem os marcadores N:r:p.
    let token = encrypt_session(plaintext, SECRET, &EncryptOptions::default()).expect("v3");
    let parts: Vec<&str> = token.split(':').collect();
    let compact = format!(
        "v3:{}:{}:{}:{}:base64",
        parts[4], parts[5], parts[6], parts[7]
    );
    assert_eq!(
        decrypt_session(&compact, SECRET).expect("compacto"),
        plaintext
    );

    // Segredo errado não decifra.
    let err = decrypt_session(&token, "segredo-errado-com-mais-de-32-caracteres!!").unwrap_err();
    assert!(matches!(err, CryptoError::Authentication));
}

#[test]
fn parser_aceita_campos_extras_apos_o_ciphertext() {
    let doc = fixtures();
    let valid_v3 = doc["tokens"]["v3"].as_str().expect("v3");
    let with_extra = format!("{valid_v3}:lixo:extra");
    let parsed = parse_session_token(&with_extra).expect("campos extras são ignorados");
    assert!(!parsed.ciphertext.is_empty());
}
