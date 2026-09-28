//! Runner de tarefas do painel "Ganhe mais moedas" (versão conservadora).
//!
//! Extrai a gaveta via JS (1 avaliação, como o verifier), executa **claims** e
//! a tarefa de **busca** com a query fixa, e rola a página em tarefas de
//! navegação. Tarefas de surprise/minigame/avaliação **não são clicadas**
//! ainda — ficam classificadas para a paridade com o site real (D-09).
//!
//! Guards: `max_actions`, `max_attempts` por título, `skip_app_only`.

use crate::tasks::{Rounds, TaskKind, classify_task_kind, classify_task_status, priority};
use ali_coins_browser::driver::{BrowserError, Page};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use thiserror::Error;

/// URL desktop da página de moedas (painel de tarefas).
pub const DESKTOP_COIN_URL: &str =
    "https://m.aliexpress.com/p/coin-index/index.html?_immersiveMode=true&from=pc302";

/// Seletores da gaveta de tarefas.
pub mod selectors {
    /// Container da gaveta.
    pub const DRAWER_CONTAINER: &str = ".e2e_task";
    /// Item de tarefa normal.
    pub const TASK_ITEM: &str = ".e2e_normal_task";
    /// Botões que abrem a gaveta.
    pub const OPEN_DRAWER: [&str; 4] = [
        "button[class*=\"taskButton\"]",
        "[class*=\"signButtonWrapper\"] button",
        "div[class*=\"aecoin-signButton\"]",
        "button[class*=\"aecoin-signButton\"]",
    ];
}

/// Script de extração (equivalente ao `$$eval` do verifier).
pub const EXTRACT_SCRIPT: &str = r"(function () {
  const items = Array.from(document.querySelectorAll('.e2e_normal_task'));
  return items.map((el) => {
    if (!(el.offsetWidth || el.offsetHeight || el.getClientRects().length)) return null;
    const text = (selector) => {
      const node = el.querySelector(selector);
      return node ? (node.textContent || '').trim() : '';
    };
    const button = text('.e2e_normal_task_right_btn') || text('.e2e_normal_task_right') || text('.statusText');
    const match = button.match(/^([0-9]+)\s*\/\s*([0-9]+)$/);
    return {
      title: text('.e2e_normal_task_content_title'),
      description: text('.e2e_normal_task_content_secondTitle'),
      button,
      rounds: match ? { completed: Number(match[1]), total: Number(match[2]) } : null
    };
  }).filter(Boolean);
})()";

/// Erros do runner.
#[derive(Debug, Error)]
pub enum TasksError {
    /// Gaveta não abriu.
    #[error("Gaveta de tarefas não encontrada na página.")]
    DrawerMissing,
    /// Falha de browser.
    #[error("{0}")]
    Browser(String),
}

impl From<BrowserError> for TasksError {
    fn from(error: BrowserError) -> Self {
        Self::Browser(error.to_string())
    }
}

/// Tarefa extraída da gaveta.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskItem {
    /// Título.
    pub title: String,
    /// Descrição/subtítulo.
    pub description: String,
    /// Texto do botão/status.
    pub button: String,
    /// Rounds `x/y` quando presentes.
    pub rounds: Option<Rounds>,
}

/// Opções do runner.
#[derive(Debug, Clone)]
pub struct TasksOptions {
    /// Teto global de ações.
    pub max_actions: u32,
    /// Tentativas por tarefa.
    pub max_attempts: u32,
    /// Permanência de scroll nas tarefas de navegação.
    pub scroll_wait: Duration,
    /// Pular tarefas exclusivas do app.
    pub skip_app_only: bool,
    /// Query fixa da tarefa de busca.
    pub search_query: String,
    /// Timeout de navegação (`NAV_TIMEOUT`).
    pub nav_timeout: Duration,
}

impl Default for TasksOptions {
    fn default() -> Self {
        Self {
            max_actions: 25,
            max_attempts: 4,
            scroll_wait: Duration::from_secs(10),
            skip_app_only: true,
            search_query: crate::tasks::SEARCH_QUERY.to_string(),
            nav_timeout: crate::navigation::NAV_TIMEOUT,
        }
    }
}

/// Status final por tarefa.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskOutcome {
    /// Título.
    pub title: String,
    /// Status contratual (C-15).
    pub status: String,
}

/// Resultado do run.
#[derive(Debug, Clone, PartialEq)]
pub struct TasksRun {
    /// Resultados por tarefa.
    pub results: Vec<TaskOutcome>,
    /// Ações executadas.
    pub actions: u32,
}

