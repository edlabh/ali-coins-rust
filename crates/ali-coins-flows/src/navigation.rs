//! Helpers de navegação (equivalente a `libs/ui/navigation.js`).
//!
//! `goto_with_retry` (3 tentativas com backoff 2s→8s), `wait_and_click` com
//! fallback de clique via evaluate e `close_modals` que só clica em botões de
//! fechar dentro de diálogos/overlays.
//!
//! D-06/D-07 concluídos: `close_modals` usa a lista exata de seletores do
//! oráculo (CSS + `:has-text`) e o slider anti-bot é resolvido com movimento
//! humanizado (easing + jitter + frames).

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

/// Seletores de fechamento do oráculo (`SELECTORS.modals.closeButtons`).
pub const CLOSE_BUTTONS: [&str; 8] = [
    "[class*=\"close\"]",
    "[class*=\"dialog-close\"]",
    "[class*=\"aecoin-close\"]",
    ".ui-dialog-close",
    "button:has-text(\"OK\")",
    "button:has-text(\"Confirm\")",
    "button:has-text(\"Confirmar\")",
    "button:has-text(\"Fechar\")",
];

/// Raízes consideradas "diálogo/overlay" para fechar somente em contexto.
const DIALOG_CLOSEST: &str = "[role=\"dialog\"], [class*=\"modal\"], [class*=\"dialog\"], [class*=\"popup\"], [class*=\"toast\"], [class*=\"overlay\"], [class*=\"mask\"]";

/// Separa `base:has-text("texto")` em `(base, texto)` (port do filtro do oráculo).
#[must_use]
pub fn split_has_text(selector: &str) -> Option<(&str, &str)> {
    let marker = ":has-text(";
    let index = selector.find(marker)?;
    let base = selector[..index].trim();
    let rest = &selector[index + marker.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some((base, &rest[..end]))
}

/// Clica o primeiro elemento visível que casa o seletor (suporta `:has-text`).
pub async fn click_selector_or_text(page: &dyn Page, selector: &str) -> bool {
    let script = if let Some((base, text)) = split_has_text(selector) {
        format!(
            "(() => {{ const nodes = document.querySelectorAll({base}); \
             for (const el of nodes) {{ const r = el.getBoundingClientRect(); \
             if (r.width <= 0 || r.height <= 0) continue; \
             if (!(el.textContent || '').toLowerCase().includes({text})) continue; \
             el.click(); return true; }} return false; }})()",
            base = serde_json::to_string(base).unwrap_or_else(|_| "\"\"".to_string()),
            text =
                serde_json::to_string(&text.to_lowercase()).unwrap_or_else(|_| "\"\"".to_string()),
        )
    } else {
        format!(
            "(() => {{ const el = document.querySelector({selector}); if (!el) return false; \
             const r = el.getBoundingClientRect(); if (r.width <= 0 || r.height <= 0) return false; \
             el.click(); return true; }})()",
            selector = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string()),
        )
    };
    page.eval_raw(&script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Fecha diálogos/overlays abertos com a lista exata do oráculo (port de
/// `closeModals`): CSS puro em um round-trip + `:has-text` com filtro de diálogo.
///
/// Retorna a quantidade de cliques de fechamento disparados.
pub async fn close_modals(page: &dyn Page) -> Result<u32, BrowserError> {
    let (css, has_text): (Vec<&str>, Vec<&str>) = CLOSE_BUTTONS
        .iter()
        .partition(|selector| !selector.contains(":has-text("));

    let mut closed = 0_u32;
    if !css.is_empty() {
        let script = format!(
            "(() => {{ const sels = {sels}; const isVisible = (el) => {{ \
             const rect = el.getBoundingClientRect(); \
             return rect.width > 0 && rect.height > 0 && window.getComputedStyle(el).visibility !== 'hidden'; }}; \
             const inDialog = (el) => el.closest({dialog}) !== null; let closed = 0; \
             for (const sel of sels) {{ let els = []; try {{ els = Array.from(document.querySelectorAll(sel)); }} catch {{ continue; }} \
             for (const el of els) {{ if (!isVisible(el) || !inDialog(el)) continue; \
             try {{ el.click(); closed++; }} catch {{}} }} }} return closed; }})()",
            sels = serde_json::to_string(&css).unwrap_or_else(|_| "[]".to_string()),
            dialog = serde_json::to_string(DIALOG_CLOSEST).unwrap_or_else(|_| "\"\"".to_string()),
        );
        if let Ok(value) = page.eval_raw(&script).await {
            closed += u32::try_from(value.as_u64().unwrap_or(0)).unwrap_or(0);
        }
    }

    for selector in has_text {
        if let Some((base, text)) = split_has_text(selector) {
            let script = format!(
                "(() => {{ const nodes = document.querySelectorAll({base}); \
                 for (const el of nodes) {{ const r = el.getBoundingClientRect(); \
                 if (r.width <= 0 || r.height <= 0) continue; \
                 if (!(el.textContent || '').toLowerCase().includes({text})) continue; \
                 if (el.closest({dialog}) === null) continue; el.click(); return true; }} return false; }})()",
                base = serde_json::to_string(base).unwrap_or_else(|_| "\"\"".to_string()),
                text = serde_json::to_string(&text.to_lowercase())
                    .unwrap_or_else(|_| "\"\"".to_string()),
                dialog =
                    serde_json::to_string(DIALOG_CLOSEST).unwrap_or_else(|_| "\"\"".to_string()),
            );
            if page
                .eval_raw(&script)
                .await
                .ok()
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                closed += 1;
            }
        }
    }
    Ok(closed)
}

/// Gerador de trajetória do slider (easing quadrático + jitter vertical).
pub fn slider_trajectory(
    start_x: f64,
    start_y: f64,
    distance: f64,
    steps: usize,
    random: &mut dyn FnMut() -> f64,
) -> Vec<(f64, f64)> {
    let mut points = Vec::with_capacity(steps);
    for index in 1..=steps {
        #[allow(clippy::cast_precision_loss)]
        let progress = index as f64 / steps as f64;
        let ease = if progress < 0.5 {
            2.0 * progress * progress
        } else {
            -1.0 + (4.0 - 2.0 * progress) * progress
        };
        let current_x = start_x + distance * ease;
        let jitter_y = start_y + (random() * 2.0 - 1.0);
        points.push((current_x, jitter_y));
    }
    points
}

/// Handle visível do slider anti-bot.
pub const SLIDER_HANDLE_SELECTOR: &str = "#nc_1_n1z, .btn_slide, span[class*=\"btn_slide\"], #nc_1__scale_text .btn_slide, div[id*=\"nocaptcha\"] .btn_slide";
/// Trilha do slider (usada para medir a distância e confirmar a solução).
pub const SLIDER_TRACK_SELECTOR: &str = "#nc_1__scale_text, .nc_scale, div[id*=\"nocaptcha\"]";

fn random_fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos());
    f64::from(nanos) / 1_000_000_000.0
}

