//! Driver CDP de produção (chromiumoxide).
//!
//! Mapeia a interface `BrowserDriver` para o Chrome `DevTools Protocol` usando a
//! política de launch já portada (`launch.rs`) e aplica a emulação mobile
//! (Pixel 7) por página. Bloqueio de recursos e trace entram no incremento de
//! diagnósticos; aqui cobrimos navegação, evaluate, seletores, clique, scroll,
//! screenshot e perfil de device.

use super::driver::{Browser, BrowserDriver, BrowserError, LaunchOptions, NavOptions, Page};
use super::launch::DeviceProfile;
use ali_coins_core::logging;
use async_trait::async_trait;
use chromiumoxide::browser::{Browser as CdpBrowser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::{
    SetDeviceMetricsOverrideParams, SetEmitTouchEventsForMouseConfiguration,
    SetEmitTouchEventsForMouseParams, SetLocaleOverrideParams, SetTouchEmulationEnabledParams,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType, GestureSourceType, MouseButton,
    SynthesizeTapGestureParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, NavigateParams,
};
use chromiumoxide::page::{Page as CdpPage, ScreenshotParams};
use futures::StreamExt as _;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

/// Script de init que esconde o `navigator.webdriver` (igual ao oráculo).
pub const STEALTH_INIT_SCRIPT: &str = r"
Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
window.chrome = window.chrome || { runtime: {} };
";

/// Divide um argumento no formato do oráculo (`--chave` ou `--chave=valor`)
/// na chave e no valor esperados pelo chromiumoxide (que adiciona `--`).
fn parse_chromium_arg(argument: &str) -> (&str, Option<&str>) {
    let trimmed = argument.strip_prefix("--").unwrap_or(argument);
    match trimmed.split_once('=') {
        Some((key, value)) => (key, Some(value)),
        None => (trimmed, None),
    }
}

/// Driver de produção.
#[derive(Debug, Clone, Default)]
pub struct CdpDriver;

impl CdpDriver {
    /// Cria o driver.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn build_config(options: &LaunchOptions) -> Result<BrowserConfig, BrowserError> {
        let mut builder = BrowserConfig::builder()
            // VM pequena (1 vCPU): comandos CDP e launch podem demorar bem mais
            // que o default do chromiumoxide (30s).
            .request_timeout(Duration::from_secs(180))
            .launch_timeout(Duration::from_secs(90));
        builder = if options.headless {
            builder.new_headless_mode()
        } else {
            builder.with_head()
        };
        if !options.args.is_empty() {
            // O chromiumoxide prefixa `--` a cada argumento: enviamos a chave
            // (e o valor) sem os hífens para não gerar `----flag`, que o Chrome
            // ignora silenciosamente (ex.: `--no-sandbox` virava `----no-sandbox`).
            let mut flags: Vec<&str> = Vec::new();
            let mut pairs: Vec<(&str, &str)> = Vec::new();
            for arg in &options.args {
                let (key, value) = parse_chromium_arg(arg);
                match value {
                    Some(value) => pairs.push((key, value)),
                    None => flags.push(key),
                }
            }
            builder = builder.args(flags).args(pairs);
        }
        if !options.env.is_empty() {
            builder = builder.envs(options.env.iter().cloned());
        }
        if let Some(executable) = &options.executable_path {
            builder = builder.chrome_executable(executable);
        }
        if let Some(user_data_dir) = &options.user_data_dir {
            builder = builder.user_data_dir(user_data_dir);
        }
        builder.build().map_err(BrowserError::Launch)
    }
}

/// Browser CDP iniciado.
pub struct CdpBrowserHandle {
    browser: CdpBrowser,
    /// Origins http(s) visitados por **qualquer página** deste browser
    /// (o `storageState()` do Playwright enumera todos os origins do contexto).
    visited_origins: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

/// Página CDP.
pub struct CdpPageHandle {
    page: CdpPage,
    /// Origins http(s) visitados nesta página (para o storage state multi-origin).
    visited_origins: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Trace CDP ativo (diagnóstico D-03).
    trace: std::sync::Mutex<Option<crate::trace::Trace>>,
}

#[async_trait]
impl BrowserDriver for CdpDriver {
    async fn launch(&self, options: &LaunchOptions) -> Result<Box<dyn Browser>, BrowserError> {
        let config = Self::build_config(options)?;
        let (browser, mut handler) = CdpBrowser::launch(config)
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        // O handler precisa ser drenado para o browser responder aos comandos.
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });
        Ok(Box::new(CdpBrowserHandle {
            browser,
            visited_origins: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }))
    }
}

