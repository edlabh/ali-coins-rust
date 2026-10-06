//! Verificação e extração da gaveta de tarefas — port fiel de
//! `libs/tasks/verifier.js` (abertura resiliente, extração e retry).

use crate::navigation::close_modals;
use crate::tasks::{TaskItem, TasksError, parse_task_item};
use ali_coins_browser::driver::{Browser, Page};
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

        // 1) Clique via JS nos melhores candidatos (botão real primeiro).
        let mut opened = false;
        if click_open_button(page).await {
            opened = page
                .wait_for_selector(selectors::TASK_ITEM, Duration::from_secs(6))
                .await
                .is_ok()
                || drawer_is_open(page).await;
        }
        // 2) Clique real de mouse (gesto confiável, como o Playwright) com
        //    verificação de posição (`elementFromPoint`) em cada candidato.
        if !opened {
            for (x, y) in open_button_candidates(page).await {
                if page.click_at(x, y).await.is_err() {
                    continue;
                }
                if page
                    .wait_for_selector(selectors::TASK_ITEM, Duration::from_secs(4))
                    .await
                    .is_ok()
                    || drawer_is_open(page).await
                {
                    opened = true;
                    break;
                }
            }
        }
        if opened {
            return true;
        }
        let _ = page.eval_raw("window.scrollBy(0, 150); true").await;

        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    false
}

/// Candidatos de abertura ordenados (botão real primeiro, menor área depois),
/// com scroll para o centro e verificação de posição (`elementFromPoint`).
async fn open_button_candidates(page: &dyn Page) -> Vec<(f64, f64)> {
    let script = r#"(() => {
      const sels = [
        'button.aecoin-taskButton-3V41b', '[class*="taskButton"]',
        'button[class*="aecoin-signButton"]', '.aecoin-signButtonWrapper-3p3NS button',
        '[class*="signButtonWrapper"] button', 'div[class*="aecoin-signButton"]'
      ];
      const cands = [];
      for (const sel of sels) {
        for (const node of document.querySelectorAll(sel)) {
          const r = node.getBoundingClientRect();
          if (r.width <= 0 || r.height <= 0 || node.offsetParent === null) continue;
          cands.push({ node, area: r.width * r.height, isButton: node.tagName === 'BUTTON' });
        }
      }
      const needles = ['earn more coins', 'ganhe mais moedas'];
      for (const node of document.querySelectorAll('button, [role="button"], div')) {
        const text = (node.textContent || '').trim().toLowerCase();
        if (!needles.some((n) => text.includes(n))) continue;
        const r = node.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0 || node.offsetParent === null) continue;
        cands.push({ node, area: r.width * r.height, isButton: node.tagName === 'BUTTON' });
      }
      cands.sort((a, b) => (a.isButton === b.isButton ? 0 : a.isButton ? -1 : 1) || a.area - b.area);
      const out = [];
      for (const c of cands.slice(0, 8)) {
        c.node.scrollIntoView({ block: 'center' });
        const r = c.node.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) continue;
        const x = r.left + r.width / 2;
        const y = r.top + r.height / 2;
        const top = document.elementFromPoint(x, y);
        const ok = Boolean(top && (c.node === top || c.node.contains(top)));
        out.push({ x, y, ok, top: top ? String(top.className || top.tagName || '').slice(0, 60) : '' });
      }
      return out;
    })()"#;
    let Ok(value) = page.eval_raw(script).await else {
        return Vec::new();
    };
    let items = value.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for item in &items {
        let x = item.get("x").and_then(serde_json::Value::as_f64);
        let y = item.get("y").and_then(serde_json::Value::as_f64);
        let ok = item
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let top = item
            .get("top")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if let (Some(x), Some(y)) = (x, y) {
            ali_coins_core::logging::global().info(
                &format!(
                    "Candidato de abertura: ({x:.0},{y:.0}) coberto={} topo={top:?}",
                    !ok
                ),
                &[],
            );
            if ok {
                out.push((x, y));
            }
        }
    }
    out
}

