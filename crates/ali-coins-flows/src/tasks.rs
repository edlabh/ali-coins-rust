//! Máquina de estados, extração e classificações das tarefas do painel
//! "Ganhe mais moedas" — port fiel de `libs/tasks/state.js` + `verifier.js`.
//!
//! Tarefas extraídas carregam os mesmos sinais do oráculo (`isDone`,
//! `isClaimable`, `isActionable`, rodadas, status) e o laço de execução usa
//! as mesmas regras de tentativas (por tarefa e por rodada) e desistências.
#![allow(clippy::implicit_hasher)]

use ali_coins_browser::driver::BrowserError;
use serde_json::Value;
use std::collections::HashMap;

/// Query fixa da tarefa de busca (como no oráculo).
pub const SEARCH_QUERY: &str = "fone bluetooth";

/// Status de tarefas exclusivas do app quando `SKIP_APP_ONLY_TASKS=true`.
pub const APP_ONLY_DISABLED_STATUS: &str = "Desativada (tarefas que exigem o app desligadas)";

/// Erros do fluxo de tarefas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TasksError {
    /// Gaveta de tarefas inacessível.
    DrawerMissing,
    /// Falha persistente de extração.
    Extract(String),
    /// Falha de browser.
    Browser(String),
}

impl std::fmt::Display for TasksError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DrawerMissing => write!(formatter, "Gaveta de tarefas não encontrada na página."),
            Self::Extract(message) | Self::Browser(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for TasksError {}

impl From<BrowserError> for TasksError {
    fn from(error: BrowserError) -> Self {
        Self::Browser(error.to_string())
    }
}

/// Tarefa extraída da gaveta (campos de `extractTasksFromDrawer`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskItem {
    /// Índice na gaveta.
    pub index: u64,
    /// Título.
    pub title: String,
    /// Descrição/subtítulo.
    pub desc: String,
    /// Texto do botão.
    pub btn_text: String,
    /// Atributo `style` do botão.
    pub btn_style: String,
    /// Texto de status (rodadas `x/y`).
    pub status_text: String,
    /// Rodadas concluídas.
    pub completed_rounds: Option<u32>,
    /// Rodada atual (`min(completed+1, total)`).
    pub current_round: Option<u32>,
    /// Total de rodadas.
    pub total_rounds: Option<u32>,
    /// Tarefa totalmente concluída.
    pub is_done: bool,
    /// Botão de ação (`GO`/`IR`).
    pub is_actionable: bool,
    /// Botão de resgate/coleta.
    pub is_claimable: bool,
    /// `data-groupid` do card.
    pub group_id: String,
    /// Moedas estimadas (`+5 moedas`).
    pub coins: String,
    /// Texto completo do card (com quebras normalizadas).
    pub all_text: String,
}

impl TaskItem {
    /// Status exibido no relatório final.
    #[must_use]
    pub fn report_status(&self, failed: &HashMap<String, String>, skip_app_only: bool) -> String {
        if let Some(reason) = failed.get(&self.title) {
            return reason.clone();
        }
        if self.is_done {
            if let Some(total) = self.total_rounds {
                return format!("Concluída ({total}/{total})");
            }
            return if self.status_text.is_empty() {
                "Concluída".to_string()
            } else {
                format!("Concluída ({})", self.status_text)
            };
        }
        if is_interactive_or_app_only(self) {
            if skip_app_only {
                return APP_ONLY_DISABLED_STATUS.to_string();
            }
            let title = self.title.to_lowercase();
            let desc = self.desc.to_lowercase();
            if title.contains("review")
                || title.contains("avalia")
                || desc.contains("review")
                || desc.contains("avalia")
            {
                return "Requer pedido entregue elegível para avaliação".to_string();
            }
            if title.contains("quiz") || title.contains("merge boss") {
                return "Requer interação direta no App AliExpress (minigame/quiz)".to_string();
            }
            return "Exclusiva do App AliExpress (requer rega no app móvel)".to_string();
        }
        if !self.status_text.is_empty() {
            return format!("Executada parcialmente ({})", self.status_text);
        }
        if !self.btn_text.is_empty()
            && !self.is_actionable
            && !self.is_claimable
            && !is_done_button_text(&self.btn_text)
        {
            return format!(
                "Requer verificação manual (botão \"{}\" não reconhecido)",
                self.btn_text
            );
        }
        "Pendente".to_string()
    }
}

fn is_done_button_text(text: &str) -> bool {
    let upper = text.to_uppercase();
    upper == "DONE" || upper == "COMPLETED" || upper == "CONCLUÍDO" || upper == "CONCLUIDO"
}

