//! Paridade das funções puras de relatório contra fixtures do oráculo.
//!
//! Fixture: `tools/parity/fixtures/common/report.json`
//! (gere com `./tools/parity/generate-fixtures.sh`).

use ali_coins_core::report::{
    CheckinInput, ResolveStreakParams, StreakValue, TasksInput, compute_checkin_coins_gained,
    compute_final_balance, compute_tasks_coins_gained, is_streak_break, resolve_streak_days,
};
use serde_json::Value;
use std::path::Path;

fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/parity/fixtures/common/report.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "fixture {} ausente ({err}); rode ./tools/parity/generate-fixtures.sh",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("fixture JSON")
}

fn as_streak(value: &Value) -> StreakValue {
    match value {
        Value::Number(number) => StreakValue::Number(number.as_i64().expect("inteiro")),
        Value::String(text) => StreakValue::Text(text.clone()),
        other => panic!("streak inválido: {other:?}"),
    }
}

fn as_optional_streak(value: &Value) -> Option<StreakValue> {
    if value.is_null() {
        None
    } else {
        Some(as_streak(value))
    }
}

#[test]
fn funcoes_puras_de_relatorio_iguais_ao_oraculo() {
    let doc = fixture();

    for case in doc["isStreakBreak"].as_array().expect("isStreakBreak") {
        let current = case["current"].as_f64();
        let previous = case["previous"].as_f64();
        let already = case["already"].as_bool().expect("already");
        assert_eq!(
            is_streak_break(current, previous, already),
            case["result"].as_bool().unwrap(),
            "isStreakBreak({current:?}, {previous:?}, {already})"
        );
    }

    for case in doc["resolveStreakDays"].as_array().expect("resolve") {
        let params = &case["params"];
        let detected = as_optional_streak(&params["detectedStreak"]);
        let statement = as_optional_streak(&params["statementStreak"]);
        let resolved = resolve_streak_days(&ResolveStreakParams {
            detected_streak: detected.as_ref(),
            previous_streak_days: params["previousStreakDays"].as_i64(),
            early_desktop_streak: params["earlyDesktopStreak"].as_i64(),
            just_collected: params["justCollected"].as_bool().unwrap_or(false),
            already_collected: params["alreadyCollected"].as_bool().unwrap_or(false),
            confirmed_by_ledger: params["confirmedByLedger"].as_bool().unwrap_or(false),
            statement_streak: statement.as_ref(),
        });
        let expected_streak = &case["streakDays"];
        match (&resolved.streak_days, expected_streak) {
            (StreakValue::Number(actual), Value::Number(expected)) => {
                assert_eq!(*actual, expected.as_i64().unwrap(), "params={params:?}");
            }
            (StreakValue::Text(actual), Value::String(expected)) => {
                assert_eq!(actual, expected, "params={params:?}");
            }
            other => panic!("tipo de streak divergente: {other:?}"),
        }
        assert_eq!(
            resolved.base_streak,
            case["baseStreak"].as_i64(),
            "baseStreak params={params:?}"
        );
    }

    for case in doc["computeCheckinCoinsGained"]
        .as_array()
        .expect("checkin")
    {
        let checkin: Option<CheckinInput> = serde_json::from_value(case["checkin"].clone()).ok();
        let expected = case["result"].as_i64().expect("result");
        assert_eq!(
            compute_checkin_coins_gained(checkin.as_ref()),
            expected,
            "checkin={:?}",
            case["checkin"]
        );
    }

    for case in doc["computeTasksCoinsGained"].as_array().expect("tasks") {
        let tasks: Option<TasksInput> = serde_json::from_value(case["tasks"].clone()).ok();
        let checkin: Option<CheckinInput> = serde_json::from_value(case["checkin"].clone()).ok();
        let expected = case["result"].as_i64().expect("result");
        assert_eq!(
            compute_tasks_coins_gained(tasks.as_ref(), checkin.as_ref()),
            expected,
            "tasks={:?} checkin={:?}",
            case["tasks"],
            case["checkin"]
        );
    }

    for case in doc["computeFinalBalance"].as_array().expect("final") {
        let checkin: Option<CheckinInput> = serde_json::from_value(case["checkin"].clone()).ok();
        let tasks: Option<TasksInput> = serde_json::from_value(case["tasks"].clone()).ok();
        assert_eq!(
            compute_final_balance(checkin.as_ref(), tasks.as_ref()),
            case["result"].as_str().expect("result"),
            "final checkin={:?} tasks={:?}",
            case["checkin"],
            case["tasks"]
        );
    }
}