/// Clique via JS nos melhores candidatos (botão real primeiro; inclui o
/// fallback semântico por texto para versões novas do site).
async fn click_open_button(page: &dyn Page) -> bool {
    let script = r#"(() => {
      const sels = [
        'button.aecoin-taskButton-3V41b', '[class*="taskButton"]',
        'button[class*="aecoin-signButton"]', '.aecoin-signButtonWrapper-3p3NS button',
        '[class*="signButtonWrapper"] button', 'div[class*="aecoin-signButton"]'
      ];
      const cands = [];
      for (const sel of sels) {
        for (const node of document.querySelectorAll(sel)) {
          const r = node.getBoundingClientRect();
          if (r.width <= 0 || r.height <= 0 || node.offsetParent === null) continue;
          cands.push({ node, area: r.width * r.height, isButton: node.tagName === 'BUTTON' });
        }
      }
      const needles = ['earn more coins', 'ganhe mais moedas'];
      for (const node of document.querySelectorAll('button, [role="button"], div')) {
        const text = (node.textContent || '').trim().toLowerCase();
        if (!needles.some((n) => text.includes(n))) continue;
        const r = node.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0 || node.offsetParent === null) continue;
        cands.push({ node, area: r.width * r.height, isButton: node.tagName === 'BUTTON' });
      }
      cands.sort((a, b) => (a.isButton === b.isButton ? 0 : a.isButton ? -1 : 1) || a.area - b.area);
      if (!cands.length) return false;
      cands[0].node.click();
      return true;
    })()"#;
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

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

/// Clique **trusted** (mouse real) no botão de ação da tarefa, com fallback JS.
///
/// Alguns handlers do site (ex.: ativar o modo "tocar 3 itens") só reagem a
/// eventos de input reais.
pub async fn click_task_button_trusted(page: &dyn Page, title: &str) -> bool {
    if let Some(index) = find_task_index_by_title(page, title).await {
        let script = format!(
            "(() => {{ const els = Array.from(document.querySelectorAll('.e2e_normal_task')); \
             const el = els[{index}]; if (!el) return null; \
             const btn = el.querySelector('.e2e_normal_task_right_btn'); if (!btn) return null; \
             if (btn.scrollIntoView) btn.scrollIntoView({{ block: 'center' }}); \
             const r = btn.getBoundingClientRect(); \
             if (r.width <= 0 || r.height <= 0) return null; \
             return JSON.stringify({{ x: r.left + r.width / 2, y: r.top + r.height / 2 }}); }})()"
        );
        if let Ok(value) = page.eval_raw(&script).await {
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
                    if x >= 0.0 && y >= 0.0 && page.click_at(x, y).await.is_ok() {
                        return true;
                    }
                }
            }
        }
    }
    click_task_button(page, title).await
}

