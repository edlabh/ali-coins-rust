//! Paridade de sessão contra fixtures geradas pelo oráculo Node.
//!
//! Fixtures: `tools/parity/fixtures/session/`
//! (gere com `./tools/parity/generate-fixtures.sh`).

use ali_coins_core::config::EnvSource;
use ali_coins_core::session::{
    SessionOptions, filter_storage_state, is_allowed_storage_key, load_session_files,
    should_filter_storage, validate_session,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

const SECRET: &str = "parity-test-secret-0123456789abcdef";
const USER: &str = "parity-fixture@example.com";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity/fixtures/session")
}

#[test]
fn storage_filter_igual_ao_oraculo() {
    let raw = std::fs::read_to_string(fixtures_dir().join("storage_filter.json"))
        .expect("rode ./tools/parity/generate-fixtures.sh");
    let doc: Value = serde_json::from_str(&raw).expect("fixture JSON");

    let allowed = doc["allowed"].as_object().expect("allowed");
    for (key, expected) in allowed {
        assert_eq!(
            is_allowed_storage_key(key),
            expected.as_bool().expect("bool"),
            "chave {key:?}"
        );
    }

    let input = doc["input"].clone();
    let expected_filtered = doc["filtered"].clone();
    assert_eq!(filter_storage_state(input), expected_filtered);

    for (value, expected) in doc["shouldFilterStorageEnv"]
        .as_object()
        .expect("env cases")
    {
        let env = if value == "__unset__" {
            EnvSource::default()
        } else {
            EnvSource::from_pairs([("SESSION_STRICT_STORAGE", value.as_str())])
        };
        let options = SessionOptions::default();
        assert_eq!(
            should_filter_storage(&options, &env),
            expected.as_bool().expect("bool"),
            "SESSION_STRICT_STORAGE={value:?}"
        );
    }

    let explicit_true = SessionOptions {
        filter_storage: Some(true),
        ..SessionOptions::default()
    };
    let explicit_false = SessionOptions {
        filter_storage: Some(false),
        ..SessionOptions::default()
    };
    assert_eq!(
        should_filter_storage(&explicit_true, &EnvSource::default()),
        doc["shouldFilterStorageOptions"]["true"].as_bool().unwrap()
    );
    assert_eq!(
        should_filter_storage(&explicit_false, &EnvSource::default()),
        doc["shouldFilterStorageOptions"]["false"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn le_sessao_em_texto_puro_gravada_pelo_oraculo() {
    let dir = fixtures_dir().join("plain");
    let mut options = SessionOptions::with_base_dir(dir.clone());
    options.encrypt_local_session = Some(false);
    let env = EnvSource::default();

    let loaded = load_session_files(&options, &dir, &env).expect("load");
    let data = loaded.session_data.expect("sessão presente");
    assert!(validate_session(&data, loaded.meta_data.as_ref(), Some(USER)).valid);
    let entries = data["origins"][0]["localStorage"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "filtro deve remover 'aegis'");
    assert_eq!(entries[0]["name"], "userInfo");
    assert_eq!(loaded.meta_data.unwrap().user.as_deref(), Some(USER));
}

#[test]
fn le_sessao_cifrada_gravada_pelo_oraculo() {
    let dir = fixtures_dir().join("encrypted");
    let mut options = SessionOptions::with_base_dir(dir.clone());
    options.secret = Some(SECRET.to_string());
    options.encrypt_local_session = Some(true);
    let env = EnvSource::default();

    let loaded = load_session_files(&options, &dir, &env).expect("load");
    let data = loaded.session_data.expect("sessão cifrada presente");
    assert!(validate_session(&data, loaded.meta_data.as_ref(), Some(USER)).valid);
    let meta = loaded.meta_data.expect("meta");
    assert_eq!(meta.encrypted, Some(true));
}

#[test]
fn rust_grava_e_oraculo_continua_lendo_o_formato() {
    // Grava com a implementação Rust e valida o formato que o Node espera
    // (JSON de storage state + meta com os mesmos campos).
    let dir = tempfile::tempdir().expect("tempdir");
    let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
    let env = EnvSource::from_pairs([("ENCRYPT_LOCAL_SESSION", "false")]);
    let state = serde_json::json!({
        "cookies": [
            { "name": "xman_us_t", "value": "auth-value", "domain": ".aliexpress.com", "path": "/" }
        ],
        "origins": [
            { "origin": "https://www.aliexpress.com", "localStorage": [
                { "name": "userInfo", "value": "1" },
                { "name": "aegis", "value": "ruido" }
            ] }
        ]
    });

    let saved = ali_coins_core::session::save_session(&options, dir.path(), &env, state, USER)
        .expect("save")
        .expect("gravou");
    assert_eq!(saved.meta.user.as_deref(), Some(USER));

    let raw = std::fs::read_to_string(dir.path().join("session.json")).expect("arquivo");
    let parsed: Value = serde_json::from_str(&raw).expect("JSON");
    assert!(parsed["cookies"].is_array());
    assert!(parsed["origins"].is_array());
    // O filtro de storage deve ter removido 'aegis'.
    assert_eq!(
        parsed["origins"][0]["localStorage"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let meta_raw = std::fs::read_to_string(dir.path().join("session_meta.json")).expect("meta");
    let meta: Value = serde_json::from_str(&meta_raw).expect("meta JSON");
    assert_eq!(meta["user"], USER);
    assert!(meta["savedAt"].is_string());
    assert_eq!(meta["encrypted"], false);
}