#[async_trait]
impl Browser for CdpBrowserHandle {
    async fn new_page(&self) -> Result<Box<dyn Page>, BrowserError> {
        let page = self
            .browser
            .new_page("about:blank")
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        // Init script anti-detecção.
        page.execute(AddScriptToEvaluateOnNewDocumentParams::new(
            STEALTH_INIT_SCRIPT,
        ))
        .await
        .map_err(|err| BrowserError::Launch(err.to_string()))?;
        Ok(Box::new(CdpPageHandle {
            page,
            visited_origins: std::sync::Arc::clone(&self.visited_origins),
            trace: std::sync::Mutex::new(None),
        }))
    }

    async fn pages(&self) -> Result<Vec<Box<dyn Page>>, BrowserError> {
        let pages = self
            .browser
            .pages()
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        Ok(pages
            .into_iter()
            .map(|page| {
                Box::new(CdpPageHandle {
                    page,
                    visited_origins: std::sync::Arc::clone(&self.visited_origins),
                    trace: std::sync::Mutex::new(None),
                }) as Box<dyn Page>
            })
            .collect())
    }

    async fn version(&self) -> Result<String, BrowserError> {
        self.browser
            .version()
            .await
            .map(|version| format!("{version:?}"))
            .map_err(|err| BrowserError::Launch(err.to_string()))
    }

    async fn close(mut self: Box<Self>) -> Result<(), BrowserError> {
        self.browser
            .close()
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        Ok(())
    }
}

/// Teto por chamada CDP (evita comandos pendurados travando o run).
const CDP_CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// Teto para consultas de elemento no DOM.
const FIND_TIMEOUT: Duration = Duration::from_secs(4);

/// Executa um comando CDP com teto de tempo, mapeando erro/timeout.
async fn bounded_cdp<T, E, F>(what: &str, future: F) -> Result<T, BrowserError>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = Result<T, E>>,
{
    bounded_cdp_with(what, CDP_CALL_TIMEOUT, future).await
}

/// Como `bounded_cdp`, com teto customizado (operações mais pesadas).
async fn bounded_cdp_with<T, E, F>(
    what: &str,
    timeout: Duration,
    future: F,
) -> Result<T, BrowserError>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = Result<T, E>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(BrowserError::Evaluate(format!("{what}: {error}"))),
        Err(_) => Err(BrowserError::Evaluate(format!(
            "{what}: sem resposta em {}s (CDP pendurado)",
            timeout.as_secs()
        ))),
    }
}