/// Aguarda o loading interno da gaveta (`common-loading-icon`) sumir.
///
/// Após o GO de algumas tarefas a gaveta recarrega o conteúdo; fechar/tocar
/// antes do fim do loading pode impedir o "modo" da tarefa de armar.
pub async fn wait_drawer_loading(page: &dyn Page, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let script = "(() => { const el = document.querySelector('.common-loading-icon'); \
                      if (!el) return false; \
                      const r = el.getBoundingClientRect(); \
                      return r.width > 0 && r.height > 0 && el.offsetParent !== null; })()";
        let loading = page
            .eval_raw(script)
            .await
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !loading {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Conteúdo textual da gaveta (para diagnóstico).
pub async fn drawer_text(page: &dyn Page) -> String {
    let script = "(() => { const el = document.querySelector('.e2e_task'); \
                  return el ? (el.innerText || '').split(/\\s+/).join(' ').slice(0, 240) : ''; })()";
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}
/// A gaveta de tarefas está aberta (altura > 100)?
pub async fn drawer_open(page: &dyn Page) -> bool {
    let script = "(() => { const el = document.querySelector('.e2e_task'); \
                  if (!el) return false; return el.getBoundingClientRect().height > 100; })()";
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Fecha a gaveta de tarefas (X no topo direito), com verificação.
///
/// A tarefa de surpresa exige tocar nos cards **da página**; com a gaveta
/// aberta os toques caem nela. Ordem: elemento de fechar → ESC → canto do
/// painel → neutralizar o overlay (`pointer-events: none`).
pub async fn close_task_drawer(page: &dyn Page) -> bool {
    if !drawer_open(page).await {
        return true;
    }

    // 1. Elemento de fechar (busca global) e/ou canto superior direito do painel.
    let script = r#"(() => {
      const closeSelectors = ['[class*="aecoin-close" i]', '[class*="close" i]', '[aria-label*="close" i]', '[role="button"]'];
      for (const sel of closeSelectors) {
        for (const el of document.querySelectorAll(sel)) {
          const r = el.getBoundingClientRect();
          if (r.width > 0 && r.height > 0 && r.top < innerHeight * 0.75) {
            const x = r.left + r.width / 2;
            const y = r.top + r.height / 2;
            el.click();
            return JSON.stringify({ x, y });
          }
        }
      }
      const item = document.querySelector('.e2e_normal_task');
      let panel = item ? item.parentElement : null;
      while (panel && panel !== document.body) {
        const r = panel.getBoundingClientRect();
        if (r.width >= innerWidth - 8 && r.height > 200) break;
        panel = panel.parentElement;
      }
      const rect = panel && panel !== document.body ? panel.getBoundingClientRect()
        : document.querySelector('.e2e_task').getBoundingClientRect();
      const x = rect.right - 40;
      const y = rect.top + 16;
      const hit = document.elementFromPoint(x, y);
      if (hit) hit.click();
      return JSON.stringify({ x, y });
    })()"#;
    if let Ok(value) = page.eval_raw(script).await {
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
                if x >= 0.0 && y >= 0.0 && page.click_at(x, y).await.is_ok() {
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    if !drawer_open(page).await {
                        return true;
                    }
                }
            }
        }
    }

    // 2. ESC (frameworks costumam fechar bottom sheets).
    let escape = r"(() => {
      const ev = { key: 'Escape', code: 'Escape', keyCode: 27, which: 27, bubbles: true };
      document.dispatchEvent(new KeyboardEvent('keydown', ev));
      document.dispatchEvent(new KeyboardEvent('keyup', ev));
      return true;
    })()";
    let _ = page.eval_raw(escape).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    if !drawer_open(page).await {
        return true;
    }

    // 3. Neutraliza o overlay para os toques alcançarem os cards.
    let neutralize = r"(() => {
      const drawer = document.querySelector('.e2e_task');
      if (!drawer) return true;
      drawer.style.pointerEvents = 'none';
      drawer.style.display = 'none';
      return true;
    })()";
    let _ = page.eval_raw(neutralize).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    !drawer_open(page).await
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

/// Estado da página principal após `ensure_main_page`.
pub enum MainPageState {
    /// A página original continua válida.
    Keep,
    /// A página original fechou e foi recriada (mobile, com device profile).
    Recreated(Box<dyn Page>),
}

