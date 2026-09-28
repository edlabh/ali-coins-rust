//! Crash handler em processo filho: panic deve sair com código 6 e rodar cleanup.

use std::path::PathBuf;

#[test]
fn crash_helper_child() {
    let Ok(marker) = std::env::var("CRASH_MARKER_PATH") else {
        return;
    };
    ali_coins_core::exit::install_panic_handler(move || {
        let _ = std::fs::write(&marker, "cleanup-executado");
    });
    panic!("falha proposital para o teste de crash");
}

#[test]
fn panic_sai_com_codigo_6_e_executa_cleanup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("marker.txt");
    let exe = std::env::current_exe().expect("current_exe");

    let status = std::process::Command::new(&exe)
        .args(["--exact", "crash_helper_child", "--nocapture"])
        .env("CRASH_MARKER_PATH", &marker)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("executar filho");

    assert_eq!(status.code(), Some(6), "panic deve sair com exit code 6");
    let marker_content = std::fs::read_to_string(PathBuf::from(&marker)).unwrap_or_default();
    assert_eq!(marker_content, "cleanup-executado");
}
