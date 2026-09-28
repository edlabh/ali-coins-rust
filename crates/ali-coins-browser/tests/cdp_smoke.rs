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
    let options = LaunchOptions {
        headless: true,
        args,
        executable_path: std::env::var("ALI_COINS_CHROME")
            .ok()
            .map(std::path::PathBuf::from),
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