/// Garante a página principal antes de abrir a gaveta (port de `ensureMainPage`):
/// recria a página quando ela está fechada e navega para a central mobile quando
/// a URL não é a dela. Nunca falha: devolve a melhor página disponível.
pub async fn ensure_main_page(
    browser: &dyn Browser,
    page: &dyn Page,
    nav_timeout_short: Duration,
) -> MainPageState {
    match page.url().await {
        Err(_) => {
            ali_coins_core::logging::global().warn(
                "Página principal fechada ou indisponível. Recriando página principal...",
                &[],
            );
            let Ok(fresh) = browser.new_page().await else {
                return MainPageState::Keep;
            };
            let profile = ali_coins_browser::launch::pixel7_profile();
            let _ = fresh.set_device_profile(&profile).await;
            let _ = crate::navigation::goto_with_retry_timeout(
                fresh.as_ref(),
                MOBILE_COIN_URL_IMMERSIVE,
                3,
                2_000,
                nav_timeout_short,
            )
            .await;
            MainPageState::Recreated(fresh)
        }
        Ok(url) => {
            if !url.contains("coin-index/index.html") {
                let _ = crate::navigation::goto_with_retry_timeout(
                    page,
                    MOBILE_COIN_URL_IMMERSIVE,
                    3,
                    2_000,
                    nav_timeout_short,
                )
                .await;
            }
            MainPageState::Keep
        }
    }
}

