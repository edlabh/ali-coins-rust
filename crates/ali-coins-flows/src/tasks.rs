//! Tarefas do painel "Ganhe mais moedas": estados, prioridade e roteamento.
//!
//! Porta a parte pura do oráculo (`libs/tasks/state.js` + `dispatcher.js` +
//! `search.js`): regexes de botão, status textual contratual (C-15), detecção
//! de tarefas exclusivas do app e escolha do tipo de tarefa.
//!
//! As listas de palavras-chave de app-only serão alinhadas com o arquivo
//! original na paridade com o site real (D-09).

use std::sync::OnceLock;

/// Query fixa da tarefa de busca do oráculo.
pub const SEARCH_QUERY: &str = "fone bluetooth";

/// Status usado quando as tarefas de app são puladas por configuração.
pub const APP_ONLY_DISABLED_STATUS: &str = "Desativada (tarefas que exigem o app desligadas)";

/// Tipo de tarefa roteado pelo dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// Produtos surpresa (3 cards).
    Surprise,
    /// Busca por palavra-chave.
    Search,
    /// Prize Land / regar / valor 0.1.
    PrizeLand,
    /// Minigame/quiz exclusivo do app.
    InteractiveOrAppOnly,
    /// Avaliação de pedido.
    Review,
    /// Navegação genérica com scroll.
    Navigation,
}

/// Rounds da tarefa (`x/y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rounds {
    /// Rounds concluídas.
    pub completed: u32,
    /// Rounds totais.
    pub total: u32,
}

impl Rounds {
    /// Está completa?
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.total > 0 && self.completed >= self.total
    }
}

