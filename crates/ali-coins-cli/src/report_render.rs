//! Render textual dos relatórios (port de `libs/report.js` `render*`).
//!
//! Usado quando `--json` não é passado: reproduz o "RELATÓRIO CONSOLIDADO
//! FINAL" (all), o resumo do check-in e o resumo das tarefas.

use ali_coins_core::report::{
    CheckinInput, StreakValue, TasksInput, checkin_coins_from_streak, compute_checkin_coins_gained,
};
use ali_coins_core::{logging, time};
use chrono::{DateTime, Utc};
use serde_json::Value;

fn log(message: &str) {
    logging::global().info(message, &[]);
}

fn streak_display(value: &StreakValue) -> String {
    match value {
        StreakValue::Number(number) => number.to_string(),
        StreakValue::Text(text) => text.clone(),
    }
}

fn iso_parts(iso: &str) -> (String, String) {
    match DateTime::parse_from_rfc3339(iso) {
        Ok(parsed) => {
            let utc = parsed.with_timezone(&Utc);
            (time::format_date(utc), time::format_time(utc))
        }
        Err(_) => ("N/D".to_string(), "N/D".to_string()),
    }
}

/// `renderCheckinReport` (modo texto).
pub fn render_checkin(checkin: &CheckinInput) {
    let gained = compute_checkin_coins_gained(Some(checkin));
    let has_ledger_credit = checkin.checkin_coins_from_ledger == Some(true) && gained > 0;
    let line1 = if checkin.already_collected == Some(true) && !has_ledger_credit {
        "já estava coletado (+0 moedas)".to_string()
    } else {
        let value = if gained > 0 {
            gained.to_string()
        } else {
            checkin
                .coins_gained_today
                .clone()
                .unwrap_or_else(|| "0".to_string())
        };
        format!("{value} moedas")
    };
    let line2 = format!(
        "{} moedas",
        checkin
            .total_balance
            .clone()
            .unwrap_or_else(|| "N/D".to_string())
    );
    let line3 = match checkin.streak_days.as_ref() {
        Some(StreakValue::Text(text)) if text == "N/D" => {
            "sequência não identificada na página".to_string()
        }
        Some(streak) => format!(
            "a sequência subiu ({} dias seguidos)",
            streak_display(streak)
        ),
        None => "sequência não identificada na página".to_string(),
    };

    log("=== RELATORIO_OUTPUT ===");
    log(&line1);
    log(&line2);
    log(&line3);
    log("---------------------------------------------------------------");
    let (date, start_time) = checkin
        .start_time
        .as_deref()
        .map_or(("N/D".to_string(), "N/D".to_string()), iso_parts);
    let (_, end_time) = checkin
        .end_time
        .as_deref()
        .map_or(("N/D".to_string(), "N/D".to_string()), iso_parts);
    log(&format!("Data:                {date}"));
    log(&format!("Hora de Início:      {start_time}"));
    log(&format!("Hora de Finalização: {end_time}"));
    log(&format!(
        "Duração Total:       {}",
        checkin
            .duration
            .clone()
            .unwrap_or_else(|| "N/D".to_string())
    ));
    log("===============================================================\n");
}

/// `renderTasksReport` (modo texto).
pub fn render_tasks(tasks: &TasksInput) {
    log("\n================ RESUMO DAS TAREFAS ================");
    if let Some(results) = tasks.results.as_ref() {
        for item in results {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let coins = item.get("coins").and_then(Value::as_str).unwrap_or("");
            log(&format!("- {title}: {status} ({coins})"));
        }
    }
    log(&format!(
        "\nSaldo total final: {}",
        tasks
            .final_coins
            .clone()
            .unwrap_or_else(|| "N/D".to_string())
    ));
    log("----------------------------------------------------");
    let (date, start_time) = tasks
        .start_time
        .as_deref()
        .map_or(("N/D".to_string(), "N/D".to_string()), iso_parts);
    let (_, end_time) = tasks
        .end_time
        .as_deref()
        .map_or(("N/D".to_string(), "N/D".to_string()), iso_parts);
    log(&format!("Data:                {date}"));
    log(&format!("Hora de Início:      {start_time}"));
    log(&format!("Hora de Finalização: {end_time}"));
    log(&format!(
        "Duração Total:       {}",
        tasks.duration.clone().unwrap_or_else(|| "N/D".to_string())
    ));
    log("====================================================\n");
}

