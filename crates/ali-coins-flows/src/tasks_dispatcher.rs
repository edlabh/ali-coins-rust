//! Despacho e execução das ações de tarefa — port de
//! `libs/tasks/dispatcher.js` + `search.js` + `prizeland.js` + `waitWithScroll`.
#![allow(clippy::implicit_hasher)]

use crate::tasks::TaskItem;
use crate::tasks_surprise::execute_surprise_items;
use ali_coins_browser::driver::{Browser, Page};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Opções do despacho (subconjunto de config usado pelo oráculo).
#[derive(Debug, Clone)]
pub struct DispatchOptions {
    /// `SCROLL_WAIT_SECONDS` (permanência de scroll).
    pub scroll_wait_seconds: u64,
    /// `TASK_SCROLL_MAX_MS` (teto de permanência).
    pub task_scroll_max_ms: Duration,
    /// Query da tarefa de busca.
    pub search_query: String,
}

impl Default for DispatchOptions {
    fn default() -> Self {
        Self {
            scroll_wait_seconds: 15,
            task_scroll_max_ms: Duration::from_secs(30),
            search_query: crate::tasks::SEARCH_QUERY.to_string(),
        }
    }
}

/// Resultado da ação.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchOutcome {
    /// Tarefa exclusiva do app/interação especial.
    pub is_special_or_app_only: bool,
}

/// Executa a ação da tarefa (port de `executeTaskAction`).
pub async fn execute_task_action(
    browser: Option<&dyn Browser>,
    page: &dyn Page,
    task: &TaskItem,
    attempt: u32,
    options: &DispatchOptions,
    touched_cards: &mut HashSet<String>,
) -> DispatchOutcome {
    let title = task.title.to_lowercase();
    let desc = task.desc.to_lowercase();
    let text = format!("{title} {desc}");

    // 1. Produtos surpresa ("toque em 3 itens").
    if text.contains("surprise")
        || text.contains("surpresa")
        || text.contains("tap 3")
        || text.contains("toque em 3")
    {
        let round_offset = task
            .completed_rounds
            .filter(|value| *value > 0)
            .map_or(0, |value| usize::try_from(value).unwrap_or(0) * 3);
        let attempt_offset = if attempt > 1 {
            usize::try_from(attempt - 1).unwrap_or(0) * 3
        } else {
            0
        };
        let start_index = round_offset + attempt_offset;
        let _ = execute_surprise_items(browser, page, start_index, touched_cards).await;
        return DispatchOutcome::default();
    }

    let mut scroll_seconds = options.scroll_wait_seconds;
    if text.contains("desconto")
        || text.contains("discount")
        || text.contains("superdeal")
        || text.contains("super deal")
        || text.contains("15s")
        || text.contains("15 s")
    {
        scroll_seconds = scroll_seconds.max(16);
    }

    // 2. Busca por palavra-chave.
    if text.contains("search")
        || text.contains("pesquisa")
        || text.contains("buscar")
        || text.contains("keywords")
        || text.contains("palavra")
    {
        execute_search_task(page, &options.search_query).await;
        wait_with_scroll(page, scroll_seconds, options.task_scroll_max_ms, true).await;
        return DispatchOutcome::default();
    }

    // 3. Fazenda Mágica / Prize Land (regar).
    if text.contains("prize land")
        || text.contains("water")
        || text.contains("regar")
        || text.contains("0.1")
    {
        click_water_button(page).await;
        return DispatchOutcome {
            is_special_or_app_only: true,
        };
    }

    // 4. Minigames/quizzes exclusivos do app.
    if text.contains("merge boss")
        || text.contains("game")
        || text.contains("jogo")
        || text.contains("quiz")
    {
        return DispatchOutcome {
            is_special_or_app_only: true,
        };
    }

    // 5. Avaliações de pedidos entregues.
    if title.contains("review")
        || title.contains("avalia")
        || desc.contains("review")
        || desc.contains("avalia")
    {
        ali_coins_core::logging::global().info(
            &format!(
                "Tarefa \"{}\" requer pedido entregue para avaliação. Marcando como especial.",
                task.title
            ),
            &[],
        );
        return DispatchOutcome {
            is_special_or_app_only: true,
        };
    }

    // 6. Tarefas normais de navegação e scroll.
    ali_coins_core::logging::global().info(
        &format!(
            "Executando navegação com scroll ({scroll_seconds}s): \"{}\"...",
            task.title
        ),
        &[],
    );
    wait_with_scroll(page, scroll_seconds, options.task_scroll_max_ms, true).await;
    DispatchOutcome::default()
}

