//! Smoke test opt-in do `CdpDriver` (requer Chromium + libs de sistema).
//!
//! Rode com:
//! `ALI_COINS_CHROME=/caminho/para/chrome cargo test -p ali-coins-browser --test cdp_smoke -- --ignored`
//!
//! Em ambientes sem as libs do Chromium (ex.: `libnspr4.so`), instale primeiro:
//! `npx playwright install-deps chromium` (requer sudo) ou use um Chromium do sistema.

use ali_coins_browser::cdp::CdpDriver;
use ali_coins_browser::driver::{BrowserDriver, LaunchOptions, NavOptions};
use ali_coins_browser::launch::{ChromiumArgsInput, build_chromium_args, pixel7_profile};
use ali_coins_core::config::EnvSource;
use std::time::Duration;

#[tokio::test]
#[ignore = "requer Chromium instalado (rode com --ignored)"]
async fn abre_pagina_aplica_device_e_avalia() {
    let env = EnvSource::from_current_process();
    let args = build_chromium_args(&ChromiumArgsInput {
        env: &env,
        is_root: false,
        dev_shm_small: true,
        force_no_sandbox: false,
        low_memory: Some(true),
    });
    let profile_dir = tempfile::tempdir().expect("perfil");
    let options = LaunchOptions {
        headless: true,
        args,
        executable_path: std::env::var("ALI_COINS_CHROME")
            .ok()
            .map(std::path::PathBuf::from),
        user_data_dir: Some(profile_dir.path().to_path_buf()),
        ..LaunchOptions::default()
    };
    let driver = CdpDriver::new();
    let browser = driver.launch(&options).await.expect("launch");
    let page = browser.new_page().await.expect("page");
    page.set_device_profile(&pixel7_profile())
        .await
        .expect("device");

    page.goto(
        "data:text/html,<title>ali-coins</title><h1 id='t'>ok</h1>",
        &NavOptions {
            timeout: Some(Duration::from_secs(20)),
            wait_until: None,
        },
    )
    .await
    .expect("goto");

    assert_eq!(page.title().await.expect("title"), "ali-coins");
    let text = page.query_all_text("#t").await.expect("texto");
    assert_eq!(text, vec!["ok".to_string()]);
    let screenshot = page.screenshot().await.expect("screenshot");
    assert!(!screenshot.is_empty());

    let ua: String = ali_coins_browser::cdp::eval_typed(&*page, "navigator.userAgent")
        .await
        .expect("ua");
    assert!(ua.contains("Pixel 7"), "UA inesperado: {ua}");
}

#[tokio::test]
#[ignore = "requer Chromium instalado (rode com --ignored)"]
async fn bloqueia_recursos_e_gera_diagnosticos() {
    use ali_coins_browser::diagnostics::{
        DiagnosticsOptions, ScreenshotMode, capture_dom_artifacts, capture_screenshot,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Servidor local que conta conexões: a imagem NÃO deve chegar nele.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let hits_clone = std::sync::Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let _ = stream;
            hits_clone.fetch_add(1, Ordering::SeqCst);
        }
    });

    let env = EnvSource::from_current_process();
    let args = build_chromium_args(&ChromiumArgsInput {
        env: &env,
        is_root: false,
        dev_shm_small: true,
        force_no_sandbox: false,
        low_memory: Some(true),
    });
    let profile_dir = tempfile::tempdir().expect("perfil");
    let options = LaunchOptions {
        headless: true,
        args,
        executable_path: std::env::var("ALI_COINS_CHROME")
            .ok()
            .map(std::path::PathBuf::from),
        user_data_dir: Some(profile_dir.path().to_path_buf()),
        ..LaunchOptions::default()
    };
    let driver = CdpDriver::new();
    let browser = driver.launch(&options).await.expect("launch");
    let page = browser.new_page().await.expect("page");

    page.enable_resource_blocking(false)
        .await
        .expect("bloqueio");

    page.goto(
        &format!("data:text/html,<img src='http://127.0.0.1:{port}/x.png'>"),
        &NavOptions {
            timeout: Some(Duration::from_secs(20)),
            wait_until: None,
        },
    )
    .await
    .expect("goto");
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Diagnósticos em diretório temporário.
    let dir = tempfile::tempdir().expect("tempdir");
    let diag = DiagnosticsOptions {
        output_dir: dir.path().to_path_buf(),
        screenshot: ScreenshotMode::On,
        dump_dom: true,
    };
    let screenshot = capture_screenshot(&*page, &diag, "smoke", false)
        .await
        .expect("screenshot");
    assert!(screenshot.is_some());
    let hash = capture_dom_artifacts(&*page, &diag, "smoke")
        .await
        .expect("dom");
    assert_eq!(hash.len(), 64);
    assert!(dir.path().join("mobile_body.html").exists());
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "imagem deveria ser bloqueada"
    );
}

#[tokio::test]
#[ignore = "requer Chromium instalado (rode com --ignored)"]
async fn storage_state_ida_e_volta() {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    // Servidor local com uma página simples.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = "<html><body>ok</body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let env = EnvSource::from_current_process();
    let args = build_chromium_args(&ChromiumArgsInput {
        env: &env,
        is_root: false,
        dev_shm_small: true,
        force_no_sandbox: false,
        low_memory: Some(true),
    });
    let profile_dir = tempfile::tempdir().expect("perfil");
    let options = LaunchOptions {
        headless: true,
        args,
        executable_path: std::env::var("ALI_COINS_CHROME")
            .ok()
            .map(std::path::PathBuf::from),
        user_data_dir: Some(profile_dir.path().to_path_buf()),
        ..LaunchOptions::default()
    };
    let driver = CdpDriver::new();
    let browser = driver.launch(&options).await.expect("launch");
    let page = browser.new_page().await.expect("page");

    let base = format!("http://127.0.0.1:{port}");
    page.goto(&format!("{base}/"), &NavOptions::default())
        .await
        .expect("goto");

    let seed = serde_json::json!({
        "cookies": [
            { "name": "xman_us_t", "value": "auth-value", "domain": "127.0.0.1", "path": "/" }
        ],
        "origins": [
            { "origin": base, "localStorage": [ { "name": "userInfo", "value": "1" } ] }
        ]
    });
    page.seed_storage_state(&seed).await.expect("seed");

    let state = page.storage_state().await.expect("state");
    let cookies = state["cookies"].as_array().expect("cookies");
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0]["name"], "xman_us_t");
    assert_eq!(cookies[0]["value"], "auth-value");
    let origin = &state["origins"][0];
    assert_eq!(origin["origin"], base);
    let entries = origin["localStorage"].as_array().expect("localStorage");
    assert_eq!(entries[0]["name"], "userInfo");
    assert_eq!(entries[0]["value"], "1");
}