/// `renderUnifiedReport` (modo texto) a partir do payload e das entradas.
pub fn render_unified(payload: &Value, checkin: Option<&CheckinInput>, masked_user: &str) {
    let meta = payload.get("meta").cloned().unwrap_or_default();
    let checkin_coins = meta
        .get("checkinCoinsGained")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let tasks_coins = meta
        .get("tasksCoinsGained")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let total_coins = meta
        .get("totalCoinsGained")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let final_balance = meta
        .get("finalBalance")
        .and_then(Value::as_str)
        .unwrap_or("N/D");

    log("\n===============================================================");
    log("                RELATÓRIO CONSOLIDADO FINAL");
    log("===============================================================");

    if let Some(checkin) = checkin {
        let streak = checkin
            .streak_days
            .as_ref()
            .map_or_else(|| "N/D".to_string(), streak_display);
        let daily_tier = match checkin.streak_days.as_ref() {
            Some(StreakValue::Text(text)) if text == "N/D" => checkin
                .coins_gained_today
                .clone()
                .filter(|value| value != "0")
                .unwrap_or_else(|| "N/D".to_string()),
            Some(streak) => checkin_coins_from_streak(Some(streak)).to_string(),
            None => "N/D".to_string(),
        };
        log(&format!("Conta: {masked_user}"));
        log(&format!(
            "Sequência (Streak): {streak} dias seguidos (+{daily_tier} moedas/dia)"
        ));
        let has_ledger_credit =
            checkin.checkin_coins_from_ledger == Some(true) && checkin_coins > 0;
        let status = if checkin.already_collected == Some(true) && !has_ledger_credit {
            "Já coletado hoje (+0 moedas)".to_string()
        } else if checkin.already_collected != Some(false)
            && !has_ledger_credit
            && checkin_coins <= 0
        {
            "não confirmado nesta execução (nova tentativa na etapa de tarefas)".to_string()
        } else {
            format!("Coletado com sucesso (+{checkin_coins} moedas)")
        };
        log(&format!("Check-in Diário: {status}"));
    }

    if let Some(results) = payload
        .get("tasks")
        .and_then(|tasks| tasks.get("results"))
        .and_then(Value::as_array)
    {
        log("\nTarefas do Painel \"Ganhe mais moedas\":");
        for item in results {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let coins = item.get("coins").and_then(Value::as_str).unwrap_or("");
            log(&format!("  • {title}: {status} ({coins})"));
        }
        log(&format!("Ganho Real pelas Tarefas: +{tasks_coins} moedas"));
    }

    log("---------------------------------------------------------------");
    log(&format!(
        "Moedas Ganhas Hoje:     +{total_coins} moedas (check-in +{checkin_coins} / tarefas +{tasks_coins})"
    ));
    log(&format!("Saldo Total Atualizado: {final_balance}"));
    log("---------------------------------------------------------------");

    let start = meta.get("startTime").and_then(Value::as_str);
    let end = meta.get("endTime").and_then(Value::as_str);
    if let (Some(start), Some(end)) = (start, end) {
        let (date, start_time) = iso_parts(start);
        let (_, end_time) = iso_parts(end);
        log(&format!("Data:                {date}"));
        log(&format!("Hora de Início:      {start_time}"));
        log(&format!("Hora de Finalização: {end_time}"));
    }
    if let Some(step1) = meta.get("step1Duration").and_then(Value::as_str) {
        log(&format!("Duração Etapa 1:     {step1}"));
    }
    if let Some(step2) = meta.get("step2Duration").and_then(Value::as_str) {
        log(&format!("Duração Etapa 2:     {step2}"));
    }
    if let Some(total) = meta.get("totalDuration").and_then(Value::as_str) {
        log(&format!("Duração Total:       {total}"));
    }
    log("===============================================================\n");
}