#[async_trait]
impl Page for CdpPageHandle {
    async fn goto(&self, url: &str, options: &NavOptions) -> Result<(), BrowserError> {
        // Equivalente a waitUntil: 'domcontentloaded' do oráculo, com polling de
        // readyState: não espera o "load" completo (páginas pesadas em VPS
        // pequena demoram dezenas de segundos) nem depende de comando CDP
        // bloqueado pelo renderer ocupado.
        let timeout = options.timeout.unwrap_or(Duration::from_secs(35));
        let started = Instant::now();
        // Em VPS pequena o `Page.navigate` pode não responder enquanto o
        // renderer está ocupado; a navegação segue e o polling confirma.
        match tokio::time::timeout(
            Duration::from_secs(20),
            self.page.execute(NavigateParams::new(url)),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => return Err(BrowserError::Navigation(err.to_string())),
            Err(_) => {
                logging::global().warn(
                    &format!("Page.navigate sem resposta em 20s; aguardando readyState de {url}"),
                    &[],
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let deadline = Instant::now() + timeout;
        loop {
            // Cada sondagem tem teto próprio para não travar se o renderer
            // estiver ocupado (comandos CDP enfileirados).
            let probe = tokio::time::timeout(
                Duration::from_secs(5),
                self.page.evaluate_expression("document.readyState"),
            )
            .await;
            if let Ok(Ok(result)) = probe {
                if let Ok(state) = result.into_value::<String>() {
                    if state == "interactive" || state == "complete" {
                        logging::global().info(
                            &format!(
                                "navegação pronta ({state}) para {url} em {}ms",
                                started.elapsed().as_millis()
                            ),
                            &[],
                        );
                        self.remember_origin().await;
                        return Ok(());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(BrowserError::Timeout(format!(
                    "goto {url} (readyState não atingido em {}ms)",
                    started.elapsed().as_millis()
                )));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        bounded_cdp(
            "voltar no histórico",
            self.page.evaluate_expression("history.back()"),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(())
    }

    async fn url(&self) -> Result<String, BrowserError> {
        self.page
            .url()
            .await
            .map(Option::unwrap_or_default)
            .map_err(|err| BrowserError::Navigation(err.to_string()))
    }

    async fn title(&self) -> Result<String, BrowserError> {
        let result = self.eval_raw("document.title").await?;
        Ok(result.as_str().unwrap_or_default().to_string())
    }

    async fn content(&self) -> Result<String, BrowserError> {
        let result = self.eval_raw("document.documentElement.outerHTML").await?;
        Ok(result.as_str().unwrap_or_default().to_string())
    }

    async fn eval_raw(&self, script: &str) -> Result<Value, BrowserError> {
        let result = bounded_cdp("avaliar script", self.page.evaluate_expression(script)).await?;
        Ok(result.into_value().unwrap_or(Value::Null))
    }

    async fn wait_for_selector(
        &self,
        selector: &str,
        timeout: Duration,
    ) -> Result<(), BrowserError> {
        let deadline = Instant::now() + timeout;
        loop {
            if bounded_cdp("buscar elemento", self.page.find_element(selector))
                .await
                .is_ok()
                && selector_visible(&self.page, selector).await
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(BrowserError::Timeout(format!("seletor {selector}")));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError> {
        let script = format!(
            "Array.from(document.querySelectorAll({})).map(e => e.textContent)",
            serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string())
        );
        let value = self.eval_raw(&script).await?;
        serde_json::from_value(value).map_err(|err| BrowserError::Evaluate(err.to_string()))
    }

    async fn click_selector(&self, selector: &str) -> Result<(), BrowserError> {
        if bounded_cdp(
            "buscar elemento para clique",
            self.page.find_element(selector),
        )
        .await
        .is_err()
        {
            return Err(BrowserError::NotFound(selector.to_string()));
        }
        let script = format!(
            "const el = document.querySelector({}); if (el) {{ el.click(); true }} else {{ false }}",
            serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string())
        );
        let clicked = self.eval_raw(&script).await?;
        if clicked.as_bool() == Some(true) {
            Ok(())
        } else {
            Err(BrowserError::NotFound(selector.to_string()))
        }
    }

    async fn tap_at(&self, x: f64, y: f64) -> Result<(), BrowserError> {
        let mut tap = SynthesizeTapGestureParams::new(x, y);
        tap.duration = Some(60);
        tap.tap_count = Some(1);
        tap.gesture_source_type = Some(GestureSourceType::Touch);
        bounded_cdp("toque (touch)", self.page.execute(tap)).await?;
        Ok(())
    }

    async fn mouse_move(&self, x: f64, y: f64, drag: bool) -> Result<(), BrowserError> {
        let mut event = DispatchMouseEventParams::new(DispatchMouseEventType::MouseMoved, x, y);
        event.buttons = Some(i64::from(drag));
        bounded_cdp("mover mouse", self.page.execute(event)).await?;
        Ok(())
    }

    async fn mouse_down(&self, x: f64, y: f64) -> Result<(), BrowserError> {
        let mut event = DispatchMouseEventParams::new(DispatchMouseEventType::MousePressed, x, y);
        event.button = Some(MouseButton::Left);
        event.buttons = Some(1);
        event.click_count = Some(1);
        bounded_cdp("pressionar mouse", self.page.execute(event)).await?;
        Ok(())
    }

    async fn mouse_up(&self, x: f64, y: f64) -> Result<(), BrowserError> {
        let mut event = DispatchMouseEventParams::new(DispatchMouseEventType::MouseReleased, x, y);
        event.button = Some(MouseButton::Left);
        event.buttons = Some(0);
        event.click_count = Some(1);
        bounded_cdp("pressionar mouse", self.page.execute(event)).await?;
        Ok(())
    }

    async fn click_at_with_modifiers(
        &self,
        x: f64,
        y: f64,
        modifiers: i64,
    ) -> Result<(), BrowserError> {
        // O Playwright move o mouse antes do clique (hover); alguns handlers do
        // site só reagem ao clique após o `mousemove`.
        let mut moved = DispatchMouseEventParams::new(DispatchMouseEventType::MouseMoved, x, y);
        moved.buttons = Some(0);
        bounded_cdp("mover mouse", self.page.execute(moved)).await?;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let mut press = DispatchMouseEventParams::new(DispatchMouseEventType::MousePressed, x, y);
        press.button = Some(MouseButton::Left);
        press.buttons = Some(1);
        press.click_count = Some(1);
        press.modifiers = Some(modifiers);
        bounded_cdp("clique (press)", self.page.execute(press)).await?;
        let mut release =
            DispatchMouseEventParams::new(DispatchMouseEventType::MouseReleased, x, y);
        release.button = Some(MouseButton::Left);
        release.buttons = Some(0);
        release.click_count = Some(1);
        release.modifiers = Some(modifiers);
        bounded_cdp("clique (release)", self.page.execute(release)).await?;
        Ok(())
    }

    async fn scroll_by(&self, x: i64, y: i64) -> Result<(), BrowserError> {
        bounded_cdp(
            "rolar página",
            self.page
                .evaluate_expression(format!("window.scrollBy({x}, {y})")),
        )
        .await?;
        Ok(())
    }

    async fn enable_resource_blocking(&self, allow_media: bool) -> Result<(), BrowserError> {
        super::network::enable_resource_blocking(&self.page, allow_media).await
    }

    async fn storage_state(&self) -> Result<Value, BrowserError> {
        let visited = self
            .visited_origins
            .lock()
            .map(|origins| origins.clone())
            .unwrap_or_default();
        bounded_cdp_with(
            "ler storage state",
            Duration::from_secs(30),
            super::storage::read_storage_state(&self.page, &visited),
        )
        .await
    }

    async fn seed_storage_state(&self, state: &Value) -> Result<(), BrowserError> {
        bounded_cdp_with(
            "semear storage state",
            Duration::from_secs(30),
            super::storage::seed_storage_state(&self.page, state),
        )
        .await
    }

    async fn set_device_profile(&self, profile: &DeviceProfile) -> Result<(), BrowserError> {
        bounded_cdp_with(
            "aplicar perfil de device",
            Duration::from_secs(30),
            apply_device_profile(&self.page, profile),
        )
        .await
    }

    async fn screenshot(&self) -> Result<Vec<u8>, BrowserError> {
        bounded_cdp(
            "capturar screenshot",
            self.page.screenshot(ScreenshotParams::default()),
        )
        .await
    }

    async fn start_trace(&self) -> Result<bool, BrowserError> {
        let trace = crate::trace::Trace::start(&self.page).await?;
        if let Ok(mut guard) = self.trace.lock() {
            *guard = Some(trace);
        }
        Ok(true)
    }

    async fn stop_trace(
        &self,
        output_dir: &std::path::Path,
        name: &str,
        keep: bool,
    ) -> Result<Option<std::path::PathBuf>, BrowserError> {
        let trace = self.trace.lock().ok().and_then(|mut guard| guard.take());
        let Some(trace) = trace else {
            return Ok(None);
        };
        trace.stop(&self.page, output_dir, name, keep).await
    }

    async fn close(&self) -> Result<(), BrowserError> {
        self.page
            .clone()
            .close()
            .await
            .map_err(|err| BrowserError::PageClosed(err.to_string()))?;
        Ok(())
    }
}

impl CdpPageHandle {
    /// Registra o origin atual (best-effort) para o storage state multi-origin.
    async fn remember_origin(&self) {
        let Ok(result) = bounded_cdp(
            "ler origin",
            self.page.evaluate_expression("location.origin"),
        )
        .await
        else {
            return;
        };
        let Ok(origin) = result.into_value::<String>() else {
            return;
        };
        if !origin.starts_with("http") {
            return;
        }
        if let Ok(mut origins) = self.visited_origins.lock() {
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
    }
}

/// O elemento do seletor está visível? (semântica do Playwright `state: visible`).
async fn selector_visible(page: &CdpPage, selector: &str) -> bool {
    let script = format!(
        "(() => {{ const el = document.querySelector({}); if (!el) return false; \
         if (typeof el.checkVisibility === 'function') return el.checkVisibility({{ checkOpacity: true, checkVisibilityCSS: true }}); \
         const rect = el.getBoundingClientRect(); \
         return rect.width > 0 && rect.height > 0 && el.offsetParent !== null; }})()",
        serde_json::to_string(selector).unwrap_or_default()
    );
    match tokio::time::timeout(FIND_TIMEOUT, page.evaluate_expression(script)).await {
        Ok(Ok(result)) => result.into_value::<bool>().ok().unwrap_or(false),
        _ => false,
    }
}

/// Aplica o perfil mobile (device metrics + touch + UA/locale) numa página CDP.
pub async fn apply_device_profile(
    page: &CdpPage,
    profile: &DeviceProfile,
) -> Result<(), BrowserError> {
    page.execute(
        SetDeviceMetricsOverrideParams::builder()
            .width(i64::from(profile.viewport.width))
            .height(i64::from(profile.viewport.height))
            .device_scale_factor(profile.device_scale_factor)
            .mobile(profile.is_mobile)
            .build()
            .map_err(|err| BrowserError::Launch(err.clone()))?,
    )
    .await
    .map_err(|err| BrowserError::Launch(err.to_string()))?;

    page.execute(SetTouchEmulationEnabledParams::new(profile.has_touch))
        .await
        .map_err(|err| BrowserError::Launch(err.to_string()))?;
    if profile.has_touch {
        // Conversão de mouse→toque (device mode) **desligada por padrão**: o
        // Playwright emite mouse em `click()` (gaveta de tarefas, cards da
        // surpresa) e toque apenas em `tap()`. Com a conversão ligada, o site
        // ignora os cliques/toques (gaveta não abre e `tracking` não sobe).
        // Mantida atrás de `PW_EMIT_TOUCH=1` para depuração/compatibilidade.
        let emit_touch = std::env::var("PW_EMIT_TOUCH").is_ok_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        });
        if emit_touch {
            let mut emit = SetEmitTouchEventsForMouseParams::new(true);
            emit.configuration = Some(SetEmitTouchEventsForMouseConfiguration::Mobile);
            page.execute(emit)
                .await
                .map_err(|err| BrowserError::Launch(err.to_string()))?;
        }
    }

    page.set_user_agent(profile.user_agent.as_str())
        .await
        .map_err(|err| BrowserError::Launch(err.to_string()))?;
    page.emulate_locale(SetLocaleOverrideParams {
        locale: Some(profile.locale.clone()),
    })
    .await
    .map_err(|err| BrowserError::Launch(err.to_string()))?;
    Ok(())
}

/// Helper tipado para `eval_raw` (evita genéricos no trait).
pub async fn eval_typed<T: DeserializeOwned>(
    page: &dyn Page,
    script: &str,
) -> Result<T, BrowserError> {
    let value = page.eval_raw(script).await?;
    serde_json::from_value(value).map_err(|err| BrowserError::Evaluate(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::parse_chromium_arg;

    #[test]
    fn parse_flag_sem_valor() {
        assert_eq!(parse_chromium_arg("--no-sandbox"), ("no-sandbox", None));
        assert_eq!(parse_chromium_arg("no-zygote"), ("no-zygote", None));
    }

    #[test]
    fn parse_chave_valor() {
        assert_eq!(
            parse_chromium_arg("--disable-dev-shm-usage"),
            ("disable-dev-shm-usage", None)
        );
        assert_eq!(
            parse_chromium_arg("--disable-features=Translate,AcceptCHFrame"),
            ("disable-features", Some("Translate,AcceptCHFrame"))
        );
    }

    #[test]
    fn parse_valor_com_hifens() {
        assert_eq!(
            parse_chromium_arg("--js-flags=--max-old-space-size=128"),
            ("js-flags", Some("--max-old-space-size=128"))
        );
    }
}