/// Aguarda `domcontentloaded` com teto (equivalente ao waitForLoadState curto).
pub async fn wait_dom_content_loaded(page: &dyn Page, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let ready = page
            .eval_raw("document.readyState")
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "complete".to_string());
        if ready == "interactive" || ready == "complete" {
            return;
        }
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Busca por produto (port de `executeSearchTask`).
pub async fn execute_search_task(page: &dyn Page, query: &str) {
    ali_coins_core::logging::global().info(
        &format!("Executando busca por produto: \"{query}\"..."),
        &[],
    );
    let query_json = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string());
    let script = format!(
        r#"(() => {{
          const q = {query_json};
          const sels = ['input[type="search"]', 'input[name*="SearchText" i]',
            'input[placeholder*="search" i]', 'input[aria-label*="search" i]', 'input[name*="search" i]'];
          let el = null;
          for (const sel of sels) {{ const found = document.querySelector(sel); if (found) {{ el = found; break; }} }}
          if (!el) {{
            const inputs = Array.from(document.querySelectorAll('input'));
            for (const cand of inputs) {{
              const r = cand.getBoundingClientRect();
              if (r.width > 0 && r.height > 0 && cand.offsetParent !== null) {{ el = cand; break; }}
            }}
          }}
          if (!el) return false;
          const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
          setter.call(el, q);
          el.dispatchEvent(new Event('input', {{ bubbles: true }}));
          el.dispatchEvent(new Event('change', {{ bubbles: true }}));
          for (const type of ['keydown', 'keypress', 'keyup']) {{
            el.dispatchEvent(new KeyboardEvent(type, {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }}));
          }}
          return true;
        }})()"#
    );
    let found = page
        .eval_raw(&script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !found {
        ali_coins_core::logging::global().warn(
            "Campo de busca não localizado na página; tarefa de busca não iniciada.",
            &[],
        );
    }
}

/// Clique no botão de regar (Prize Land).
pub async fn click_water_button(page: &dyn Page) -> bool {
    ali_coins_core::logging::global().info(
        "Verificando botão de água da Fazenda Mágica / Prize Land...",
        &[],
    );
    let script = r#"(() => {
      const sels = ['.Footer--waterCollectedButtonBg--2jKL1c5', '[class*="waterCollected"]'];
      let el = null;
      for (const sel of sels) { const found = document.querySelector(sel); if (found) { el = found; break; } }
      if (!el) {
        el = Array.from(document.querySelectorAll('button')).find((b) => /regar|water/i.test(b.innerText || '')) || null;
      }
      if (!el) return false;
      el.click();
      return true;
    })()"#;
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Permanência com scroll gradual (port de `waitWithScroll`).
///
/// `early_exit_on_no_progress`: encerra em 10s sem requests de tracking
/// (detectados via Resource Timing), como o listener de response do oráculo.
pub async fn wait_with_scroll(
    page: &dyn Page,
    max_seconds: u64,
    task_scroll_max_ms: Duration,
    early_exit_on_no_progress: bool,
) {
    let requested = Duration::from_secs(max_seconds);
    let max_ms = requested.min(task_scroll_max_ms);
    let start = Instant::now();
    let no_progress_timeout = Duration::from_secs(10);
    let mut tracking_detected = false;

    while start.elapsed() < max_ms {
        let _ = page.eval_raw("window.scrollBy(0, 300); true").await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if !tracking_detected {
            tracking_detected = detect_tracking(page).await;
        }
        let elapsed = start.elapsed();
        if early_exit_on_no_progress && !tracking_detected && elapsed >= no_progress_timeout {
            break;
        }
    }
}

/// Requests de tracking (`/track`, `/trace`, `adclick`, `ae-`) na página?
async fn detect_tracking(page: &dyn Page) -> bool {
    let script = r"(() => {
      const entries = performance.getEntriesByType('resource') || [];
      return entries.some((entry) => {
        const url = entry.name || '';
        return url.includes('/track') || url.includes('/trace') || url.includes('adclick') || url.includes('ae-');
      });
    })()";
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
    use ali_coins_browser::mock::{MockAction, MockDriver, MockPageSpec};

    #[tokio::test]
    async fn busca_preenche_input_visivel() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        execute_search_task(&*page, "fone bluetooth").await;
        assert!(driver.actions().iter().any(|action| matches!(
            action,
            MockAction::Eval(script) if script.contains("HTMLInputElement")
        )));
    }

    #[tokio::test]
    async fn scroll_normal_respeita_o_teto() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        let started = Instant::now();
        // Teto curto: 400ms (o loop de 1.5s pode passar um pouco, mas não os 16s).
        wait_with_scroll(&*page, 16, Duration::from_millis(400), true).await;
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