fn regex(pattern: &'static str) -> &'static regex::Regex {
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static regex::Regex>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut cache = cache.lock().expect("cache de regex");
    if let Some(found) = cache.get(pattern) {
        return found;
    }
    let compiled: &'static regex::Regex =
        Box::leak(Box::new(regex::Regex::new(pattern).expect("regex válida")));
    cache.insert(pattern, compiled);
    compiled
}

/// Botão é de resgate (claim)?
#[must_use]
pub fn is_claimable(button_text: &str) -> bool {
    regex(r"(?i)^(COLLECT|COLETAR|GET|RECEBER|CLAIM|RESGATAR|\+[0-9]+)")
        .is_match(button_text.trim())
}

/// Botão é de ação (`GO`/`IR`)?
#[must_use]
pub fn is_actionable(button_text: &str) -> bool {
    regex(r"(?i)^(GO|IR)$").is_match(button_text.trim())
}

/// Rounds no formato `x/y`.
#[must_use]
pub fn parse_rounds(text: &str) -> Option<Rounds> {
    let captures = regex(r"^\s*([0-9]+)\s*/\s*([0-9]+)\s*$").captures(text)?;
    let completed = captures.get(1)?.as_str().parse().ok()?;
    let total = captures.get(2)?.as_str().parse().ok()?;
    Some(Rounds { completed, total })
}

/// Tarefa marcada como concluída (rounds completos ou marcador DONE/opacidade).
#[must_use]
pub fn is_done(button_text: &str, rounds: Option<Rounds>, opacity: Option<f64>) -> bool {
    if rounds.is_some_and(Rounds::is_complete) {
        return true;
    }
    if opacity.is_some_and(|value| (value - 0.5).abs() < f64::EPSILON) {
        return true;
    }
    regex(r"(?i)\bDONE\b").is_match(button_text)
}

/// Palavras-chave de tarefas exclusivas/interativas do app.
pub const APP_ONLY_KEYWORDS: [&str; 10] = [
    "prize land",
    "prizeland",
    "minigame",
    "mini game",
    "quiz",
    "merge boss",
    "regar",
    "water",
    "avali",
    "review",
];

/// Tarefa interativa/exclusiva do app (minigame, quiz, regar, avaliação).
#[must_use]
pub fn is_interactive_or_app_only(title: &str, description: &str) -> bool {
    let text = format!("{title} {description}").to_lowercase();
    APP_ONLY_KEYWORDS
        .iter()
        .any(|keyword| text.contains(keyword))
}

/// Status textual contratual da tarefa (C-15).
#[must_use]
pub fn classify_task_status(
    button_text: &str,
    rounds: Option<Rounds>,
    opacity: Option<f64>,
    title: &str,
    description: &str,
    skip_app_only: bool,
) -> String {
    let app_only = is_interactive_or_app_only(title, description);
    if app_only && skip_app_only {
        return APP_ONLY_DISABLED_STATUS.to_string();
    }
    if is_done(button_text, rounds, opacity) {
        return match rounds {
            Some(rounds) => format!("Concluída ({}/{})", rounds.completed, rounds.total),
            None => "Concluída".to_string(),
        };
    }
    if app_only {
        let lower = format!("{title} {description}").to_lowercase();
        if lower.contains("avali") || lower.contains("review") {
            return "Requer pedido entregue elegível para avaliação".to_string();
        }
        if lower.contains("regar") || lower.contains("water") {
            return "Exclusiva do App AliExpress (requer rega no app móvel)".to_string();
        }
        return "Requer interação direta no App AliExpress (minigame/quiz)".to_string();
    }
    if let Some(rounds) = rounds {
        if rounds.completed > 0 && !rounds.is_complete() {
            return format!(
                "Executada parcialmente ({}/{})",
                rounds.completed, rounds.total
            );
        }
    }
    if !is_claimable(button_text) && !is_actionable(button_text) && !button_text.trim().is_empty() {
        return format!(
            "Requer verificação manual (botão \"{}\" não reconhecido)",
            button_text.trim()
        );
    }
    "Pendente".to_string()
}

/// Mensagem de falha por limite de tentativas.
#[must_use]
pub fn failure_attempts_message(attempts: u32) -> String {
    format!("Falhou (limite de {attempts} tentativas atingido)")
}

/// Mensagem de falha por ausência de progresso.
#[must_use]
pub fn failure_no_progress_message(attempts: u32) -> String {
    format!("Falhou (sem progresso após {attempts} tentativas)")
}

/// Prioridade: claimable antes de actionable, o resto por ordem.
#[must_use]
pub fn priority(button_text: &str) -> u8 {
    if is_claimable(button_text) {
        0
    } else if is_actionable(button_text) {
        1
    } else {
        2
    }
}

/// Roteia o tipo de tarefa (mesma ordem do dispatcher do oráculo).
#[must_use]
pub fn classify_task_kind(title: &str, description: &str, button_text: &str) -> TaskKind {
    let text = format!("{title} {description}").to_lowercase();
    let button = button_text.trim();
    if regex(r"(?i)(surprise|surpresa)|(tap\s*3|toque\s*em\s*3)").is_match(&text) {
        return TaskKind::Surprise;
    }
    if regex(r"(?i)(search|pesquisa|buscar|keywords|palavra)").is_match(&text) {
        return TaskKind::Search;
    }
    if regex(r"(?i)(prize\s*land|prizeland|regar|water|0\.1)").is_match(&text) {
        return TaskKind::PrizeLand;
    }
    if regex(r"(?i)(game|jogo|quiz)").is_match(&text) {
        return TaskKind::InteractiveOrAppOnly;
    }
    if regex(r"(?i)(review|avali)").is_match(&text) {
        return TaskKind::Review;
    }
    if is_claimable(button) || is_actionable(button) {
        return TaskKind::Navigation;
    }
    TaskKind::Navigation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regexes_de_botao() {
        assert!(is_claimable("COLLECT"));
        assert!(is_claimable("+5 moedas"));
        assert!(is_claimable("Resgatar"));
        assert!(!is_claimable("GO"));
        assert!(is_actionable("GO"));
        assert!(is_actionable("ir"));
        assert!(!is_actionable("GO agora"));
    }

    #[test]
    fn rounds() {
        assert_eq!(
            parse_rounds("2/3"),
            Some(Rounds {
                completed: 2,
                total: 3
            })
        );
        assert_eq!(
            parse_rounds(" 1 / 1 "),
            Some(Rounds {
                completed: 1,
                total: 1
            })
        );
        assert_eq!(parse_rounds("x/y"), None);
        assert!(parse_rounds("3/3").unwrap().is_complete());
    }

    #[test]
    fn status_concluida_e_parcial() {
        assert_eq!(
            classify_task_status("", parse_rounds("3/3"), None, "Tarefa", "", true),
            "Concluída (3/3)"
        );
        assert_eq!(
            classify_task_status("GO", parse_rounds("1/3"), None, "Tarefa", "", true),
            "Executada parcialmente (1/3)"
        );
    }

    #[test]
    fn status_app_only() {
        assert_eq!(
            classify_task_status("GO", None, None, "Merge Boss", "minigame", true),
            APP_ONLY_DISABLED_STATUS
        );
        assert_eq!(
            classify_task_status("GO", None, None, "Prize Land", "regar", false),
            "Exclusiva do App AliExpress (requer rega no app móvel)"
        );
        assert_eq!(
            classify_task_status("GO", None, None, "Avalie", "review do pedido", false),
            "Requer pedido entregue elegível para avaliação"
        );
    }

    #[test]
    fn status_botao_desconhecido() {
        assert_eq!(
            classify_task_status("WATCH", None, None, "Tarefa", "", true),
            "Requer verificação manual (botão \"WATCH\" não reconhecido)"
        );
        assert_eq!(
            classify_task_status("GO", None, None, "Tarefa", "", true),
            "Pendente"
        );
    }

    #[test]
    fn prioridade_e_mensagens() {
        assert_eq!(priority("COLLECT"), 0);
        assert_eq!(priority("GO"), 1);
        assert_eq!(priority("WATCH"), 2);
        assert_eq!(
            failure_attempts_message(4),
            "Falhou (limite de 4 tentativas atingido)"
        );
        assert_eq!(
            failure_no_progress_message(3),
            "Falhou (sem progresso após 3 tentativas)"
        );
    }

    #[test]
    fn dispatcher_de_tipos() {
        assert_eq!(
            classify_task_kind("Toque em 3 produtos surpresa", "", "GO"),
            TaskKind::Surprise
        );
        assert_eq!(
            classify_task_kind("Pesquise por palavra-chave", "", "GO"),
            TaskKind::Search
        );
        assert_eq!(SEARCH_QUERY, "fone bluetooth");
        assert_eq!(
            classify_task_kind("Regue a horta", "Prize Land", "GO"),
            TaskKind::PrizeLand
        );
        assert_eq!(
            classify_task_kind("Complete o jogo", "quiz diário", "GO"),
            TaskKind::InteractiveOrAppOnly
        );
        assert_eq!(
            classify_task_kind("Avalie a compra", "", "GO"),
            TaskKind::Review
        );
        assert_eq!(
            classify_task_kind("Visite a loja", "navegue", "GO"),
            TaskKind::Navigation
        );
    }
}