/// Abre a gaveta e extrai com retentativas (port de `getDrawerTasksWithRetry`).
/// Devolve as tarefas e a página ativa (recriada quando a original fechou).
pub async fn get_drawer_tasks_with_retry(
    browser: &dyn Browser,
    page: &dyn Page,
    nav_timeout_short: Duration,
    opener_timeout: Duration,
    max_retries: u32,
) -> Result<(Vec<TaskItem>, MainPageState), TasksError> {
    let mut state = MainPageState::Keep;
    let last_attempt = max_retries + 1;
    let mut reset_done = false;
    for attempt in 1..=last_attempt {
        let current: &dyn Page = match &state {
            MainPageState::Keep => page,
            MainPageState::Recreated(recreated) => recreated.as_ref(),
        };
        if let MainPageState::Recreated(recreated) =
            ensure_main_page(browser, current, nav_timeout_short).await
        {
            state = MainPageState::Recreated(recreated);
        }
        let active: &dyn Page = match &state {
            MainPageState::Keep => page,
            MainPageState::Recreated(recreated) => recreated.as_ref(),
        };
        if !open_task_drawer(active, opener_timeout).await {
            ali_coins_core::logging::global().warn(
                &format!(
                    "Tentativa {attempt}/{last_attempt}: painel de tarefas fechado ou não detectado."
                ),
                &[],
            );
            if !reset_done && attempt < last_attempt {
                // Estado do site pode impedir a gaveta de abrir (ex.: após a
                // tarefa surpresa); recarrega a central uma vez para resetar.
                reset_done = true;
                ali_coins_core::logging::global().warn(
                    "Gaveta inacessível; recarregando a central de moedas para resetar o estado...",
                    &[],
                );
                let _ = active.eval_raw("location.reload()").await;
                tokio::time::sleep(Duration::from_millis(3000)).await;
            }
            if attempt == last_attempt {
                log_drawer_failure_diagnostics(active).await;
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
            continue;
        }
        match extract_tasks_from_drawer(active).await {
            Ok(tasks) => return Ok((tasks, state)),
            Err(error) => {
                ali_coins_core::logging::global().warn(
                    &format!(
                        "Tentativa {attempt}/{last_attempt}: falha na leitura dos elementos de tarefas: {error}"
                    ),
                    &[],
                );
                if attempt == last_attempt {
                    log_drawer_failure_diagnostics(active).await;
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
    Err(TasksError::DrawerMissing)
}

/// Diagnóstico best-effort quando a gaveta não abre: URL, título, trecho do
/// corpo e screenshot `0600` em `scratch/` (facilita explicar falhas do site).
pub(crate) async fn log_drawer_failure_diagnostics(page: &dyn Page) {
    let url = page.url().await.unwrap_or_default();
    let title = page.title().await.unwrap_or_default();
    let snippet = page
        .eval_raw(
            "(document.body && document.body.innerText ? document.body.innerText.slice(0, 300) : '')",
        )
        .await
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let probe = page
        .eval_raw(
            r#"(() => {
              const iframes = Array.from(document.querySelectorAll('iframe'))
                .map((f) => (f.src || '').slice(0, 120));
              const buttons = Array.from(document.querySelectorAll('button'))
                .slice(0, 12)
                .map((b) => ({ t: (b.textContent || '').trim().slice(0, 40), c: String(b.className || '').slice(0, 60) }));
              const taskish = Array.from(document.querySelectorAll('[class*="task"], [class*="signButton"], .e2e_task'))
                .slice(0, 8)
                .map((e) => String(e.className || '').slice(0, 60));
              return { iframes, buttons, taskish };
            })()"#,
        )
        .await
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_default();
    ali_coins_core::logging::global().error(
        &format!("Diagnóstico da gaveta indisponível: url={url} título={title:?} corpo={snippet:?} sondas={probe}"),
        &[],
    );
    if let Ok(png) = page.screenshot().await {
        let dir = std::path::PathBuf::from("scratch");
        if ali_coins_browser::diagnostics::prepare_output_dir(&dir).is_ok() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| {
                    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
                });
            let path = dir.join(format!("tasks-drawer-falha-{ts}.png"));
            if ali_coins_core::secure_fs::safe_write_file(&path, &png).is_ok() {
                ali_coins_core::logging::global().warn(
                    &format!("Screenshot do diagnóstico salvo em {}", path.display()),
                    &[],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
    use ali_coins_browser::mock::{MockDriver, MockPageSpec};

    #[tokio::test]
    async fn gaveta_sem_botao_retorna_false() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver
            .launch(&LaunchOptions::default())
            .await
            .expect("launch");
        let page = browser.new_page().await.expect("página");
        let opened = open_task_drawer(&*page, Duration::from_millis(200)).await;
        assert!(!opened, "sem botão de abertura a gaveta não abre");
    }

    #[tokio::test]
    async fn get_drawer_sem_botao_falha_apos_retries() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver
            .launch(&LaunchOptions::default())
            .await
            .expect("launch");
        let page = browser.new_page().await.expect("página");
        let result = get_drawer_tasks_with_retry(
            &*browser,
            &*page,
            Duration::from_millis(100),
            Duration::from_millis(50),
            1,
        )
        .await;
        assert!(matches!(result, Err(TasksError::DrawerMissing)));
    }

    #[tokio::test]
    async fn pagina_fechada_e_recriada_na_central_mobile() {
        let driver = MockDriver::new(vec![MockPageSpec::default(), MockPageSpec::default()]);
        let browser = driver
            .launch(&LaunchOptions::default())
            .await
            .expect("launch");
        let page = browser.new_page().await.expect("página");
        page.close().await.expect("fechar");

        match ensure_main_page(&*browser, &*page, Duration::from_secs(1)).await {
            MainPageState::Recreated(nova) => {
                assert_eq!(
                    nova.url().await.expect("url"),
                    MOBILE_COIN_URL_IMMERSIVE,
                    "página recriada deve navegar para a central mobile"
                );
            }
            MainPageState::Keep => panic!("deveria recriar a página fechada"),
        }
    }

    #[tokio::test]
    async fn pagina_viva_com_url_errada_e_navegada_sem_recriar() {
        let driver = MockDriver::new(vec![MockPageSpec {
            url: "https://example.com/outra".to_string(),
            ..MockPageSpec::default()
        }]);
        let browser = driver
            .launch(&LaunchOptions::default())
            .await
            .expect("launch");
        let page = browser.new_page().await.expect("página");

        match ensure_main_page(&*browser, &*page, Duration::from_secs(1)).await {
            MainPageState::Keep => {
                assert_eq!(
                    page.url().await.expect("url"),
                    MOBILE_COIN_URL_IMMERSIVE,
                    "página viva deve ser navegada para a central mobile"
                );
            }
            MainPageState::Recreated(_) => panic!("não deveria recriar página viva"),
        }
    }
}
