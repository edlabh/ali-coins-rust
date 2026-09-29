//! Verificação e extração da gaveta de tarefas — port fiel de
//! `libs/tasks/verifier.js` (abertura resiliente, extração e retry).

use crate::navigation::close_modals;
use crate::tasks::{TaskItem, TasksError, parse_task_item};
use ali_coins_browser::driver::Page;
use std::time::{Duration, Instant};

/// URL da central de moedas mobile (painel de tarefas).
pub const MOBILE_COIN_URL: &str = "https://m.aliexpress.com/p/coin-index/index.html";

/// Variante com imersão usada pelo oráculo no mobile.
pub const MOBILE_COIN_URL_IMMERSIVE: &str =
    "https://m.aliexpress.com/p/coin-index/index.html?_immersiveMode=true&from=pc302";

/// Seletores da gaveta (subset de `SELECTORS.tasks`).
pub mod selectors {
    /// Container da gaveta.
    pub const DRAWER_CONTAINER: &str = ".e2e_task";
    /// Item de tarefa.
    pub const TASK_ITEM: &str = ".e2e_normal_task";
    /// Título do item.
    pub const TASK_TITLE: &str = ".e2e_normal_task_content_title";
    /// Botão de ação/coleta.
    pub const TASK_BTN: &str = ".e2e_normal_task_right_btn";
    /// Botão que abre a gaveta.
    pub const OPEN_DRAWER_BTN: &str = "button.aecoin-taskButton-3V41b, [class*=\"taskButton\"], button[class*=\"aecoin-signButton\"], .aecoin-signButtonWrapper-3p3NS button, [class*=\"signButtonWrapper\"] button, div[class*=\"aecoin-signButton\"]";
    /// Cards de produto (feed de surpresas).
    pub const PRODUCT_CARD: &str = ".feeds-discount-card";
    /// Skeleton de carregamento preso.
    pub const SKELETON: &str = ".login-pending-container";
}

/// A altura do container da gaveta indica que ela está aberta.
async fn drawer_is_open(page: &dyn Page) -> bool {
    let script = r"(() => {
      const el = document.querySelector('.e2e_task');
      if (!el) return false;
      return el.getBoundingClientRect().height > 100;
    })()";
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// O botão de abrir a gaveta está presente?
async fn open_button_present(page: &dyn Page) -> bool {
    page.wait_for_selector(selectors::OPEN_DRAWER_BTN, Duration::from_millis(150))
        .await
        .is_ok()
}

/// Página presa no skeleton de carregamento?
async fn skeleton_visible(page: &dyn Page) -> bool {
    page.wait_for_selector(selectors::SKELETON, Duration::from_millis(150))
        .await
        .is_ok()
}

/// Abre a gaveta de tarefas com até 5 tentativas (port de `openTaskDrawer`).
pub async fn open_task_drawer(page: &dyn Page, timeout: Duration) -> bool {
    if drawer_is_open(page).await {
        return true;
    }

    // Auto-cura: monitora skeleton congelado (até ~9s) e recarrega uma vez.
    let mut reloaded = false;
    for wait in 0..25_u32 {
        let skeleton = skeleton_visible(page).await;
        let has_button = open_button_present(page).await;
        if has_button || (!skeleton && wait > 0) {
            break;
        }
        if wait == 17 && !reloaded {
            ali_coins_core::logging::global().info(
                "Página presa em skeleton de carregamento. Recarregando página (reload)...",
                &[],
            );
            reloaded = true;
            let _ = page.eval_raw("location.reload()").await;
        }
        tokio::time::sleep(Duration::from_millis(350)).await;
    }

    let deadline = Instant::now() + timeout;
    for attempt in 1..=5_u32 {
        let _ = close_modals(page).await;
        ali_coins_core::logging::global().info(
            &format!("Tentativa {attempt}/5 de abrir o painel \"Ganhe mais moedas\"..."),
            &[],
        );

        let mut clicked = click_open_button(page).await;
        if !clicked {
            clicked = click_open_by_text(page).await;
        }
        if clicked {
            if page
                .wait_for_selector(selectors::TASK_ITEM, Duration::from_secs(6))
                .await
                .is_ok()
            {
                return true;
            }
            if drawer_is_open(page).await {
                return true;
            }
        } else {
            let _ = page.eval_raw("window.scrollBy(0, 150); true").await;
        }

        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    false
}

/// Clique no primeiro seletor de abertura visível.
async fn click_open_button(page: &dyn Page) -> bool {
    let script = r#"(() => {
      const sels = [
        'button.aecoin-taskButton-3V41b', '[class*="taskButton"]',
        'button[class*="aecoin-signButton"]', '.aecoin-signButtonWrapper-3p3NS button',
        '[class*="signButtonWrapper"] button', 'div[class*="aecoin-signButton"]'
      ];
      for (const sel of sels) {
        const el = document.querySelector(sel);
        if (!el) continue;
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) continue;
        el.click();
        return true;
      }
      return false;
    })()"#;
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Fallback semântico: botão com texto "Ganhe mais moedas"/"Earn more coins".
async fn click_open_by_text(page: &dyn Page) -> bool {
    let script = r#"(() => {
      const needles = ['earn more coins', 'ganhe mais moedas'];
      const nodes = document.querySelectorAll('button, [role="button"], div');
      for (const el of nodes) {
        const text = (el.textContent || '').trim().toLowerCase();
        if (!needles.some((n) => text.includes(n))) continue;
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0 || el.offsetParent === null) continue;
        el.click();
        return true;
      }
      return false;
    })()"#;
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Extrai as tarefas da gaveta (mesmo script de `extractTasksFromDrawer`).
pub async fn extract_tasks_from_drawer(page: &dyn Page) -> Result<Vec<TaskItem>, TasksError> {
    let script = r"(() => {
      const els = Array.from(document.querySelectorAll('.e2e_normal_task'));
      return els.map((e, idx) => {
        const q = (sel) => e.querySelector(sel);
        const title = q('.e2e_normal_task_content_title')?.innerText?.trim() || '';
        const desc = q('.e2e_normal_task_content_secondTitle')?.innerText?.trim() || '';
        const btn = q('.e2e_normal_task_right_btn');
        const btnText = btn?.innerText?.trim() || '';
        const btnStyle = btn?.getAttribute('style') || '';
        const statusText = q('.statusText')?.innerText?.trim() || '';
        let completedRounds = null, currentRound = null, totalRounds = null;
        if (statusText) {
          const sm = statusText.match(/^([0-9]+)\/([0-9]+)$/);
          if (sm) {
            completedRounds = parseInt(sm[1], 10);
            totalRounds = parseInt(sm[2], 10);
            currentRound = Math.min(completedRounds + 1, totalRounds);
          }
        }
        const isActionable = /^(GO|IR)$/i.test(btnText);
        const isClaimable = /^(COLLECT|COLETAR|GET|RECEBER|CLAIM|RESGATAR|\+[0-9]+)/i.test(btnText);
        let isDone = false;
        if (totalRounds !== null && completedRounds !== null) {
          isDone = completedRounds >= totalRounds && !isClaimable;
        } else {
          const isDisabledStyle = btnStyle.includes('opacity: 0.5');
          const isDoneText = /^(DONE|CONCLU[IÍ]DO|COMPLETED)$/i.test(btnText);
          isDone = !isClaimable && (isDisabledStyle || isDoneText);
        }
        const groupId = q('.e2e_normal_task_right')?.getAttribute('data-groupid') || '';
        const allText = (e.innerText || '').replace(/\n+/g, ' ');
        const coinMatch = allText.match(/\+([0-9]+(?:～[0-9]+)?)/);
        const estimatedCoins = coinMatch ? ('+' + coinMatch[1] + ' moedas') : '+5 moedas';
        return { index: idx, title, desc, btnText, btnStyle, statusText,
                 completedRounds, currentRound, totalRounds, isDone, isActionable,
                 isClaimable, groupId, coins: estimatedCoins, estimatedCoins, allText };
      });
    })()";
    let value = page.eval_raw(script).await?;
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(parse_task_item).collect())
}

