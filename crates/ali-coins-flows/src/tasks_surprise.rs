//! Fluxo de "itens surpresa" (tocar em 3 produtos do feed da página de moedas).
//!
//! Port de `libs/tasks/surprise.js` adaptado à evidência real do site: os
//! cards são `<div class="feeds-discount-card">` com um overlay
//! `.product-click`; o feed é virtualizado ("inscene-*"), então só cards com
//! retângulo válido são considerados. Cada operação tem teto de tempo
//! (`withTimeout` do oráculo) para o CDP pendurado não estourar a tarefa.
#![allow(clippy::implicit_hasher)]

use ali_coins_browser::driver::{Browser, Page};
use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;

/// Seletor dos cards de produto no feed.
const PRODUCT_CARD: &str = ".feeds-discount-card";

/// Parâmetros voláteis ignorados na comparação de URLs de feed.
const VOLATILE_PARAMS: [&str; 8] = [
    "_immersivemode",
    "_hideprogress",
    "aecmd",
    "_target",
    "aeia-pg-hist",
    "spm",
    "from",
    "timestamp",
];

/// Executa um future com teto de tempo (equivalente ao `withTimeout`).
async fn with_timeout<F, T, E>(ms: u64, future: F) -> Option<T>
where
    F: Future<Output = Result<T, E>>,
{
    tokio::time::timeout(Duration::from_millis(ms), future)
        .await
        .ok()?
        .ok()
}

/// Diagnóstico opt-in (`ALI_COINS_SURPRISE_DEBUG=1`): screenshot + card HTML.
async fn save_debug(page: &dyn Page, name: &str) {
    if std::env::var("ALI_COINS_SURPRISE_DEBUG").ok().as_deref() != Some("1") {
        return;
    }
    let scratch = std::path::Path::new("scratch");
    let _ = ali_coins_core::secure_fs::prepare_output_dir_for_dump(scratch);
    if let Some(bytes) = with_timeout(8000, page.screenshot()).await {
        let _ = ali_coins_core::secure_fs::safe_write_file(
            &scratch.join(format!("{name}.png")),
            &bytes,
        );
    }
    let script = "(() => { const el = document.querySelector('.feeds-discount-card'); \
                  return el ? el.outerHTML.slice(0, 3000) : ''; })()";
    if let Some(html) = with_timeout(3000, page.eval_raw(script)).await {
        if let Some(html) = html.as_str() {
            let _ = ali_coins_core::secure_fs::safe_write_file(
                &scratch.join(format!("{name}.html")),
                html.as_bytes(),
            );
        }
    }
}

