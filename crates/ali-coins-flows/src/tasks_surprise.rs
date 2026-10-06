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
pub(crate) async fn save_debug(page: &dyn Page, name: &str) {
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
    let script = "(() => { const el = document.querySelector('.e2e_task') || document.querySelector('.feeds-discount-card'); \
                  return el ? el.outerHTML.slice(0, 5000) : ''; })()";
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
/// Clica no card como o Playwright (`card.click({ delay: 50 })`):
/// move o mouse, pressiona, aguarda 50 ms e solta — somente se o centro do
/// card estiver realmente recebendo o evento (actionability/hit target).
async fn click_card(page: &dyn Page, data_id: &str) -> bool {
    let id_json = serde_json::to_string(data_id).unwrap_or_else(|_| "\"\"".to_string());
    for attempt in 1..=3_u32 {
        let probe = format!(
            "(() => {{ const el = document.getElementById({id_json}); if (!el) return null; \
             if (el.scrollIntoView) el.scrollIntoView({{ block: 'center' }}); \
             const r = el.getBoundingClientRect(); \
             if (r.width < 40 || r.height < 40) return null; \
             const x = r.left + r.width / 2, y = r.top + r.height / 2; \
             const hit = document.elementFromPoint(x, y); \
             const inside = Boolean(hit) && (hit === el || el.contains(hit)); \
             return JSON.stringify({{ x, y, inside }}); }})()"
        );
        let Some(text) = with_timeout(3000, page.eval_raw(&probe))
            .await
            .and_then(|value| value.as_str().map(str::to_string))
        else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if parsed.get("inside").and_then(serde_json::Value::as_bool) != Some(true) {
            // Outro elemento intercepta o ponto (overlay/gaveta): tenta de novo.
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        let x = parsed
            .get("x")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(-1.0);
        let y = parsed
            .get("y")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(-1.0);
        if x < 0.0 || y < 0.0 {
            continue;
        }
        // Eventos reais com teto curto: se o CDP demorar para responder por
        // causa de uma navegação disparada pelo próprio clique, o evento já foi
        // entregue — tratamos como "clique despachado" e o pós-clique recupera.
        let _ = with_timeout(5000, page.mouse_move(x, y, false)).await;
        let _ = with_timeout(5000, page.mouse_down(x, y)).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = with_timeout(5000, page.mouse_up(x, y)).await;
        ali_coins_core::logging::global().info(
            &format!(
                "Clique (mouse, delay 50ms) no card {data_id} ({x:.0},{y:.0}) [tentativa {attempt}]."
            ),
            &[],
        );
        return true;
    }

    // Fallback JS (mocks/ambientes sem input real).
    let script = format!(
        "(() => {{ const el = document.getElementById({id_json}); if (!el) return false; \
         const target = el.querySelector('.product-click') || el; target.click(); return true; }})()"
    );
    let fallback = with_timeout(4000, page.eval_raw(&script))
        .await
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if fallback {
        ali_coins_core::logging::global().warn(
            &format!("Clique JS (fallback) no card {data_id} — sem clique real disponível."),
            &[],
        );
    }
    fallback
}

/// Primeira página aberta cuja URL não estava na lista e não é a página principal.
async fn find_new_page(
    browser: Option<&dyn Browser>,
    before: &[String],
    main_url: &str,
) -> Option<Box<dyn Page>> {
    let browser = browser?;
    let pages = with_timeout(5000, browser.pages()).await?;
    for page in pages {
        let url = with_timeout(2000, page.url()).await.unwrap_or_default();
        if !url.is_empty() && url != main_url && !before.contains(&url) {
            return Some(page);
        }
    }
    None
}

/// Neutraliza overlays de tela cheia (gaveta/máscaras) que interceptam os toques.
async fn neutralize_overlays(page: &dyn Page) -> usize {
    let script = r#"(() => {
      const sels = ['.e2e_task', '[class*="aecoin-mask"]', '[class*="mask"]', '[class*="overlay"]'];
      let n = 0;
      for (const sel of sels) {
        for (const el of document.querySelectorAll(sel)) {
          const r = el.getBoundingClientRect();
          if (r.width >= innerWidth * 0.6 && r.height >= innerHeight * 0.4) {
            el.style.pointerEvents = 'none';
            el.style.display = 'none';
            n++;
          }
        }
      }
      return n;
    })()"#;
    with_timeout(3000, page.eval_raw(script))
        .await
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

/// Quantidade de requests de tracking/carregamento do card na página.
async fn tracking_count(page: &dyn Page) -> usize {
    let script = r"(() => {
      const entries = performance.getEntriesByType('resource') || [];
      return entries.filter((entry) => {
        const url = entry.name || '';
        return url.includes('/track') || url.includes('/trace') || url.includes('adclick') || url.includes('ae-');
      }).length;
    })()";
    with_timeout(3000, page.eval_raw(script))
        .await
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
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
        if url.is_empty()
            || url == protect_url
            || before.contains(&url)
            // Nunca fechar a central de moedas: pode ser a própria página
            // principal (o matching por URL não distingue abas irmãs).
            || url.contains("coin-index")
        {
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

    // Fecha a gaveta/modal residual e neutraliza máscaras/overlays de tela cheia
    // (o site mantém a máscara do modal mesmo com a gaveta fechada em alguns casos).
    let _ = crate::navigation::close_modals(page).await;
    let neutralized = neutralize_overlays(page).await;
    if neutralized > 0 {
        ali_coins_core::logging::global().info(
            &format!("Overlays neutralizados antes dos toques: {neutralized}."),
            &[],
        );
    }
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
    let mut no_progress_streak = 0_u32;

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
        let tracking_before = tracking_count(page).await;
        let clicked = click_card(page, &card.data_id).await;
        if clicked {
            touched.insert(card.signature.clone());
            clicked_count += 1;
            // Oráculo: o clique abre o item em NOVA ABA; espera o beacon curto
            // (load + 1s), fecha a aba e mantém a feed intacta.
            let main_url = current_url(page).await;
            let new_tab = find_new_page(browser, &before_pages, &main_url).await;
            if let Some(tab) = new_tab {
                tokio::time::sleep(Duration::from_millis(3000)).await;
                let _ = with_timeout(2000, tab.url()).await;
                tokio::time::sleep(Duration::from_millis(1000)).await;
                let _ = with_timeout(3000, tab.close()).await;
                last_click_stayed_on_feed = true;
            } else {
                // Sem aba nova: o clique pode ter navegado a própria página.
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let after_url = current_url(page).await;
                let after_metrics = card_metrics(page).await;
                let navigated = after_url.contains("/item/") || after_url.contains("/detail/");
                if navigated || after_metrics.is_empty() {
                    navigated_away = navigated;
                    tokio::time::sleep(Duration::from_millis(2000)).await;
                    let _ = with_timeout(15000, page.go_back()).await;
                    let after_metrics = wait_for_cards_until(page, Duration::from_secs(45)).await;
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
            }
            let protect = current_url(page).await;
            let tracking_after = tracking_count(page).await;
            ali_coins_core::logging::global().info(
                &format!(
                    "Pós-clique: url={} cards={} tracking={tracking_before}->{tracking_after}",
                    protect,
                    card_metrics(page).await.len()
                ),
                &[],
            );
            if i == 0 {
                save_debug(page, "surprise-after-tap").await;
            }
            close_new_tabs(browser, &before_pages, &protect).await;
            // Progresso é o `tracking` subir; sem progresso em toques seguidos,
            // encerra para não deixar a gaveta de tarefas inacessível.
            if tracking_after <= tracking_before {
                no_progress_streak += 1;
                if no_progress_streak >= 2 {
                    ali_coins_core::logging::global().warn(
                        "Sem progresso de tracking em toques seguidos; encerrando a tarefa surpresa para preservar a gaveta.",
                        &[],
                    );
                    break;
                }
            } else {
                no_progress_streak = 0;
            }
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
        && no_progress_streak < 2
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

    #[tokio::test]
    async fn fluxo_surpresa_para_sem_progresso() {
        use ali_coins_browser::driver::BrowserDriver as _;
        use ali_coins_browser::mock::{MockDriver, MockPageSpec};

        let cards = serde_json::json!([
            { "dataId": "card-0", "signature": "card-0" },
            { "dataId": "card-1", "signature": "card-1" }
        ]);
        let driver = MockDriver::new(vec![MockPageSpec {
            visible_selectors: vec![".feeds-discount-card".to_string()],
            eval_contains: vec![
                (
                    "document.querySelectorAll('.feeds-discount-card')".to_string(),
                    cards,
                ),
                (
                    "el.scrollIntoView".to_string(),
                    serde_json::json!("{\"x\":100,\"y\":200,\"inside\":true}"),
                ),
                (
                    "performance.getEntriesByType('resource')".to_string(),
                    serde_json::json!(5),
                ),
            ],
            ..MockPageSpec::default()
        }]);
        let browser = driver
            .launch(&ali_coins_browser::driver::LaunchOptions::default())
            .await
            .expect("launch");
        let page = browser.new_page().await.expect("page");
        let mut touched = std::collections::HashSet::new();
        let clicked = execute_surprise_items(Some(&*browser), &*page, 0, &mut touched).await;
        assert_eq!(clicked, 2, "deve parar após 2 toques sem progresso");
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
