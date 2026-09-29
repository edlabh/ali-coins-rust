//! Runner de tarefas do painel "Ganhe mais moedas" — port fiel do laço de
//! `do_tasks.js` + `libs/tasks/{state,verifier,dispatcher}.js`.
//!
//! Regras preservadas: extração com retry, prioridade claimable > ação,
//! tentativas por tarefa e por rodada, desistências (`failedTasks`), pausa
//! opcional, teto global de ações, segunda passada opt-in e relatório final
//! com os mesmos status contratuais (C-15).

use crate::tasks::{
    APP_ONLY_DISABLED_STATUS, TaskItem, TasksError, find_next_pending_task, get_round_key,
    is_interactive_or_app_only, mark_special_or_app_only, record_round_attempt,
    record_task_attempt, reset_task_attempt, select_reopenable_tasks,
};
use crate::tasks_dispatcher::{DispatchOptions, execute_task_action, wait_dom_content_loaded};
use crate::tasks_verifier::{
    MOBILE_COIN_URL_IMMERSIVE, click_task_button, ensure_coin_page, get_drawer_tasks_with_retry,
    open_task_drawer,
};
use ali_coins_browser::driver::{Browser, Page};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// URL da central de moedas mobile.
pub const DESKTOP_COIN_URL: &str = MOBILE_COIN_URL_IMMERSIVE;

/// Opções do runner (mapeadas de `Config` na CLI).
#[derive(Debug, Clone)]
pub struct TasksOptions {
    /// Teto global de ações (`TASK_MAX_ACTIONS`).
    pub max_actions: u32,
    /// Tentativas por tarefa (`TASK_MAX_ATTEMPTS`).
    pub max_attempts: u32,
    /// Tentativas por rodada (`TASK_ROUND_MAX_ATTEMPTS`, padrão 3).
    pub max_round_attempts: u32,
    /// Orçamento por tarefa (`TASK_MAX_DURATION_MS`).
    pub max_duration: Duration,
    /// `SCROLL_WAIT_SECONDS`.
    pub scroll_wait_seconds: u64,
    /// `TASK_SCROLL_MAX_MS`.
    pub task_scroll_max_ms: Duration,
    /// `SKIP_APP_ONLY_TASKS`.
    pub skip_app_only: bool,
    /// Query fixa da busca.
    pub search_query: String,
    /// `NAV_TIMEOUT`.
    pub nav_timeout: Duration,
    /// `NAV_TIMEOUT_SHORT`.
    pub nav_timeout_short: Duration,
    /// Segunda passada (`TASK_RETRY_UNFINISHED`).
    pub retry_unfinished: bool,
    /// Passadas extras (`TASK_RETRY_PASSES`).
    pub retry_passes: u32,
    /// Espera entre passadas (`TASK_RETRY_DELAY_MS`).
    pub retry_delay: Duration,
    /// Pausa mínima antes da tarefa (`TASK_PAUSE_MIN_MS`).
    pub pause_min: Duration,
    /// Pausa máxima antes da tarefa (`TASK_PAUSE_MAX_MS`).
    pub pause_max: Duration,
    /// `ALLOW_MEDIA` (ao recriar a página principal).
    pub allow_media: bool,
    /// Teto de tempo para abrir a gaveta (5 tentativas do oráculo).
    pub open_drawer_timeout: Duration,
}

impl Default for TasksOptions {
    fn default() -> Self {
        Self {
            max_actions: 25,
            max_attempts: 4,
            max_round_attempts: 3,
            max_duration: Duration::from_secs(180),
            scroll_wait_seconds: 15,
            task_scroll_max_ms: Duration::from_secs(30),
            skip_app_only: true,
            search_query: crate::tasks::SEARCH_QUERY.to_string(),
            nav_timeout: Duration::from_secs(20),
            nav_timeout_short: Duration::from_secs(8),
            retry_unfinished: false,
            retry_passes: 1,
            retry_delay: Duration::from_secs(5),
            pause_min: Duration::ZERO,
            pause_max: Duration::ZERO,
            allow_media: false,
            open_drawer_timeout: Duration::from_secs(20),
        }
    }
}