/// Resolve o slide captcha com movimento humanizado (port de `trySolveSlider`).
pub async fn solve_slider(page: &dyn Page) -> bool {
    let script = format!(
        "(() => {{ const handle = document.querySelector({handle}); if (!handle) return null; \
         const r = handle.getBoundingClientRect(); \
         const style = window.getComputedStyle(handle); \
         if (r.width <= 0 || r.height <= 0 || style.visibility === 'hidden') return null; \
         const track = document.querySelector({track}); \
         const trackWidth = track ? track.getBoundingClientRect().width : 320; \
         return JSON.stringify({{ x: r.left, y: r.top, w: r.width, h: r.height, trackWidth }}); }})()",
        handle =
            serde_json::to_string(SLIDER_HANDLE_SELECTOR).unwrap_or_else(|_| "\"\"".to_string()),
        track = serde_json::to_string(SLIDER_TRACK_SELECTOR).unwrap_or_else(|_| "\"\"".to_string()),
    );
    let Some(text) = page
        .eval_raw(&script)
        .await
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
    else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let x = parsed
        .get("x")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(-1.0);
    let y = parsed
        .get("y")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(-1.0);
    let width = parsed
        .get("w")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let track_width = parsed
        .get("trackWidth")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(320.0);
    if x < 0.0 || y < 0.0 {
        return false;
    }

    ali_coins_core::logging::global().info(
        "Verificação de segurança (slide captcha) detectada. Tentando deslizar...",
        &[],
    );
    let distance = if track_width > 150.0 {
        track_width - width + 10.0
    } else {
        300.0
    };
    let start_x = x + width / 2.0;
    let start_y = y + parsed
        .get("h")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
        / 2.0;

    if page.mouse_move(start_x, start_y, false).await.is_err()
        || page.mouse_down(start_x, start_y).await.is_err()
    {
        return false;
    }
    let points = slider_trajectory(start_x, start_y, distance, 25, &mut random_fraction);
    for (point_x, point_y) in points {
        let _ = page.mouse_move(point_x, point_y, true).await;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let jitter = (random_fraction() * 10.0) as u64;
        tokio::time::sleep(Duration::from_millis(10 + jitter)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = page.mouse_up(start_x + distance, start_y).await;

    // Sucesso somente com a trilha removida (mesmo alvo).
    let detached_script = format!(
        "!document.querySelector({})",
        serde_json::to_string(SLIDER_TRACK_SELECTOR).unwrap_or_else(|_| "\"\"".to_string())
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut solved = false;
    while std::time::Instant::now() < deadline {
        let detached = page
            .eval_raw(&detached_script)
            .await
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if detached {
            solved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    solved
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
                "window.getComputedStyle(el).visibility".to_string(),
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

    #[test]
    fn separa_has_text() {
        assert_eq!(
            split_has_text("button:has-text(\"Entrar\")"),
            Some(("button", "Entrar"))
        );
        assert_eq!(
            split_has_text("[class*=\"modal\"] button:has-text('Confirmar')"),
            Some(("[class*=\"modal\"] button", "Confirmar"))
        );
        assert_eq!(split_has_text("button.cosmos-btn-primary"), None);
    }

    #[test]
    fn trajetoria_do_slider_tem_easing_e_jitter() {
        let mut random = || 0.5_f64;
        let points = slider_trajectory(100.0, 200.0, 300.0, 25, &mut random);
        assert_eq!(points.len(), 25);
        // Easing termina exatamente na distância percorrida.
        assert!((points[24].0 - 400.0).abs() < 0.0001);
        // Jitter com random fixo em 0.5 não desloca o eixo Y.
        for point in &points {
            assert!((point.1 - 200.0).abs() < 0.0001);
        }
        // Progresso monotônico no eixo X (easing quadrático).
        for pair in points.windows(2) {
            assert!(pair[1].0 >= pair[0].0);
        }
    }
}