/// Clica no primeiro botão visível cujo texto casa (fallback do `:has-text`).
async fn click_by_text(page: &dyn Page, needles: &[&str]) -> bool {
    let script = format!(
        "(() => {{ const needles = {}; const nodes = document.querySelectorAll('button, [role=\"button\"], div'); \
         for (const el of nodes) {{ const text = (el.textContent || '').trim(); \
         if (!needles.some((n) => text.toLowerCase().includes(n.toLowerCase()))) continue; \
         const r = el.getBoundingClientRect(); if (r.width <= 0 || r.height <= 0 || el.offsetParent === null) continue; \
         el.click(); return true; }} return false; }})()",
        serde_json::to_string(needles).unwrap_or_else(|_| "[]".to_string())
    );
    page.eval_raw(&script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Abre a gaveta de tarefas (espera o SPA e tenta CSS + texto).
pub async fn open_drawer(page: &dyn Page, timeout: Duration) -> Result<bool, TasksError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if page
            .wait_for_selector(selectors::DRAWER_CONTAINER, Duration::from_millis(400))
            .await
            .is_ok()
        {
            return Ok(true);
        }
        for opener in selectors::OPEN_DRAWER {
            if page
                .wait_for_selector(opener, Duration::from_millis(200))
                .await
                .is_ok()
            {
                let _ = page.click_selector(opener).await;
                if page
                    .wait_for_selector(selectors::DRAWER_CONTAINER, Duration::from_secs(3))
                    .await
                    .is_ok()
                {
                    return Ok(true);
                }
            }
        }
        if click_by_text(
            page,
            &["Ganhe mais moedas", "Earn more coins", "mais moedas"],
        )
        .await
            && page
                .wait_for_selector(selectors::DRAWER_CONTAINER, Duration::from_secs(3))
                .await
                .is_ok()
        {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let diagnostics = page
        .eval_raw(
            "(function () { \
             const classes = Array.from(document.querySelectorAll('*')) \
               .map((el) => String(el.className || '')) \
               .filter((name) => /task|coin|sign/i.test(name)); \
             const unique = Array.from(new Set(classes)).slice(0, 20); \
             const text = document.body ? document.body.innerText : ''; \
             return JSON.stringify({ \
               e2e_task: document.querySelectorAll('.e2e_task').length, \
               e2e_normal_task: document.querySelectorAll('.e2e_normal_task').length, \
               classes: unique, \
               text: text.split(/\\s+/).join(' ').slice(0, 300) \
             }); })()",
        )
        .await
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    ali_coins_core::logging::global().info(
        &format!(
            "Gaveta não encontrada. URL: {} | título: {} | diagnóstico: {diagnostics}",
            page.url().await.unwrap_or_default(),
            page.title().await.unwrap_or_default()
        ),
        &[],
    );
    Ok(false)
}

/// Extrai as tarefas da gaveta.
pub async fn extract_tasks(page: &dyn Page) -> Result<Vec<TaskItem>, TasksError> {
    let value = page.eval_raw(EXTRACT_SCRIPT).await?;
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(parse_task).collect())
}

fn parse_task(value: &Value) -> Option<TaskItem> {
    let title = value.get("title").and_then(Value::as_str)?.to_string();
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let button = value
        .get("button")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let rounds = value.get("rounds").and_then(|rounds| {
        let completed = rounds.get("completed").and_then(Value::as_u64)?;
        let total = rounds.get("total").and_then(Value::as_u64)?;
        Some(Rounds {
            completed: u32::try_from(completed).ok()?,
            total: u32::try_from(total).ok()?,
        })
    });
    Some(TaskItem {
        title,
        description,
        button,
        rounds,
    })
}

fn click_task_button(index: usize) -> String {
    format!(
        "(() => {{ const items = Array.from(document.querySelectorAll('.e2e_normal_task')); \
         const el = items[{index}]; if (!el) return false; \
         const btn = el.querySelector('.e2e_normal_task_right_btn') || el.querySelector('.e2e_normal_task_right'); \
         if (!btn) return false; btn.click(); return true; }})()"
    )
}

/// O loader (`common-loading-icon`) está visível na gaveta?
pub async fn loading_skeleton_visible(page: &dyn Page) -> bool {
    let script = r"(() => {
  const node = document.querySelector('.common-loading-icon');
  if (!node) return false;
  const rect = node.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0 && node.offsetParent !== null;
})()";
    page.eval_raw(script)
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Aguarda os itens da gaveta renderizarem (loader precisa sumir, como no verifier).
pub async fn wait_for_tasks(page: &dyn Page, timeout: Duration) -> Vec<TaskItem> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(tasks) = extract_tasks(page).await {
            if !tasks.is_empty() && !loading_skeleton_visible(page).await {
                return tasks;
            }
        }
        if std::time::Instant::now() >= deadline {
            return Vec::new();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Espera itens com reload-retry (o verifier recarrega quando o skeleton persiste).
pub async fn wait_for_tasks_with_reload(page: &dyn Page, timeout: Duration) -> Vec<TaskItem> {
    for attempt in 0..5_u32 {
        let tasks = wait_for_tasks(page, Duration::from_secs(6)).await;
        if !tasks.is_empty() {
            return tasks;
        }
        if attempt == 4 {
            break;
        }
        ali_coins_core::logging::global().info(
            &format!(
                "Skeleton persistente; recarregando a página (tentativa {}).",
                attempt + 2
            ),
            &[],
        );
        let _ =
            crate::navigation::goto_with_retry_timeout(page, DESKTOP_COIN_URL, 3, 2_000, timeout)
                .await;
        let _ = open_drawer(page, timeout.min(Duration::from_secs(10))).await;
    }
    Vec::new()
}

/// Executa as tarefas de forma conservadora (claims, busca e navegação).
pub async fn run_tasks(page: &dyn Page, options: &TasksOptions) -> Result<TasksRun, TasksError> {
    crate::navigation::goto_with_retry_timeout(
        page,
        DESKTOP_COIN_URL,
        3,
        2_000,
        options.nav_timeout,
    )
    .await?;
    if !open_drawer(page, Duration::from_secs(10)).await? {
        return Err(TasksError::DrawerMissing);
    }
    let initial = wait_for_tasks_with_reload(page, Duration::from_secs(15)).await;
    for task in &initial {
        ali_coins_core::logging::global().info(
            &format!(
                "Tarefa extraída: {:?} | botão: {:?} | rounds: {:?}",
                task.title, task.button, task.rounds
            ),
            &[],
        );
    }
    if initial.is_empty() {
        let snippet = page
            .eval_raw(
                "(() => { const el = document.querySelector('.e2e_task'); return el ? (el.innerText || '').split(/\\s+/).join(' ').slice(0, 400) : ''; })()",
            )
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();
        ali_coins_core::logging::global().info(
            &format!("Gaveta sem itens após 15s. Conteúdo: {snippet}"),
            &[],
        );
        // Guarda o HTML da gaveta em scratch/ para alinhar seletores depois.
        if let Ok(html) = page
            .eval_raw("document.querySelector('.e2e_task') ? document.querySelector('.e2e_task').outerHTML : ''")
            .await
        {
            if let Some(html) = html.as_str() {
                let scratch = std::path::Path::new("scratch");
                let _ = ali_coins_core::secure_fs::prepare_output_dir_for_dump(scratch);
                let _ = ali_coins_core::secure_fs::safe_write_file(
                    &scratch.join("tasks-drawer.html"),
                    html.as_bytes(),
                );
            }
        }
    }

    let mut attempts: HashMap<String, u32> = HashMap::new();
    let mut blocked: HashSet<String> = HashSet::new();
    let mut actions = 0_u32;
    let mut results: Vec<TaskOutcome> = Vec::new();

    loop {
        if actions >= options.max_actions {
            break;
        }
        let tasks = extract_tasks(page).await?;
        if tasks.is_empty() {
            break;
        }

        // Escolhe a próxima tarefa pendente (claimable antes de action).
        let mut candidates: Vec<(usize, &TaskItem)> = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                !blocked.contains(&task.title)
                    && !crate::tasks::is_done(&task.button, task.rounds, None)
                    && (crate::tasks::is_claimable(&task.button)
                        || crate::tasks::is_actionable(&task.button)
                        || task.button.trim().is_empty())
            })
            .collect();
        candidates.sort_by_key(|(_, task)| priority(&task.button));

        let Some((index, task)) = candidates.first().copied() else {
            break;
        };
        let title = task.title.clone();
        let count = attempts.entry(title.clone()).or_insert(0);
        *count += 1;
        if *count > options.max_attempts {
            results.push(TaskOutcome {
                title: title.clone(),
                status: crate::tasks::failure_attempts_message(options.max_attempts),
            });
            // Bloqueia o título nesta execução (evita loop sem progresso).
            blocked.insert(title);
            continue;
        }

        if crate::tasks::is_claimable(&task.button) {
            let script = click_task_button(index);
            let _ = page.eval_raw(&script).await;
            actions += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _ = crate::navigation::close_modals(page).await;
            continue;
        }

        // Card sem texto de botão: o alvo do clique é o próprio card/right.
        if task.button.trim().is_empty() {
            let script = format!(
                "(() => {{ const items = Array.from(document.querySelectorAll('.e2e_normal_task')); \
                 const el = items[{index}]; if (!el) return false; \
                 const target = el.querySelector('.e2e_normal_task_right') || el; target.click(); return true; }})()"
            );
            let _ = page.eval_raw(&script).await;
            actions += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let url = page.url().await.unwrap_or_default();
            if url.contains("coin-index") {
                let _ = crate::navigation::close_modals(page).await;
            } else {
                // Permanência com scroll na página da tarefa (navegação real).
                let wait = options.scroll_wait.max(Duration::from_secs(8));
                let deadline = std::time::Instant::now() + wait;
                while std::time::Instant::now() < deadline {
                    let _ = page.scroll_by(0, 1200).await;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                let _ = crate::navigation::goto_with_retry_timeout(
                    page,
                    DESKTOP_COIN_URL,
                    3,
                    2_000,
                    options.nav_timeout,
                )
                .await;
                let _ = open_drawer(page, Duration::from_secs(10)).await;
            }
            continue;
        }

        // Actionable: roteia pelo tipo.
        match classify_task_kind(&task.title, &task.description, &task.button) {
            TaskKind::Search => {
                let slug = options.search_query.replace(' ', "-");
                let url = format!("https://www.aliexpress.com/w/wholesale-{slug}.html");
                let _ = crate::navigation::goto_with_retry_timeout(
                    page,
                    &url,
                    3,
                    2_000,
                    options.nav_timeout,
                )
                .await;
                actions += 1;
                let _ = crate::navigation::goto_with_retry_timeout(
                    page,
                    DESKTOP_COIN_URL,
                    3,
                    2_000,
                    options.nav_timeout,
                )
                .await;
                let _ = open_drawer(page, Duration::from_secs(10)).await;
            }
            TaskKind::Navigation => {
                page.scroll_by(0, 1200).await?;
                tokio::time::sleep(options.scroll_wait).await;
                actions += 1;
            }
            other => {
                // Sem clique: classifica para o relatório (paridade pendente).
                let status = classify_task_status(
                    &task.button,
                    task.rounds,
                    None,
                    &task.title,
                    &task.description,
                    options.skip_app_only,
                );
                results.push(TaskOutcome {
                    title: title.clone(),
                    status,
                });
                blocked.insert(title);
                let _ = other;
            }
        }
    }

    // Classificação final de todas as tarefas vistas.
    if let Ok(tasks) = extract_tasks(page).await {
        for task in tasks {
            if results.iter().any(|outcome| outcome.title == task.title) {
                continue;
            }
            let status = classify_task_status(
                &task.button,
                task.rounds,
                None,
                &task.title,
                &task.description,
                options.skip_app_only,
            );
            results.push(TaskOutcome {
                title: task.title,
                status,
            });
        }
    }

    Ok(TasksRun { results, actions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
    use ali_coins_browser::mock::{MockAction, MockDriver, MockPageSpec};
    use serde_json::json;

    fn auth_state() -> Value {
        json!({ "cookies": [{ "name": "xman_us_t", "value": "abc" }], "origins": [] })
    }

    fn task_json(title: &str, button: &str, rounds: Option<(u32, u32)>) -> Value {
        json!({
            "title": title,
            "description": "",
            "button": button,
            "rounds": rounds.map(|(c, t)| json!({ "completed": c, "total": t }))
        })
    }

    #[tokio::test]
    async fn extrai_e_clica_claimable() {
        let tasks = json!([
            task_json("Tarefa A", "COLLECT", None),
            task_json("Tarefa B", "GO", None)
        ]);
        let driver = MockDriver::new(vec![MockPageSpec {
            visible_selectors: vec![".e2e_task".to_string()],
            storage_state: Some(auth_state()),
            eval_contains: vec![(".e2e_normal_task".to_string(), tasks)],
            ..MockPageSpec::default()
        }]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        let options = TasksOptions {
            max_actions: 3,
            max_attempts: 2,
            scroll_wait: Duration::from_millis(1),
            ..TasksOptions::default()
        };
        let run = run_tasks(&*page, &options).await.expect("runner");
        assert!(run.actions >= 1);
        assert!(driver.actions().iter().any(
            |action| matches!(action, MockAction::Eval(script) if script.contains("btn.click()"))
        ));
    }

    #[tokio::test]
    async fn gaveta_ausente_vira_erro() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        let error = run_tasks(&*page, &TasksOptions::default())
            .await
            .expect_err("sem gaveta");
        assert!(matches!(error, TasksError::DrawerMissing));
    }
}
