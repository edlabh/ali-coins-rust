//! Relatórios compatíveis com `libs/report.js` do oráculo — parte pura.
//!
//! Cobre os contratos C-09/C-10: tipos do payload, regras de streak e as
//! contabilidades de check-in/tarefas/saldo final. Os builders dos payloads e a
//! renderização texto/webhook entram no incremento de notificações.

use serde::{Deserialize, Serialize};

/// Valor de streak que o oráculo aceita como número ou texto (`N/D`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StreakValue {
    /// Número de dias.
    Number(i64),
    /// Texto (ex.: `N/D`).
    Text(String),
}

impl StreakValue {
    /// Representação de exibição (`to_string` do JS).
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Number(value) => value.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

/// Valor numérico ou textual usado em saldos de tarefas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NumOrText {
    /// Número.
    Number(f64),
    /// Texto.
    Text(String),
}

/// Entrada de check-in usada nas contabilidades.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinInput {
    /// Check-in já constava como feito hoje.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub already_collected: Option<bool>,
    /// Moedas exibidas hoje (pode ser `N/D`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coins_gained_today: Option<String>,
    /// Dias de streak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streak_days: Option<StreakValue>,
    /// Saldo total exibido.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_balance: Option<String>,
    /// Crédito vindo do extrato de hoje (contabiliza mesmo se alreadyCollected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin_coins_from_ledger: Option<bool>,
    /// Sinal explícito de que o saldo informado é pré-crédito do check-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_before_checkin: Option<bool>,
}

/// Entrada de tarefas usada nas contabilidades.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TasksInput {
    /// Ganho medido por diferença de saldo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coins_gained: Option<f64>,
    /// Ganho vindo do extrato (já isolado do check-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coins_from_ledger: Option<bool>,
    /// Saldo inicial capturado para as tarefas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_balance: Option<NumOrText>,
    /// Saldo final exibido (`N/D` quando indisponível).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_coins: Option<String>,
}

/// Parâmetros de `resolveStreakDays`.
#[derive(Debug, Clone, Default)]
pub struct ResolveStreakParams<'a> {
    /// Streak lido na tela.
    pub detected_streak: Option<&'a StreakValue>,
    /// Último streak persistido nos metadados.
    pub previous_streak_days: Option<i64>,
    /// Streak lido no desktop pré-check-in.
    pub early_desktop_streak: Option<i64>,
    /// Check-in recém-realizado nesta execução.
    pub just_collected: bool,
    /// Check-in já constava como feito hoje.
    pub already_collected: bool,
    /// Extrato confirma o bônus diário de hoje.
    pub confirmed_by_ledger: bool,
    /// Sequência lida no extrato desktop.
    pub statement_streak: Option<&'a StreakValue>,
}

/// Resultado de `resolveStreakDays`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStreak {
    /// Streak final.
    pub streak_days: StreakValue,
    /// Base numérica usada (meta/desktop), quando houver.
    pub base_streak: Option<i64>,
}

/// Dias por moedas do ciclo oficial (Dia 1..7+).
#[must_use]
pub fn checkin_coins_from_streak(streak: Option<&StreakValue>) -> i64 {
    let Some(value) = streak else {
        return 10;
    };
    if matches!(value, StreakValue::Text(text) if text == "N/D") {
        return 10;
    }
    match parse_streak(Some(value)) {
        Some(days) if days > 1 => match days {
            2 => 15,
            3 => 20,
            4 => 25,
            5 => 30,
            6 => 35,
            _ => 40,
        },
        _ => 10,
    }
}

/// Quebra real de streak (current=1, previous>1, não já coletado).
#[must_use]
#[allow(clippy::float_cmp)]
pub fn is_streak_break(
    current_streak: Option<f64>,
    previous_streak: Option<f64>,
    already_collected: bool,
) -> bool {
    let (Some(current), Some(previous)) = (current_streak, previous_streak) else {
        return false;
    };
    if !current.is_finite() || !previous.is_finite() {
        return false;
    }
    if previous <= 1.0 {
        return false;
    }
    if already_collected {
        return false;
    }
    if current > 1.0 && current < previous {
        return false;
    }
    current == 1.0 && previous > 1.0
}