/// Normaliza a URL removendo parâmetros voláteis (port de `normalizeFeedUrl`).
#[must_use]
pub fn normalize_feed_url(raw: &str) -> String {
    let without_hash = raw.split('#').next().unwrap_or("");
    let Some((base, query)) = without_hash.split_once('?') else {
        return without_hash.to_string();
    };
    let mut params: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| {
            if pair.is_empty() {
                return None;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let lowered = key.to_ascii_lowercase();
            if VOLATILE_PARAMS.contains(&lowered.as_str())
                || lowered == "ts"
                || lowered == "adpost"
                || lowered.starts_with("spm")
            {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect();
    params.sort();
    if params.is_empty() {
        return base.to_string();
    }
    let joined = params
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{joined}")
}

/// URL e DOM correspondem ao feed de surpresas (port de `isFeedUrl`).
#[must_use]
pub fn is_feed_url(url: &str, feed_url: &str, card_count: Option<usize>) -> bool {
    if url.is_empty() || feed_url.is_empty() {
        return false;
    }
    if card_count == Some(0) {
        return false;
    }
    normalize_feed_url(url) == normalize_feed_url(feed_url)
}

/// Card visível do feed (retângulo válido).
#[derive(Debug, Clone)]
struct CardMetric {
    data_id: String,
    signature: String,
}

/// Cards com retângulo válido (feed é virtualizado; ignora `inscene-outside`).
async fn card_metrics(page: &dyn Page) -> Vec<CardMetric> {
    let script = format!(
        r"(() => {{
          const els = Array.from(document.querySelectorAll('{PRODUCT_CARD}'));
          const out = [];
          for (const el of els) {{
            const r = el.getBoundingClientRect();
            if (r.width < 40 || r.height < 40) continue;
            const link = el.querySelector('a');
            const href = link ? (link.getAttribute('href') || link.href || '') : '';
            const text = (el.innerText || '').trim().replace(/\s+/g, ' ').slice(0, 80);
            const imgEl = el.querySelector('img');
            const img = imgEl ? (imgEl.getAttribute('src') || '') : '';
            const dataId = el.id || el.getAttribute('data-item-id') || el.getAttribute('data-product-id') || el.getAttribute('data-id') || '';
            out.push({{ dataId, signature: dataId || href || text || img }});
          }}
          return out;
        }})()"
    );
    with_timeout(5000, page.eval_raw(&script))
        .await
        .and_then(|value| value.as_array().cloned())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| {
                    let data_id = value.get("dataId")?.as_str()?.to_string();
                    let signature = value
                        .get("signature")
                        .and_then(|item| item.as_str())
                        .unwrap_or(&data_id)
                        .to_string();
                    if data_id.is_empty() {
                        return None;
                    }
                    Some(CardMetric { data_id, signature })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Clica no card (scroll até o centro + clique **real** no overlay `.product-click`).
async fn click_card(page: &dyn Page, data_id: &str) -> bool {
    let id_json = serde_json::to_string(data_id).unwrap_or_else(|_| "\"\"".to_string());
    let scroll_script = format!(
        "(() => {{ const el = document.getElementById({id_json}); if (!el) return false; \
         if (el.scrollIntoView) el.scrollIntoView({{ block: 'center' }}); return true; }})()"
    );
    let _ = with_timeout(3000, page.eval_raw(&scroll_script)).await;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let rect_script = format!(
        "(() => {{ const el = document.getElementById({id_json}); if (!el) return null; \
         const target = el.querySelector('.product-click') || el; \
         const r = target.getBoundingClientRect(); \
         return JSON.stringify({{ x: r.left + r.width / 2, y: r.top + r.height / 2 }}); }})()"
    );
    if let Some(value) = with_timeout(3000, page.eval_raw(&rect_script)).await {
        if let Some(text) = value.as_str() {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
                let x = parsed
                    .get("x")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(-1.0);
                let y = parsed
                    .get("y")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(-1.0);
                match tokio::time::timeout(Duration::from_millis(6000), page.tap_at(x, y)).await {
                    Ok(Ok(())) => {
                        ali_coins_core::logging::global().info(
                            &format!("Toque (touch) no card {data_id} ({x:.0},{y:.0})."),
                            &[],
                        );
                        return true;
                    }
                    Ok(Err(error)) => {
                        ali_coins_core::logging::global()
                            .warn(&format!("Touch no card {data_id} falhou: {error}"), &[]);
                    }
                    Err(_) => {
                        ali_coins_core::logging::global().warn(
                            &format!("Touch no card {data_id} excedeu o tempo limite."),
                            &[],
                        );
                    }
                }
                if x >= 0.0 && y >= 0.0 && with_timeout(6000, page.click_at(x, y)).await.is_some() {
                    ali_coins_core::logging::global().info(
                        &format!("Toque real (mouse) no card {data_id} ({x:.0},{y:.0})."),
                        &[],
                    );
                    return true;
                }
            }
        }
    }

    let script = format!(
        "(() => {{ const el = document.getElementById({id_json}); if (!el) return false; \
         const target = el.querySelector('.product-click') || el; target.click(); return true; }})()"
    );
    let clicked = with_timeout(4000, page.eval_raw(&script))
        .await
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if clicked {
        ali_coins_core::logging::global().warn(
            &format!("Toque JS (fallback) no card {data_id} — sem clique real disponível."),
            &[],
        );
    }
    clicked
}

/// URL atual da página (com teto de tempo).
async fn current_url(page: &dyn Page) -> String {
    with_timeout(3000, page.url()).await.unwrap_or_default()
}

/// URLs das páginas abertas (para detectar novas abas).
async fn page_urls(browser: Option<&dyn Browser>) -> Vec<String> {
    let Some(browser) = browser else {
        return Vec::new();
    };
    let Some(pages) = with_timeout(5000, browser.pages()).await else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for page in pages {
        if let Some(url) = with_timeout(2000, page.url()).await {
            urls.push(url);
        }
    }
    urls
}

/// Fecha abas novas criadas após o clique (best-effort, com tetos curtos).
///
/// `protect_url` é a URL atual da página principal no momento do fechamento —
/// ela nunca é fechada, mesmo que tenha navegado para fora do feed.
async fn close_new_tabs(browser: Option<&dyn Browser>, before: &[String], protect_url: &str) {
    let Some(browser) = browser else {
        return;
    };
    let Some(pages) = with_timeout(5000, browser.pages()).await else {
        return;
    };
    for page in pages {
        let Some(url) = with_timeout(2000, page.url()).await else {
            continue;
        };
        if url.is_empty() || url == protect_url || before.contains(&url) {
            continue;
        }
        let _ = with_timeout(3000, page.close()).await;
    }
}

/// Espera o feed renderizar cards (com scroll de lazy-load).
async fn wait_for_cards(page: &dyn Page) -> Vec<CardMetric> {
    let mut metrics = card_metrics(page).await;
    if metrics.is_empty() {
        for _ in 0..4 {
            let _ = with_timeout(3000, page.eval_raw("window.scrollBy(0, 800); true")).await;
            tokio::time::sleep(Duration::from_millis(800)).await;
            metrics = card_metrics(page).await;
            if !metrics.is_empty() {
                break;
            }
        }
    }
    metrics
}

/// Polla até os cards voltarem (sem forçar scroll), com teto de tempo.
async fn wait_for_cards_until(page: &dyn Page, timeout: Duration) -> Vec<CardMetric> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let metrics = card_metrics(page).await;
        if !metrics.is_empty() {
            return metrics;
        }
        if std::time::Instant::now() >= deadline {
            return Vec::new();
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

/// Toca em 3 produtos do feed (port de `executeSurpriseItems`).
pub async fn execute_surprise_items(
    browser: Option<&dyn Browser>,
    page: &dyn Page,
    start_index: usize,
    touched: &mut HashSet<String>,
) -> u32 {
    let mut feed_url = current_url(page).await;
    ali_coins_core::logging::global().info(
        &format!(
            "Executando tarefa: tocar em 3 itens (a partir do card #{})...",
            start_index + 1
        ),
        &[],
    );
    let _ = with_timeout(
        12000,
        page.wait_for_selector(PRODUCT_CARD, Duration::from_secs(12)),
    )
    .await;
    let stabilized = current_url(page).await;
    if !stabilized.is_empty() {
        feed_url = stabilized;
    }

    // Fecha a gaveta/modal residual para os cards ficarem realmente clicáveis.
    let _ = crate::navigation::close_modals(page).await;
    let metrics = wait_for_cards(page).await;
    ali_coins_core::logging::global().info(
        &format!(
            "Feed de surpresas: url={} cards={} (feed={feed_url})",
            current_url(page).await,
            metrics.len()
        ),
        &[],
    );
    save_debug(page, "surprise-feed").await;

    let mut clicked_count = 0_u32;
    let mut navigated_away = false;
    let mut last_click_stayed_on_feed = false;

    for i in 0..3_usize {
        let metrics = wait_for_cards(page).await;
        if metrics.is_empty() {
            ali_coins_core::logging::global().warn(
                &format!(
                    "Nenhum card de produto visível no DOM para o clique {}/3.",
                    i + 1
                ),
                &[],
            );
            break;
        }
        let selected = metrics
            .iter()
            .find(|card| !touched.contains(&card.signature))
            .cloned();
        let Some(card) = selected else {
            ali_coins_core::logging::global().warn(
                &format!(
                    "Todos os {} cards visíveis já foram tocados nesta rodada. Encerrando toques para evitar repetição de itens.",
                    metrics.len()
                ),
                &[],
            );
            break;
        };

        ali_coins_core::logging::global().info(
            &format!("Tocando item {}/3 (card {})...", i + 1, card.data_id),
            &[],
        );
        tokio::time::sleep(Duration::from_millis(200)).await;

        let before_pages = page_urls(browser).await;
        let clicked = click_card(page, &card.data_id).await;
        if clicked {
            touched.insert(card.signature.clone());
            clicked_count += 1;
            // Dá tempo do handler da página contabilizar o toque.
            tokio::time::sleep(Duration::from_millis(2000)).await;
            let after_url = current_url(page).await;
            let mut after_metrics = card_metrics(page).await;
            let navigated = after_url.contains("/item/") || after_url.contains("/detail/");
            if navigated || after_metrics.is_empty() {
                // Clique abriu o detalhe (mesma aba) ou a página está navegando:
                // espera o beacon disparar e volta para o feed.
                navigated_away = navigated;
                tokio::time::sleep(Duration::from_millis(2000)).await;
                let _ = with_timeout(15000, page.go_back()).await;
                after_metrics = wait_for_cards_until(page, Duration::from_secs(45)).await;
                if after_metrics.is_empty() && !feed_url.is_empty() {
                    let _ = with_timeout(
                        40000,
                        page.goto(
                            &feed_url,
                            &ali_coins_browser::driver::NavOptions {
                                timeout: Some(Duration::from_secs(35)),
                                wait_until: Some("domcontentloaded".to_string()),
                            },
                        ),
                    )
                    .await;
                    let _ = wait_for_cards_until(page, Duration::from_secs(30)).await;
                }
                last_click_stayed_on_feed = false;
            } else {
                last_click_stayed_on_feed = true;
            }
            let protect = current_url(page).await;
            if i == 0 {
                save_debug(page, "surprise-after-tap").await;
            }
            close_new_tabs(browser, &before_pages, &protect).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    tokio::time::sleep(Duration::from_millis(600)).await;

    // Fallback de detalhe (SURPRISE_DETAIL_FALLBACK, padrão ligado).
    let detail_enabled = std::env::var("SURPRISE_DETAIL_FALLBACK").map_or(true, |value| {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    });
    let final_url = current_url(page).await;
    let final_is_detail = final_url.contains("/item/") || final_url.contains("/detail/");
    let can_fallback = browser.is_some()
        && !feed_url.is_empty()
        && is_feed_url(&final_url, &feed_url, None)
        && !final_is_detail;
    if detail_enabled
        && can_fallback
        && clicked_count > 0
        && !navigated_away
        && last_click_stayed_on_feed
    {
        try_open_first_product_detail(browser, page, touched).await;
    }

    clicked_count
}

/// Abre o primeiro card não tocado em detalhe e retorna à feed (best-effort).
async fn try_open_first_product_detail(
    browser: Option<&dyn Browser>,
    page: &dyn Page,
    exclude: &HashSet<String>,
) -> bool {
    let metrics = card_metrics(page).await;
    let Some(card) = metrics
        .iter()
        .find(|card| !exclude.contains(&card.signature))
        .cloned()
    else {
        return false;
    };

    let before_pages = page_urls(browser).await;
    if !click_card(page, &card.data_id).await {
        return false;
    }
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after_url = current_url(page).await;
    if after_url.contains("/item/") || after_url.contains("/detail/") {
        let _ = with_timeout(10000, page.go_back()).await;
        return true;
    }
    let protect = current_url(page).await;
    close_new_tabs(browser, &before_pages, &protect).await;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_url_do_feed() {
        assert_eq!(
            normalize_feed_url(
                "https://m.aliexpress.com/p/coin-index/feed.html?spm=a2g0o.x&_immersiveMode=true&from=pc302&b=2&a=1"
            ),
            "https://m.aliexpress.com/p/coin-index/feed.html?a=1&b=2"
        );
    }

    #[test]
    fn compara_feed_por_url_normalizada() {
        let feed = "https://m.aliexpress.com/p/coin-index/feed.html?_immersiveMode=true&spm=abc";
        assert!(is_feed_url(feed, feed, Some(5)));
        assert!(!is_feed_url(feed, feed, Some(0)));
        assert!(!is_feed_url(
            feed,
            "https://m.aliexpress.com/item/1.html",
            Some(5)
        ));
    }
}