/// Tarefa exige app nativo ou interação especial (port de `isInteractiveOrAppOnly`).
#[must_use]
pub fn is_interactive_or_app_only(task: &TaskItem) -> bool {
    let text = format!("{} {}", task.title, task.desc).to_lowercase();
    [
        "prize land",
        "0.1",
        "water",
        "regar",
        "merge boss",
        "game",
        "jogo",
        "quiz",
        "review",
        "avalia",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Chave de rodada (`título:::status`).
#[must_use]
pub fn get_round_key(task: &TaskItem) -> String {
    format!("{}::{}", task.title, task.status_text)
}

/// Registra tentativa da tarefa (retorna o novo contador).
pub fn record_task_attempt(attempts: &mut HashMap<String, u32>, title: &str) -> u32 {
    let counter = attempts.entry(title.to_string()).or_insert(0);
    *counter += 1;
    *counter
}

/// Registra tentativa de rodada (retorna o novo contador).
pub fn record_round_attempt(attempts: &mut HashMap<String, u32>, round_key: &str) -> u32 {
    if round_key.is_empty() {
        return 0;
    }
    let counter = attempts.entry(round_key.to_string()).or_insert(0);
    *counter += 1;
    *counter
}

/// Zera as tentativas de uma tarefa (progresso de rodada/status).
pub fn reset_task_attempt(attempts: &mut HashMap<String, u32>, title: &str) {
    attempts.insert(title.to_string(), 0);
}

/// Marca tarefa especial/app-only como concluída (não tentar de novo).
pub fn mark_special_or_app_only(attempts: &mut HashMap<String, u32>, title: &str) {
    attempts.insert(title.to_string(), 999);
}

/// Encontra a próxima tarefa pendente elegível (port de `findNextPendingTask`).
///
/// Muta `failed_tasks` com os motivos de desistência (app-only, tentativas,
/// rodadas), exatamente como o oráculo.
pub fn find_next_pending_task<'a>(
    tasks: &'a [TaskItem],
    attempts: &mut HashMap<String, u32>,
    max_attempts: u32,
    round_attempts: &mut HashMap<String, u32>,
    max_round_attempts: u32,
    failed_tasks: &mut HashMap<String, String>,
    skip_app_only: bool,
) -> Option<&'a TaskItem> {
    let is_eligible = |task: &TaskItem,
                       attempts: &mut HashMap<String, u32>,
                       round_attempts: &mut HashMap<String, u32>,
                       failed_tasks: &mut HashMap<String, String>| {
        if task.is_done || failed_tasks.contains_key(&task.title) {
            return false;
        }
        if skip_app_only && is_interactive_or_app_only(task) {
            failed_tasks
                .entry(task.title.clone())
                .or_insert_with(|| APP_ONLY_DISABLED_STATUS.to_string());
            return false;
        }
        let used = attempts.get(&task.title).copied().unwrap_or(0);
        if used >= max_attempts {
            failed_tasks.entry(task.title.clone()).or_insert_with(|| {
                format!("Falhou (limite de {max_attempts} tentativas atingido)")
            });
            return false;
        }
        let round_key = get_round_key(task);
        let round_used = round_attempts.get(&round_key).copied().unwrap_or(0);
        if round_used >= max_round_attempts {
            failed_tasks.entry(task.title.clone()).or_insert_with(|| {
                format!("Falhou (sem progresso após {max_round_attempts} tentativas)")
            });
            return false;
        }
        true
    };

    // Prioridade 1: resgate/coleta pendente.
    if let Some(task) = tasks
        .iter()
        .find(|task| is_eligible(task, attempts, round_attempts, failed_tasks) && task.is_claimable)
    {
        return Some(task);
    }

    // Prioridade 2: executável (GO/IR ou rodadas pendentes habilitadas).
    tasks.iter().find(|task| {
        if !is_eligible(task, attempts, round_attempts, failed_tasks) {
            return false;
        }
        let has_pending_rounds = task
            .completed_rounds
            .zip(task.total_rounds)
            .is_some_and(|(completed, total)| completed < total)
            && !task.btn_style.contains("opacity: 0.5");
        task.is_actionable || task.btn_text == "GO" || task.btn_text == "IR" || has_pending_rounds
    })
}

/// Títulos elegíveis para reabertura na segunda passada.
#[must_use]
pub fn select_reopenable_tasks(
    failed_tasks: &HashMap<String, String>,
    tasks: &[TaskItem],
) -> Vec<String> {
    let done: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|task| task.is_done)
        .map(|task| task.title.as_str())
        .collect();
    failed_tasks
        .iter()
        .filter(|(title, reason)| {
            !title.is_empty()
                && !reason.is_empty()
                && reason.as_str() != APP_ONLY_DISABLED_STATUS
                && !done.contains(title.as_str())
        })
        .map(|(title, _)| title.clone())
        .collect()
}

