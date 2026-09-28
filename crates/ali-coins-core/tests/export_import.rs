//! Roundtrip export → import de tokens de sessão (interop Node já coberta em crypto).

use ali_coins_core::config::{Account, EnvSource};
use ali_coins_core::session::{
    SessionOptions, export_session_token, import_session_token, load_session_files,
    storage_filter::storage_state_json,
};

const SECRET: &str = "parity-test-secret-0123456789abcdef";
const USER: &str = "fulano@example.com";

fn account_for(dir: &std::path::Path, user: &str) -> Account {
    Account {
        index: 1,
        user: user.to_string(),
        session_path: dir.join("session.json"),
        session_meta_path: dir.join("session_meta.json"),
        ..Account::default()
    }
}

fn env() -> EnvSource {
    EnvSource::from_pairs([("SESSION_SECRET", SECRET)])
}

fn sample_state() -> serde_json::Value {
    storage_state_json(
        &[("xman_us_t", "auth-value")],
        &[(
            "https://www.aliexpress.com",
            &[("userInfo", "1"), ("aegis", "ruido")],
        )],
    )
}

#[test]
fn exporta_e_importa_token_entre_diretorios() {
    let dir_a = tempfile::tempdir().expect("dir a");
    let dir_b = tempfile::tempdir().expect("dir b");
    let options_a = SessionOptions::with_base_dir(dir_a.path().to_path_buf());
    let env = env();

    // Sessão cifrada na origem.
    ali_coins_core::session::save_session(&options_a, dir_a.path(), &env, sample_state(), USER)
        .expect("save")
        .expect("gravou");

    let token = export_session_token(&options_a, dir_a.path(), &env, USER).expect("exportou");
    assert!(token.starts_with("v3:"));

    // Importa no destino com o mesmo secret.
    let options_b = SessionOptions::with_base_dir(dir_b.path().to_path_buf());
    let outcome = import_session_token(
        &options_b,
        dir_b.path(),
        &env,
        &token,
        None,
        &[account_for(dir_b.path(), USER)],
    )
    .expect("importou");
    assert_eq!(outcome.user, USER);
    assert!(outcome.encrypted);

    let loaded = load_session_files(&options_b, dir_b.path(), &env).expect("load");
    let data = loaded.session_data.expect("sessão importada");
    assert_eq!(
        data["origins"][0]["localStorage"].as_array().unwrap().len(),
        1,
        "filtro de storage aplicado antes do envio"
    );
    let meta = loaded.meta_data.expect("meta");
    assert_eq!(meta.is_imported, Some(true));
    assert!(meta.imported_at.is_some());
    assert!(meta.exported_from.is_some());
    assert!(meta.expires_at.is_some());
}

#[test]
fn importacao_recusa_conta_divergente() {
    let dir_a = tempfile::tempdir().expect("dir a");
    let dir_b = tempfile::tempdir().expect("dir b");
    let options_a = SessionOptions::with_base_dir(dir_a.path().to_path_buf());
    let env = env();
    ali_coins_core::session::save_session(&options_a, dir_a.path(), &env, sample_state(), USER)
        .expect("save")
        .expect("gravou");
    let token = export_session_token(&options_a, dir_a.path(), &env, USER).expect("export");

    let options_b = SessionOptions::with_base_dir(dir_b.path().to_path_buf());
    let error = import_session_token(
        &options_b,
        dir_b.path(),
        &env,
        &token,
        Some("outro@example.com"),
        &[account_for(dir_b.path(), USER)],
    )
    .expect_err("deveria recusar");
    assert!(error.to_string().contains("não corresponde"));
}

#[test]
fn exportacao_sem_sessao_falha() {
    let dir = tempfile::tempdir().expect("dir");
    let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
    let error = export_session_token(&options, dir.path(), &env(), USER).expect_err("sem sessão");
    assert!(
        error
            .to_string()
            .contains("Nenhuma sessão ativa encontrada")
    );
}

#[test]
fn token_adulterado_falha_na_importacao() {
    let dir = tempfile::tempdir().expect("dir");
    let options = SessionOptions::with_base_dir(dir.path().to_path_buf());
    let error = import_session_token(
        &options,
        dir.path(),
        &env(),
        "v3:16384:8:1:aaaa:bbbb:cccc:dddd:base64",
        None,
        &[account_for(dir.path(), USER)],
    )
    .expect_err("token inválido");
    assert!(!error.to_string().is_empty());
}