/// Resolve o `streakDays` final aplicando incremento/reexecução/leitura espúria.
#[must_use]
pub fn resolve_streak_days(params: &ResolveStreakParams<'_>) -> ResolvedStreak {
    let candidates: Vec<i64> = [params.previous_streak_days, params.early_desktop_streak]
        .into_iter()
        .flatten()
        .filter(|value| *value > 0)
        .collect();
    let base_streak = candidates.iter().copied().max();

    let parsed_detected = parse_streak(params.detected_streak);
    let parsed_statement = parse_streak(params.statement_streak);
    let mut streak_days = params
        .detected_streak
        .cloned()
        .unwrap_or_else(|| StreakValue::Text("N/D".to_string()));

    if let Some(base) = base_streak.filter(|base| *base > 0) {
        let is_spurious =
            parsed_detected.is_some_and(|detected| base > 7 && (2..=7).contains(&detected));

        if params.just_collected || params.confirmed_by_ledger {
            if params.detected_streak.is_none()
                || parsed_detected.is_none()
                || is_spurious
                || parsed_detected.is_some_and(|detected| detected <= base)
            {
                streak_days = StreakValue::Number(base + 1);
            } else if let Some(detected) = parsed_detected {
                streak_days = StreakValue::Number(detected);
            }
        } else if params.already_collected {
            if params.detected_streak.is_none()
                || parsed_detected.is_none()
                || is_spurious
                || parsed_detected.is_some_and(|detected| detected < base)
            {
                streak_days = StreakValue::Number(base);
            } else if let Some(detected) = parsed_detected {
                streak_days = StreakValue::Number(detected);
            }
        } else if parsed_detected == Some(1)
            && base > 1
            && parsed_statement.is_some_and(|statement| statement > 1)
        {
            streak_days = StreakValue::Number(parsed_statement.unwrap_or(base));
        } else {
            streak_days = parsed_detected
                .map(StreakValue::Number)
                .unwrap_or(StreakValue::Number(base));
        }
    } else if params.just_collected || params.confirmed_by_ledger {
        streak_days = parsed_detected
            .filter(|detected| *detected >= 1)
            .map_or(StreakValue::Number(1), StreakValue::Number);
    }

    ResolvedStreak {
        streak_days,
        base_streak,
    }
}

/// Moedas ganhas no check-in (0 quando `alreadyCollected`, salvo crédito do extrato).
#[must_use]
pub fn compute_checkin_coins_gained(checkin: Option<&CheckinInput>) -> i64 {
    let Some(checkin) = checkin else {
        return 0;
    };
    if checkin.checkin_coins_from_ledger == Some(true) {
        if let Some(from_ledger) = checkin
            .coins_gained_today
            .as_deref()
            .and_then(parse_digits_text)
            .filter(|value| *value > 0)
        {
            return from_ledger;
        }
    }
    if checkin.already_collected != Some(false) {
        return 0;
    }
    let raw_gained = checkin.coins_gained_today.as_deref();
    let has_explicit_gained = raw_gained.is_some_and(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && value != "N/D"
    });
    if has_explicit_gained {
        return raw_gained
            .and_then(parse_digits_text)
            .filter(|value| *value > 0)
            .unwrap_or(0);
    }
    if let Some(streak) = checkin.streak_days.as_ref() {
        if !matches!(streak, StreakValue::Text(text) if text == "N/D") {
            let from_streak = checkin_coins_from_streak(Some(streak));
            if from_streak > 0 {
                return from_streak;
            }
        }
    }
    0
}

/// Moedas ganhas pelas tarefas, isoladas do check-in.
#[must_use]
pub fn compute_tasks_coins_gained(
    tasks: Option<&TasksInput>,
    checkin: Option<&CheckinInput>,
) -> i64 {
    let Some(tasks) = tasks else {
        return 0;
    };
    let Some(raw_coins) = tasks.coins_gained.filter(|value| value.is_finite()) else {
        return 0;
    };
    #[allow(clippy::cast_possible_truncation)]
    let raw_coins = raw_coins as i64;
    if tasks.coins_from_ledger == Some(true) {
        return raw_coins;
    }
    let Some(checkin) = checkin else {
        return raw_coins;
    };
    let checkin_coins = compute_checkin_coins_gained(Some(checkin));
    if checkin_coins <= 0 {
        return raw_coins;
    }
    let init_balance = parse_num_or_text(tasks.initial_balance.as_ref());
    let checkin_balance = checkin.total_balance.as_deref().and_then(parse_digits_text);
    if let (Some(init), Some(checkin_bal)) = (init_balance, checkin_balance) {
        if init == checkin_bal - checkin_coins {
            return (raw_coins - checkin_coins).max(0);
        }
    }
    if checkin.balance_before_checkin == Some(true) {
        return (raw_coins - checkin_coins).max(0);
    }
    raw_coins
}

/// Saldo final consolidado (tarefas > check-in > `N/D`).
#[must_use]
pub fn compute_final_balance(checkin: Option<&CheckinInput>, tasks: Option<&TasksInput>) -> String {
    if let Some(final_coins) = tasks
        .and_then(|tasks| tasks.final_coins.as_deref())
        .filter(|value| !value.is_empty() && *value != "N/D")
    {
        return final_coins.to_string();
    }
    if let Some(total) = checkin
        .and_then(|checkin| checkin.total_balance.as_deref())
        .filter(|value| !value.is_empty() && *value != "N/D")
    {
        return format!("{total} moedas");
    }
    "N/D".to_string()
}

fn parse_streak(value: Option<&StreakValue>) -> Option<i64> {
    match value? {
        StreakValue::Number(number) => Some(*number),
        StreakValue::Text(text) => parse_digits_text(text),
    }
}

fn parse_num_or_text(value: Option<&NumOrText>) -> Option<i64> {
    match value? {
        NumOrText::Number(number) => {
            if *number == 0.0 || !number.is_finite() {
                return None;
            }
            #[allow(clippy::cast_possible_truncation)]
            Some(*number as i64)
        }
        NumOrText::Text(text) => {
            if text.is_empty() {
                return None;
            }
            parse_digits_text(text)
        }
    }
}