/// Converte um item JSON da extração em `TaskItem`.
#[must_use]
pub fn parse_task_item(value: &Value) -> Option<TaskItem> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    let boolean = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let title = text("title");
    if title.is_empty() {
        return None;
    }
    Some(TaskItem {
        index: value.get("index").and_then(Value::as_u64).unwrap_or(0),
        title,
        desc: text("desc"),
        btn_text: text("btnText"),
        btn_style: text("btnStyle"),
        status_text: text("statusText"),
        completed_rounds: number("completedRounds"),
        current_round: number("currentRound"),
        total_rounds: number("totalRounds"),
        is_done: boolean("isDone"),
        is_actionable: boolean("isActionable"),
        is_claimable: boolean("isClaimable"),
        group_id: text("groupId"),
        coins: text("coins"),
        all_text: text("allText"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, btn: &str) -> TaskItem {
        TaskItem {
            title: title.to_string(),
            btn_text: btn.to_string(),
            ..TaskItem::default()
        }
    }

    #[test]
    fn app_only_por_keywords() {
        assert!(is_interactive_or_app_only(&task("Prize Land", "GO")));
        assert!(is_interactive_or_app_only(&task(
            "Complete 1 Merge Boss game order",
            "GO"
        )));
        assert!(is_interactive_or_app_only(&task(
            "Daily quiz challenge",
            "GO"
        )));
        assert!(is_interactive_or_app_only(&task(
            "Items at $0.1 to get",
            "GO"
        )));
        assert!(!is_interactive_or_app_only(&task(
            "Browse surprise items",
            "GO"
        )));
    }

    #[test]
    fn proxima_tarefa_prioriza_claimable() {
        let tasks = vec![
            task("Tarefa GO", "GO"),
            TaskItem {
                is_claimable: true,
                ..task("Tarefa COLLECT", "COLLECT")
            },
        ];
        let mut attempts = HashMap::new();
        let mut rounds = HashMap::new();
        let mut failed = HashMap::new();
        let pending =
            find_next_pending_task(&tasks, &mut attempts, 4, &mut rounds, 3, &mut failed, true)
                .expect("pendente");
        assert_eq!(pending.title, "Tarefa COLLECT");
    }

    #[test]
    fn app_only_e_desistencia_entram_no_failed() {
        let tasks = vec![task("Daily quiz challenge", "GO"), task("Outra", "GO")];
        let mut attempts = HashMap::new();
        let mut rounds = HashMap::new();
        let mut failed = HashMap::new();
        let pending =
            find_next_pending_task(&tasks, &mut attempts, 4, &mut rounds, 3, &mut failed, true)
                .expect("pendente");
        assert_eq!(pending.title, "Outra");
        assert_eq!(
            failed.get("Daily quiz challenge").map(String::as_str),
            Some(APP_ONLY_DISABLED_STATUS)
        );
    }

    #[test]
    fn status_de_relatorio_igual_ao_oraculo() {
        let failed = HashMap::new();
        let done = TaskItem {
            title: "View Super discounts".to_string(),
            total_rounds: Some(3),
            is_done: true,
            ..TaskItem::default()
        };
        assert_eq!(
            done.report_status(&failed, true),
            "Concluída (3/3)".to_string()
        );

        let app_only = task("Daily quiz challenge", "GO");
        assert_eq!(
            app_only.report_status(&failed, true),
            APP_ONLY_DISABLED_STATUS.to_string()
        );

        let partial = TaskItem {
            status_text: "1/3".to_string(),
            ..task("View Super discounts", "GO")
        };
        assert_eq!(
            partial.report_status(&failed, true),
            "Executada parcialmente (1/3)".to_string()
        );

        let unknown = task("Nova tarefa", "FOO");
        assert!(
            unknown
                .report_status(&failed, true)
                .contains("verificação manual")
        );
    }

    #[test]
    fn extrai_item_json() {
        let value = serde_json::json!({
            "index": 2,
            "title": "Search for what you love",
            "desc": "keywords",
            "btnText": "GO",
            "btnStyle": "",
            "statusText": "1/2",
            "completedRounds": 1,
            "currentRound": 2,
            "totalRounds": 2,
            "isDone": false,
            "isActionable": true,
            "isClaimable": false,
            "groupId": "g1",
            "coins": "+5 moedas",
            "allText": "Search for what you love +5 moedas"
        });
        let item = parse_task_item(&value).expect("item");
        assert_eq!(item.index, 2);
        assert_eq!(item.title, "Search for what you love");
        assert_eq!(item.total_rounds, Some(2));
        assert!(item.is_actionable);
    }
}
