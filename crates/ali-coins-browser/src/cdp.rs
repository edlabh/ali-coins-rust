//! Driver CDP de produção (chromiumoxide).
//!
//! Mapeia a interface `BrowserDriver` para o Chrome `DevTools Protocol` usando a
//! política de launch já portada (`launch.rs`) e aplica a emulação mobile
//! (Pixel 7) por página. Bloqueio de recursos e trace entram no incremento de
//! diagnósticos; aqui cobrimos navegação, evaluate, seletores, clique, scroll,
//! screenshot e perfil de device.

use super::driver::{Browser, BrowserDriver, BrowserError, LaunchOptions, NavOptions, Page};
use super::launch::DeviceProfile;
use async_trait::async_trait;
use chromiumoxide::browser::{Browser as CdpBrowser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::{
    SetDeviceMetricsOverrideParams, SetLocaleOverrideParams, SetTouchEmulationEnabledParams,
};
use chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams;
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
        let mut builder = BrowserConfig::builder();
        builder = if options.headless {
            builder.new_headless_mode()
        } else {
            builder.with_head()
        };
        if !options.args.is_empty() {
            builder = builder.args(options.args.iter().cloned());
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
}

/// Página CDP.
pub struct CdpPageHandle {
    page: CdpPage,
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
        Ok(Box::new(CdpBrowserHandle { browser }))
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
        Ok(Box::new(CdpPageHandle { page }))
    }

    async fn pages(&self) -> Result<Vec<Box<dyn Page>>, BrowserError> {
        let pages = self
            .browser
            .pages()
            .await
            .map_err(|err| BrowserError::Launch(err.to_string()))?;
        Ok(pages
            .into_iter()
            .map(|page| Box::new(CdpPageHandle { page }) as Box<dyn Page>)
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

#[async_trait]
impl Page for CdpPageHandle {
    async fn goto(&self, url: &str, options: &NavOptions) -> Result<(), BrowserError> {
        let timeout = options.timeout.unwrap_or(Duration::from_secs(35));
        let navigation = self.page.goto(url);
        match tokio::time::timeout(timeout, navigation).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) => Err(BrowserError::Navigation(err.to_string())),
            Err(_) => Err(BrowserError::Timeout(format!("goto {url}"))),
        }
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        self.page
            .evaluate_expression("history.back()")
            .await
            .map_err(|err| BrowserError::Evaluate(err.to_string()))?;
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
        let result = self
            .page
            .evaluate_expression(script)
            .await
            .map_err(|err| BrowserError::Evaluate(err.to_string()))?;
        Ok(result.into_value().unwrap_or(Value::Null))
    }

    async fn wait_for_selector(
        &self,
        selector: &str,
        timeout: Duration,
    ) -> Result<(), BrowserError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.page.find_element(selector).await.is_ok() {
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
        if self.page.find_element(selector).await.is_err() {
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

    async fn scroll_by(&self, x: i64, y: i64) -> Result<(), BrowserError> {
        self.page
            .evaluate_expression(format!("window.scrollBy({x}, {y})"))
            .await
            .map_err(|err| BrowserError::Evaluate(err.to_string()))?;
        Ok(())
    }

    async fn enable_resource_blocking(&self, allow_media: bool) -> Result<(), BrowserError> {
        super::network::enable_resource_blocking(&self.page, allow_media).await
    }

    async fn storage_state(&self) -> Result<Value, BrowserError> {
        super::storage::read_storage_state(&self.page).await
    }

    async fn seed_storage_state(&self, state: &Value) -> Result<(), BrowserError> {
        super::storage::seed_storage_state(&self.page, state).await
    }

    async fn set_device_profile(&self, profile: &DeviceProfile) -> Result<(), BrowserError> {
        apply_device_profile(&self.page, profile).await
    }

    async fn screenshot(&self) -> Result<Vec<u8>, BrowserError> {
        self.page
            .screenshot(ScreenshotParams::default())
            .await
            .map_err(|err| BrowserError::Io(err.to_string()))
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