impl TasksOptions {
    /// Mapeia as opções a partir da configuração (mesmos campos do oráculo).
    #[must_use]
    pub fn from_config(config: &ali_coins_core::config::Config) -> Self {
        Self {
            max_actions: u32::try_from(config.task_max_actions).unwrap_or(25),
            max_attempts: u32::try_from(config.task_max_attempts).unwrap_or(4),
            max_round_attempts: u32::try_from(config.task_round_max_attempts).unwrap_or(3),
            max_duration: Duration::from_millis(config.task_max_duration_ms),
            scroll_wait_seconds: config.scroll_wait_seconds,
            task_scroll_max_ms: Duration::from_millis(config.task_scroll_max_ms),
            skip_app_only: config.skip_app_only_tasks,
            search_query: crate::tasks::SEARCH_QUERY.to_string(),
            nav_timeout: Duration::from_millis(config.nav_timeout),
            nav_timeout_short: Duration::from_millis(config.nav_timeout_short),
            retry_unfinished: config.task_retry_unfinished,
            retry_passes: u32::try_from(config.task_retry_passes).unwrap_or(1),
            retry_delay: Duration::from_millis(config.task_retry_delay_ms),
            pause_min: Duration::from_millis(config.task_pause_min_ms),
            pause_max: Duration::from_millis(config.task_pause_max_ms),
            allow_media: config.allow_media,
            open_drawer_timeout: Duration::from_secs(20),
        }
    }
}

/// Status final por tarefa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskOutcome {
    /// Título.
    pub title: String,
    /// Status contratual (C-15).
    pub status: String,
    /// Moedas estimadas exibidas no card.
    pub coins: String,
}

/// Resultado do run.
#[derive(Debug, Clone, PartialEq)]
pub struct TasksRun {
    /// Resultados por tarefa.
    pub results: Vec<TaskOutcome>,
    /// Ações executadas (teto `TASK_MAX_ACTIONS`).
    pub actions: u32,
    /// Motivos de falha/desistência por título.
    pub failed_tasks: HashMap<String, String>,
}

impl TasksRun {
    /// Falhas reais (excluindo tarefas desativadas por exigirem o app).
    #[must_use]
    pub fn real_failures(&self) -> usize {
        self.failed_tasks
            .values()
            .filter(|reason| reason.as_str() != APP_ONLY_DISABLED_STATUS)
            .count()
    }

    /// Houve falhas reais (usado para o exit code do `tasks` avulso).
    #[must_use]
    pub fn had_actions(&self) -> bool {
        self.actions > 0
    }
}

fn random_fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos());
    f64::from(nanos) / 1_000_000_000.0
}

/// Executa um future com teto de tempo (evita CDP pendurado bloquear o run).
async fn bounded<F, T, E>(ms: u64, future: F) -> Option<T>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    tokio::time::timeout(Duration::from_millis(ms), future)
        .await
        .ok()?
        .ok()
}

/// Fecha abas abertas por tarefas que não sejam a página principal.
async fn close_orphan_pages(browser: &dyn Browser, main_url: &str) {
    let Some(pages) = bounded(5000, browser.pages()).await else {
        return;
    };
    for page in pages {
        let Some(url) = bounded(2000, page.url()).await else {
            continue;
        };
        if url.is_empty() || url == main_url {
            continue;
        }
        let _ = bounded(3000, page.close()).await;
    }
}

/// URLs das páginas abertas (para detectar novas abas após um clique).
async fn open_page_urls(browser: &dyn Browser) -> Vec<String> {
    let Some(pages) = bounded(5000, browser.pages()).await else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for page in pages {
        if let Some(url) = bounded(2000, page.url()).await {
            if !url.is_empty() {
                urls.push(url);
            }
        }
    }
    urls
}