/// Índice do item cujo título casa exatamente (evita clicar na tarefa errada).
pub async fn find_task_index_by_title(page: &dyn Page, title: &str) -> Option<usize> {
    let script = format!(
        r"(() => {{
          const els = Array.from(document.querySelectorAll('.e2e_normal_task'));
          const idx = els.findIndex((el) => {{
            const node = el.querySelector('.e2e_normal_task_content_title');
            return Boolean(node) && node.innerText.trim() === {title};
          }});
          return idx;
        }})()",
        title = serde_json::to_string(title).unwrap_or_else(|_| "\"\"".to_string())
    );
    page.eval_raw(&script)
        .await
        .ok()
        .and_then(|value| value.as_i64())
        .and_then(|index| usize::try_from(index).ok())
}

/// Clica no botão de ação da tarefa com o título exato.
pub async fn click_task_button(page: &dyn Page, title: &str) -> bool {
    let Some(index) = find_task_index_by_title(page, title).await else {
        return false;
    };
    let script = format!(
        "(() => {{ const els = Array.from(document.querySelectorAll('.e2e_normal_task')); \
         const el = els[{index}]; if (!el) return false; const btn = el.querySelector('.e2e_normal_task_right_btn'); \
         if (!btn) return false; btn.click(); return true; }})()"
    );
    page.eval_raw(&script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Garante que a página esteja na central de moedas (port de `ensureMainPage`).
pub async fn ensure_coin_page(page: &dyn Page, nav_timeout: Duration) -> Result<(), TasksError> {
    let url = page.url().await.unwrap_or_default();
    if url.contains("coin-index/index.html") {
        return Ok(());
    }
    crate::navigation::goto_with_retry_timeout(
        page,
        MOBILE_COIN_URL_IMMERSIVE,
        3,
        2_000,
        nav_timeout,
    )
    .await?;
    Ok(())
}

/// Abre a gaveta e extrai com retentativas (port de `getDrawerTasksWithRetry`).
pub async fn get_drawer_tasks_with_retry(
    page: &dyn Page,
    nav_timeout_short: Duration,
    opener_timeout: Duration,
    max_retries: u32,
) -> Result<Vec<TaskItem>, TasksError> {
    for attempt in 1..=max_retries + 1 {
        let _ = ensure_coin_page(page, nav_timeout_short).await;
        if !open_task_drawer(page, opener_timeout).await {
            ali_coins_core::logging::global().warn(
                &format!(
                    "Tentativa {attempt}/{}: painel de tarefas fechado ou não detectado.",
                    max_retries + 1
                ),
                &[],
            );
            tokio::time::sleep(Duration::from_millis(1000)).await;
            continue;
        }
        match extract_tasks_from_drawer(page).await {
            Ok(tasks) => return Ok(tasks),
            Err(error) => {
                ali_coins_core::logging::global().warn(
                    &format!(
                        "Tentativa {attempt}/{}: falha na leitura dos elementos de tarefas: {error}",
                        max_retries + 1
                    ),
                    &[],
                );
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
    Err(TasksError::DrawerMissing)
}
