//! Helpers de navegação (equivalente a `libs/ui/navigation.js`).
//!
//! `goto_with_retry` (3 tentativas com backoff 2s→8s), `wait_and_click` com
//! fallback de clique via evaluate e `close_modals` que só clica em botões de
//! fechar dentro de diálogos/overlays.
//!
//! Os seletores exatos de fechamento do oráculo (`:has-text` etc.) serão
//! alinhados na fase de paridade com o site real (D-06).

use ali_coins_browser::driver::{BrowserError, NavOptions, Page};
use std::time::Duration;

/// Tentativas padrão de navegação.
pub const GOTO_ATTEMPTS: u32 = 3;
/// Base do backoff das tentativas (dobra a cada falha, teto de 8s).
pub const GOTO_BASE_DELAY_MS: u64 = 2_000;
/// Timeout padrão de navegação.
pub const NAV_TIMEOUT: Duration = Duration::from_secs(35);

/// Navega com retry e backoff exponencial (teto de 8s).
pub async fn goto_with_retry(page: &dyn Page, url: &str) -> Result<(), BrowserError> {
    goto_with_retry_options(page, url, GOTO_ATTEMPTS, GOTO_BASE_DELAY_MS).await
}

/// Navega com retry parametrizável (testes).
pub async fn goto_with_retry_options(
    page: &dyn Page,
    url: &str,
    attempts: u32,
    base_delay_ms: u64,
) -> Result<(), BrowserError> {
    goto_with_retry_timeout(page, url, attempts, base_delay_ms, NAV_TIMEOUT).await
}

/// Navega com retry e timeout explícito (valores dinâmicos do credentials.env).
pub async fn goto_with_retry_timeout(
    page: &dyn Page,
    url: &str,
    attempts: u32,
    base_delay_ms: u64,
    timeout: Duration,
) -> Result<(), BrowserError> {
    let attempts = attempts.max(1);
    let mut last_error: Option<BrowserError> = None;
    for attempt in 0..attempts {
        let result = page
            .goto(
                url,
                &NavOptions {
                    timeout: Some(timeout),
                    wait_until: None,
                },
            )
            .await;
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < attempts {
                    let delay = (base_delay_ms * 2_u64.pow(attempt)).min(8_000);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| BrowserError::Navigation(url.to_string())))
}

/// Aguarda o seletor e clica (com fallback de clique via evaluate).
pub async fn wait_and_click(
    page: &dyn Page,
    selector: &str,
    timeout: Duration,
) -> Result<(), BrowserError> {
    page.wait_for_selector(selector, timeout).await?;
    if page.click_selector(selector).await.is_ok() {
        return Ok(());
    }
    let script = format!(
        "const el = document.querySelector({}); if (el) {{ el.click(); true }} else {{ false }}",
        serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string())
    );
    let clicked = page.eval_raw(&script).await?;
    if clicked.as_bool() == Some(true) {
        Ok(())
    } else {
        Err(BrowserError::NotFound(selector.to_string()))
    }
}

/// Fecha diálogos/overlays abertos (clica apenas dentro das raízes conhecidas).
///
/// Retorna a quantidade de modais fechados.
pub async fn close_modals(page: &dyn Page) -> Result<u32, BrowserError> {
    let script = r#"(function () {
  const roots = document.querySelectorAll('[role="dialog"], .modal, [class*="modal"], .overlay, .mask, [class*="Overlay"], [class*="Modal"]');
  let closed = 0;
  for (const root of roots) {
    const candidates = root.querySelectorAll('button, [role="button"], [class*="close"], [aria-label]');
    for (const el of candidates) {
      const label = ((el.getAttribute && el.getAttribute('aria-label')) || '') + ' ' + ((el.className && String(el.className)) || '') + ' ' + (el.textContent || '');
      if (/close|fechar|cancel|cancelar|✕|×/i.test(label)) {
        el.click();
        closed++;
        break;
      }
    }
  }
  return closed;
})()"#;
    let value = page.eval_raw(script).await?;
    Ok(u32::try_from(value.as_u64().unwrap_or(0)).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::BrowserDriver as _;
    use ali_coins_browser::mock::{MockAction, MockDriver, MockPageSpec};
    use std::collections::HashMap;

    fn spec_with_selector(selector: &str) -> MockPageSpec {
        MockPageSpec {
            visible_selectors: vec![selector.to_string()],
            ..MockPageSpec::default()
        }
    }

    #[tokio::test]
    async fn retry_de_navegacao() {
        let driver = MockDriver::new(vec![MockPageSpec {
            fail_gotos: 2,
            ..MockPageSpec::default()
        }]);
        let browser = driver
            .launch(&ali_coins_browser::driver::LaunchOptions::default())
            .await
            .unwrap();
        let page = browser.new_page().await.unwrap();
        goto_with_retry_options(&*page, "https://exemplo/", 3, 1)
            .await
            .expect("3ª tentativa deveria funcionar");
        assert_eq!(page.url().await.unwrap(), "https://exemplo/");
        assert_eq!(driver.actions().len(), 3);
    }

    #[tokio::test]
    async fn retry_esgota_e_falha() {
        let driver = MockDriver::new(vec![MockPageSpec {
            fail_gotos: 5,
            ..MockPageSpec::default()
        }]);
        let browser = driver
            .launch(&ali_coins_browser::driver::LaunchOptions::default())
            .await
            .unwrap();
        let page = browser.new_page().await.unwrap();
        let error = goto_with_retry_options(&*page, "https://exemplo/", 2, 1)
            .await
            .expect_err("deveria falhar");
        assert!(matches!(error, BrowserError::Navigation(_)));
    }

    #[tokio::test]
    async fn aguarda_e_clica() {
        let driver = MockDriver::new(vec![spec_with_selector(".botao")]);
        let browser = driver
            .launch(&ali_coins_browser::driver::LaunchOptions::default())
            .await
            .unwrap();
        let page = browser.new_page().await.unwrap();
        wait_and_click(&*page, ".botao", Duration::from_millis(100))
            .await
            .expect("clicou");
        assert!(
            driver
                .actions()
                .contains(&MockAction::Click(".botao".to_string()))
        );

        let error = wait_and_click(&*page, ".ausente", Duration::from_millis(50))
            .await
            .expect_err("não existe");
        assert!(matches!(error, BrowserError::NotFound(_)));
    }

    #[tokio::test]
    async fn fecha_modais_pelo_retorno_do_script() {
        let driver = MockDriver::new(vec![MockPageSpec {
            eval_contains: vec![(
                "querySelectorAll('[role=\"dialog\"]".to_string(),
                serde_json::json!(2),
            )],
            selector_texts: HashMap::new(),
            ..MockPageSpec::default()
        }]);
        let browser = driver
            .launch(&ali_coins_browser::driver::LaunchOptions::default())
            .await
            .unwrap();
        let page = browser.new_page().await.unwrap();
        assert_eq!(close_modals(&*page).await.unwrap(), 2);
    }
}