/// Primeira página cuja URL não era conhecida (aba nova/navegação), ignorando
/// páginas de detalhe de produto (sobras de toques anteriores).
///
/// Retorna cedo quando nada mudou; só aguarda quando há aba carregando.
async fn find_changed_page(
    browser: &dyn Browser,
    known: &[String],
    main: &dyn Page,
) -> Option<Box<dyn Page>> {
    let main_url = bounded(1000, main.url()).await.unwrap_or_default();
    for _ in 0..8 {
        let pages = bounded(2500, browser.pages()).await?;
        let mut loading_tab = false;
        for page in pages {
            let url = bounded(1000, page.url()).await.unwrap_or_default();
            if url.is_empty() {
                if !main_url.is_empty() {
                    loading_tab = true;
                }
                continue;
            }
            if !known.contains(&url) {
                if url.contains("/item/") || url.contains("/detail/") {
                    continue;
                }
                return Some(page);
            }
        }
        if !loading_tab {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    None
}

/// Executa as tarefas diárias (port do laço de `do_tasks.js`).
pub async fn run_tasks(
    page: &dyn Page,
    browser: &dyn Browser,
    options: &TasksOptions,
) -> Result<TasksRun, TasksError> {
    ensure_coin_page(page, options.nav_timeout).await?;
    let _ = crate::navigation::close_modals(page).await;
    if !open_task_drawer(page, options.open_drawer_timeout).await {
        return Err(TasksError::DrawerMissing);
    }

    let dispatch_options = DispatchOptions {
        scroll_wait_seconds: options.scroll_wait_seconds,
        task_scroll_max_ms: options.task_scroll_max_ms,
        search_query: options.search_query.clone(),
    };

    let mut task_attempts: HashMap<String, u32> = HashMap::new();
    let mut round_attempts: HashMap<String, u32> = HashMap::new();
    let mut failed_tasks: HashMap<String, String> = HashMap::new();
    let mut task_progress: HashMap<String, Option<u32>> = HashMap::new();
    let mut task_statuses: HashMap<String, String> = HashMap::new();
    let mut exhausted: HashSet<String> = HashSet::new();
    let mut disabled_logged: HashSet<String> = HashSet::new();
    let mut touched_cards: HashSet<String> = HashSet::new();
    let mut actions = 0_u32;
    let mut last_tasks: Vec<TaskItem> = Vec::new();

    if options.skip_app_only {
        ali_coins_core::logging::global().warn(
            "Verificação das tarefas exclusivas do app DESLIGADA (SKIP_APP_ONLY_TASKS=true): Prize Land/regar, minigames, quizzes e avaliações serão ignorados.",
            &[],
        );
    }
    if options.retry_unfinished {
        ali_coins_core::logging::global().warn(
            &format!(
                "Segunda passada ATIVADA (TASK_RETRY_UNFINISHED=true): até {} passada(s) extra(s) apenas para tarefas incompletas.",
                options.retry_passes
            ),
            &[],
        );
    }

    let max_passes = if options.retry_unfinished {
        options.retry_passes
    } else {
        0
    };

    for pass in 0..=max_passes {
        if pass > 0 {
            let mut reopened: Vec<String> = Vec::new();
            for title in &exhausted {
                if failed_tasks.get(title).map(String::as_str) == Some(APP_ONLY_DISABLED_STATUS) {
                    continue;
                }
                failed_tasks.remove(title);
                task_attempts.insert(title.clone(), 0);
                if let Some(status) = task_statuses.get(title) {
                    round_attempts.insert(format!("{title}::{status}"), 0);
                }
                reopened.push(title.clone());
            }
            exhausted.clear();
            if reopened.is_empty() {
                ali_coins_core::logging::global().info(
                    "Segunda passada: nenhuma tarefa incompleta para reabrir. Encerrando.",
                    &[],
                );
                break;
            }
            ali_coins_core::logging::global().info(
                &format!(
                    "Segunda passada {pass}/{max_passes}: reabrindo {} tarefa(s) incompleta(s): {}",
                    reopened.len(),
                    reopened.join(" | ")
                ),
                &[],
            );
            if !options.retry_delay.is_zero() {
                tokio::time::sleep(options.retry_delay).await;
            }
        }

        while actions < options.max_actions {
            let tasks = match get_drawer_tasks_with_retry(
                page,
                options.nav_timeout_short,
                Duration::from_secs(20),
                2,
            )
            .await
            {
                Ok(tasks) => tasks,
                Err(error) => {
                    ali_coins_core::logging::global().error(
                        &format!(
                            "Falha persistente ao ler painel de tarefas no loop. Encerrando etapa para evitar loop infinito: {error}"
                        ),
                        &[],
                    );
                    break;
                }
            };
            last_tasks.clone_from(&tasks);

            if tasks.is_empty() {
                ali_coins_core::logging::global().info(
                    "Nenhuma tarefa pendente encontrada no painel. Etapa concluída com sucesso.",
                    &[],
                );
                break;
            }

            if options.skip_app_only {
                for task in &tasks {
                    if !task.is_done
                        && is_interactive_or_app_only(task)
                        && disabled_logged.insert(task.title.clone())
                    {
                        ali_coins_core::logging::global().warn(
                            &format!(
                                "Tarefa \"{}\" exige o app e está desativada (SKIP_APP_ONLY_TASKS=true). Ignorando.",
                                task.title
                            ),
                            &[],
                        );
                    }
                }
            }

            // Progresso de rodada/status reseta tentativas consecutivas.
            for task in &tasks {
                if let Some(completed) = task.completed_rounds {
                    let previous = task_progress
                        .get(&task.title)
                        .copied()
                        .flatten()
                        .map_or(-1_i64, i64::from);
                    if i64::from(completed) > previous {
                        if previous >= 0 {
                            ali_coins_core::logging::global().info(
                                &format!(
                                    "Tarefa \"{}\" avançou de rodada ({}/{}). Resetando tentativas.",
                                    task.title,
                                    completed,
                                    task.total_rounds.unwrap_or(0)
                                ),
                                &[],
                            );
                            reset_task_attempt(&mut task_attempts, &task.title);
                        }
                        task_progress.insert(task.title.clone(), Some(completed));
                    }
                }
                if !task.status_text.is_empty() {
                    let previous = task_statuses.get(&task.title).cloned();
                    if previous
                        .as_deref()
                        .is_some_and(|value| value != task.status_text)
                    {
                        reset_task_attempt(&mut task_attempts, &task.title);
                    }
                    task_statuses.insert(task.title.clone(), task.status_text.clone());
                }
            }

            let pending = find_next_pending_task(
                &tasks,
                &mut task_attempts,
                options.max_attempts,
                &mut round_attempts,
                options.max_round_attempts,
                &mut failed_tasks,
                options.skip_app_only,
            )
            .cloned();
            let Some(pending) = pending else {
                ali_coins_core::logging::global().info(
                    "Todas as tarefas disponíveis foram concluídas ou verificadas.",
                    &[],
                );
                break;
            };

            // Pausa aleatória antes da tarefa (padrão desligada).
            let pause_ms = ali_coins_core::time::pick_pause_ms(
                options.pause_min.as_secs_f64() * 1000.0,
                options.pause_max.as_secs_f64() * 1000.0,
                random_fraction(),
            );
            if pause_ms > 0 {
                ali_coins_core::logging::global().info(
                    &format!("Pausa de {}s antes da próxima tarefa.", pause_ms / 1000),
                    &[],
                );
                tokio::time::sleep(Duration::from_millis(pause_ms)).await;
            }

            let round_key = get_round_key(&pending);
            record_task_attempt(&mut task_attempts, &pending.title);
            record_round_attempt(&mut round_attempts, &round_key);

            let task_started = Instant::now();
            let round_info = pending.total_rounds.map_or_else(String::new, |total| {
                format!(
                    " [Rodada {}/{}]",
                    pending.completed_rounds.unwrap_or(0) + 1,
                    total
                )
            });
            ali_coins_core::logging::global().info(
                &format!(
                    "\n--- Executando: \"{}\" ({}){round_info} ---",
                    pending.title, pending.coins
                ),
                &[],
            );

            // Limpa abas de detalhe deixadas por toques anteriores e tira o
            // snapshot das páginas antes do clique.
            if let Some(url) = bounded(1000, page.url()).await {
                close_orphan_pages(browser, &url).await;
            }
            let known_urls = open_page_urls(browser).await;
            let clicked = if pending.is_claimable {
                click_task_button(page, &pending.title).await
            } else {
                // Ações GO/IR usam clique trusted (handlers do site podem exigir input real).
                crate::tasks_verifier::click_task_button_trusted(page, &pending.title).await
            };
            if !clicked {
                continue;
            }
            actions += 1;

            // Diagnóstico da tarefa de surpresa: gaveta fechou? cards visíveis?
            let pending_text = format!("{} {}", pending.title, pending.desc).to_lowercase();
            let is_surprise = ["surprise", "surpresa", "tap 3", "toque em 3"]
                .iter()
                .any(|needle| pending_text.contains(needle));
            if is_surprise {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let drawer = crate::tasks_verifier::drawer_open(page).await;
                let cards = bounded(
                    3000,
                    page.eval_raw("document.querySelectorAll('.feeds-discount-card').length"),
                )
                .await
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
                let url = bounded(2000, page.url()).await.unwrap_or_default();
                ali_coins_core::logging::global().info(
                    &format!("Pós-GO da surpresa: gaveta_aberta={drawer} cards={cards} url={url}"),
                    &[],
                );
                crate::tasks_surprise::save_debug(page, "surprise-after-go").await;
                let loading_done =
                    crate::tasks_verifier::wait_drawer_loading(page, Duration::from_secs(25)).await;
                let drawer_snippet = crate::tasks_verifier::drawer_text(page).await;
                ali_coins_core::logging::global().info(
                    &format!("Pós-GO: loading concluído={loading_done} | gaveta: {drawer_snippet}"),
                    &[],
                );
                let closed = crate::tasks_verifier::close_task_drawer(page).await;
                let hit = bounded(
                    3000,
                    page.eval_raw(
                        "(() => { const el = document.querySelector('.feeds-discount-card'); \
                         if (!el) return ''; if (el.scrollIntoView) el.scrollIntoView({ block: 'center' }); \
                         const r = el.getBoundingClientRect(); \
                         const target = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); \
                         return target ? (target.className || target.tagName) : ''; })()",
                    ),
                )
                .await
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
                ali_coins_core::logging::global().info(
                    &format!(
                        "Gaveta fechada para os toques: {closed} | elemento no centro do card: {hit}"
                    ),
                    &[],
                );
            }

            // Caso 1: resgate/coleta.
            if pending.is_claimable {
                ali_coins_core::logging::global().info(
                    &format!(
                        "Resgatando recompensa da tarefa \"{}\" (botão \"{}\")...",
                        pending.title, pending.btn_text
                    ),
                    &[],
                );
                tokio::time::sleep(Duration::from_millis(2000)).await;
                let _ = crate::navigation::close_modals(page).await;
                reset_task_attempt(&mut task_attempts, &pending.title);
                continue;
            }

            // Caso 2: ação executável (GO/IR) — clique já disparado acima.
            wait_dom_content_loaded(page, options.nav_timeout_short).await;
            tokio::time::sleep(Duration::from_millis(1500)).await;

            // A ação roda na aba nova quando o clique abriu uma (como o oráculo);
            // a aba criada pelo site não herda a emulação da principal, então
            // reaplicamos o perfil Pixel 7 antes de agir nela.
            let changed_page = find_changed_page(browser, &known_urls, page).await;
            let action_page: &dyn Page = changed_page.as_deref().unwrap_or(page);
            if changed_page.is_some() {
                let _ = action_page
                    .set_device_profile(&ali_coins_browser::launch::pixel7_profile())
                    .await;
                let _ = action_page
                    .enable_resource_blocking(options.allow_media)
                    .await;
                // A aba nova carregou sem emulação (UA desktop) e caiu no layout PC:
                // recarrega já emulada para o site servir o mobile. Se já é a página
                // mobile, NÃO recarrega (evita resetar o feed da tarefa).
                if let Some(url) = bounded(2000, action_page.url()).await {
                    if url.contains("coin-pc-index") {
                        let _ = bounded(
                            u64::try_from(options.nav_timeout_short.as_millis())
                                .unwrap_or(u64::MAX)
                                + 5000,
                            action_page.goto(
                                &url,
                                &ali_coins_browser::driver::NavOptions {
                                    timeout: Some(options.nav_timeout_short),
                                    wait_until: Some("domcontentloaded".to_string()),
                                },
                            ),
                        )
                        .await;
                    }
                }
                if let Ok(url) = action_page.url().await {
                    ali_coins_core::logging::global()
                        .info(&format!("Ação será executada na aba/página: {url}"), &[]);
                }
            }
            let attempt = task_attempts.get(&pending.title).copied().unwrap_or(1);
            let outcome = tokio::time::timeout(
                options.max_duration,
                execute_task_action(
                    Some(browser),
                    action_page,
                    &pending,
                    attempt,
                    &dispatch_options,
                    &mut touched_cards,
                ),
            )
            .await;
            match outcome {
                Ok(result) => {
                    if result.is_special_or_app_only {
                        mark_special_or_app_only(&mut task_attempts, &pending.title);
                    }
                }
                Err(_) => {
                    ali_coins_core::logging::global().warn(
                        &format!(
                            "Tempo limite de execução atingido para \"{}\" ({}ms). Abortando tentativa.",
                            pending.title,
                            options.max_duration.as_millis()
                        ),
                        &[],
                    );
                }
            }

            if let Ok(url) = page.url().await {
                close_orphan_pages(browser, &url).await;
            }
            let _ = ensure_coin_page(page, options.nav_timeout_short).await;
            tokio::time::sleep(Duration::from_millis(2500)).await;

            ali_coins_core::logging::global().info(
                &format!(
                    "Concluída ação em: {}",
                    ali_coins_core::time::format_duration(
                        i64::try_from(task_started.elapsed().as_millis()).unwrap_or(i64::MAX)
                    )
                ),
                &[],
            );
        }

        if options.retry_unfinished && pass < max_passes {
            for title in select_reopenable_tasks(&failed_tasks, &last_tasks) {
                exhausted.insert(title);
            }
        }
        if actions >= options.max_actions {
            ali_coins_core::logging::global().warn(
                "Teto global de ações atingido; encerrando novas passadas para não estourar o tempo.",
                &[],
            );
            break;
        }
    }

    // Relatório final (com retry de extração, como o oráculo).
    let final_tasks =
        get_drawer_tasks_with_retry(page, options.nav_timeout_short, Duration::from_secs(20), 2)
            .await
            .unwrap_or_default();

    let mut results: Vec<TaskOutcome> = final_tasks
        .iter()
        .map(|task| TaskOutcome {
            title: task.title.clone(),
            status: failed_tasks
                .get(&task.title)
                .cloned()
                .unwrap_or_else(|| task.report_status(&failed_tasks, options.skip_app_only)),
            coins: task.coins.clone(),
        })
        .collect();
    for (title, reason) in &failed_tasks {
        if !results.iter().any(|result| &result.title == title) {
            results.push(TaskOutcome {
                title: title.clone(),
                status: reason.clone(),
                coins: "+0 moedas".to_string(),
            });
        }
    }

    Ok(TasksRun {
        results,
        actions,
        failed_tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ali_coins_browser::driver::{BrowserDriver as _, LaunchOptions};
    use ali_coins_browser::mock::{MockDriver, MockPageSpec};
    use serde_json::json;

    fn task_json(title: &str, button: &str, rounds: Option<(u32, u32)>) -> serde_json::Value {
        json!({
            "index": 0,
            "title": title,
            "desc": "",
            "btnText": button,
            "btnStyle": "",
            "statusText": rounds.map(|(c, t)| format!("{c}/{t}")).unwrap_or_default(),
            "completedRounds": rounds.map(|(c, _)| c),
            "currentRound": rounds.map(|(c, t)| (c + 1).min(t)),
            "totalRounds": rounds.map(|(_, t)| t),
            "isDone": rounds.is_some_and(|(c, t)| c >= t),
            "isActionable": button == "GO" || button == "IR",
            "isClaimable": button == "COLLECT",
            "groupId": "",
            "coins": "+5 moedas",
            "allText": title
        })
    }

    #[tokio::test]
    async fn runner_executa_claimable_e_conclui() {
        let tasks = json!([task_json("Tarefa A", "COLLECT", None)]);
        let driver = MockDriver::new(vec![MockPageSpec {
            visible_selectors: vec![".e2e_task".to_string()],
            eval_contains: vec![
                (
                    "getBoundingClientRect().height > 100".to_string(),
                    json!(true),
                ),
                ("els.findIndex".to_string(), json!(0)),
                ("btn.click(); return true;".to_string(), json!(true)),
                (
                    "document.querySelectorAll('.e2e_normal_task')".to_string(),
                    tasks,
                ),
            ],
            ..MockPageSpec::default()
        }]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        let options = TasksOptions {
            max_actions: 3,
            max_attempts: 2,
            open_drawer_timeout: Duration::from_millis(200),
            ..TasksOptions::default()
        };
        let run = run_tasks(&*page, &*browser, &options)
            .await
            .expect("runner");
        assert!(run.actions >= 1);
    }

    #[tokio::test]
    async fn gaveta_ausente_vira_erro() {
        let driver = MockDriver::new(vec![MockPageSpec::default()]);
        let browser = driver.launch(&LaunchOptions::default()).await.unwrap();
        let page = browser.new_page().await.unwrap();
        let options = TasksOptions {
            open_drawer_timeout: Duration::from_millis(200),
            ..TasksOptions::default()
        };
        let error = run_tasks(&*page, &*browser, &options)
            .await
            .expect_err("sem gaveta");
        assert!(matches!(error, TasksError::DrawerMissing));
    }
}