/// Extrai dígitos e converte (`parseInt(String(v).replace(/[^0-9]/g, ''), 10)`).
fn parse_digits_text(raw: &str) -> Option<i64> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_break() {
        assert!(is_streak_break(Some(1.0), Some(10.0), false));
        assert!(!is_streak_break(Some(7.0), Some(212.0), false));
        assert!(!is_streak_break(Some(1.0), Some(10.0), true));
        assert!(!is_streak_break(Some(1.0), Some(1.0), false));
        assert!(!is_streak_break(None, Some(10.0), false));
    }

    #[test]
    fn resolve_streak_incrementa_e_preserva() {
        let detected = StreakValue::Number(5);
        let result = resolve_streak_days(&ResolveStreakParams {
            detected_streak: Some(&detected),
            previous_streak_days: Some(5),
            just_collected: true,
            ..ResolveStreakParams::default()
        });
        assert_eq!(result.streak_days, StreakValue::Number(6));
        assert_eq!(result.base_streak, Some(5));

        let result = resolve_streak_days(&ResolveStreakParams {
            detected_streak: Some(&detected),
            previous_streak_days: Some(5),
            already_collected: true,
            ..ResolveStreakParams::default()
        });
        assert_eq!(result.streak_days, StreakValue::Number(5));
    }

    #[test]
    fn resolve_streak_ciclo_esporadico_e_extrato() {
        let detected = StreakValue::Number(7);
        let result = resolve_streak_days(&ResolveStreakParams {
            detected_streak: Some(&detected),
            previous_streak_days: Some(212),
            just_collected: true,
            ..ResolveStreakParams::default()
        });
        assert_eq!(result.streak_days, StreakValue::Number(213));

        let detected_one = StreakValue::Number(1);
        let statement = StreakValue::Number(50);
        let result = resolve_streak_days(&ResolveStreakParams {
            detected_streak: Some(&detected_one),
            previous_streak_days: Some(50),
            statement_streak: Some(&statement),
            ..ResolveStreakParams::default()
        });
        assert_eq!(result.streak_days, StreakValue::Number(50));
    }

    #[test]
    fn contabilidade_do_checkin() {
        let already = CheckinInput {
            already_collected: Some(true),
            coins_gained_today: Some("40".to_string()),
            ..CheckinInput::default()
        };
        assert_eq!(compute_checkin_coins_gained(Some(&already)), 0);

        let ledger = CheckinInput {
            already_collected: Some(true),
            coins_gained_today: Some("25".to_string()),
            checkin_coins_from_ledger: Some(true),
            ..CheckinInput::default()
        };
        assert_eq!(compute_checkin_coins_gained(Some(&ledger)), 25);

        let fresh = CheckinInput {
            already_collected: Some(false),
            coins_gained_today: Some("0".to_string()),
            ..CheckinInput::default()
        };
        assert_eq!(compute_checkin_coins_gained(Some(&fresh)), 0);

        let fallback = CheckinInput {
            already_collected: Some(false),
            streak_days: Some(StreakValue::Text("N/D".to_string())),
            ..CheckinInput::default()
        };
        // 'N/D' é explicitamente ignorado pelo oráculo no fallback por streak.
        assert_eq!(compute_checkin_coins_gained(Some(&fallback)), 0);

        let fallback_numeric = CheckinInput {
            already_collected: Some(false),
            streak_days: Some(StreakValue::Number(4)),
            ..CheckinInput::default()
        };
        assert_eq!(compute_checkin_coins_gained(Some(&fallback_numeric)), 25);
    }

    #[test]
    fn contabilidade_das_tarefas_isolada() {
        let checkin = CheckinInput {
            already_collected: Some(false),
            coins_gained_today: Some("20".to_string()),
            total_balance: Some("120".to_string()),
            ..CheckinInput::default()
        };
        let tasks = TasksInput {
            coins_gained: Some(30.0),
            initial_balance: Some(NumOrText::Number(100.0)),
            final_coins: Some("150".to_string()),
            ..TasksInput::default()
        };
        // init 100 == total 120 - checkin 20 => desconta o check-in.
        assert_eq!(compute_tasks_coins_gained(Some(&tasks), Some(&checkin)), 10);

        let ledger_tasks = TasksInput {
            coins_gained: Some(30.0),
            coins_from_ledger: Some(true),
            ..TasksInput::default()
        };
        assert_eq!(
            compute_tasks_coins_gained(Some(&ledger_tasks), Some(&checkin)),
            30
        );
        assert_eq!(compute_final_balance(Some(&checkin), Some(&tasks)), "150");
    }

    #[test]
    fn saldo_final_com_fallback() {
        let checkin = CheckinInput {
            total_balance: Some("99".to_string()),
            ..CheckinInput::default()
        };
        assert_eq!(compute_final_balance(Some(&checkin), None), "99 moedas");
        assert_eq!(
            compute_final_balance(
                None,
                Some(&TasksInput {
                    final_coins: Some("N/D".to_string()),
                    ..TasksInput::default()
                })
            ),
            "N/D"
        );
    }
}
