//! Paridade dos parsers de saldo/streak contra fixtures do oráculo.
//!
//! Fixture: `tools/parity/fixtures/common/balance.json`

use ali_coins_core::report::StreakValue;
use ali_coins_flows::balance::{
    extract_streak_from_text, extract_today_ledger, is_login_prompt_text, streak_from_checkin_coins,
};
use serde_json::Value;
use std::path::Path;

fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/parity/fixtures/common/balance.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "fixture {} ausente ({err}); rode ./tools/parity/generate-fixtures.sh",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("fixture JSON")
}

#[test]
fn parsers_de_saldo_iguais_ao_oraculo() {
    let doc = fixture();

    for (text, expected) in doc["extractStreakFromText"].as_object().expect("streak") {
        assert_eq!(
            extract_streak_from_text(text),
            expected.as_i64(),
            "extractStreakFromText({text:?})"
        );
    }

    for (text, expected) in doc["isLoginPromptText"].as_object().expect("login") {
        assert_eq!(
            is_login_prompt_text(text),
            expected.as_bool().unwrap(),
            "isLoginPromptText({text:?})"
        );
    }

    for case in doc["getStreakFromCheckinCoins"]
        .as_array()
        .expect("coins->streak")
    {
        assert_eq!(
            streak_from_checkin_coins(case["coins"].as_str()),
            case["result"].as_i64(),
            "getStreakFromCheckinCoins({:?})",
            case["coins"]
        );
    }

    for case in doc["getCheckinCoinsFromStreak"]
        .as_array()
        .expect("streak->coins")
    {
        let streak_value = match &case["streak"] {
            Value::Null => None,
            Value::Number(number) => Some(StreakValue::Number(number.as_i64().unwrap())),
            Value::String(text) => Some(StreakValue::Text(text.clone())),
            other => panic!("streak inválido: {other:?}"),
        };
        assert_eq!(
            ali_coins_flows::balance::checkin_coins_from_streak(streak_value.as_ref()),
            case["result"].as_i64().expect("result"),
            "getCheckinCoinsFromStreak({:?})",
            case["streak"]
        );
    }

    for (text, expected) in doc["extractTodayLedger"].as_object().expect("ledger") {
        let extract = extract_today_ledger(text);
        assert_eq!(
            extract.bonus_coins,
            expected["bonusCoins"].as_i64(),
            "bonusCoins({text:?})"
        );
        assert_eq!(
            extract.missions_coins,
            expected["missionsCoins"].as_i64().unwrap(),
            "missionsCoins({text:?})"
        );
        assert_eq!(
            extract.bonus_count,
            u32::try_from(expected["bonusCount"].as_u64().unwrap()).unwrap(),
            "bonusCount({text:?})"
        );
        assert_eq!(
            extract.missions_count,
            u32::try_from(expected["missionsCount"].as_u64().unwrap()).unwrap(),
            "missionsCount({text:?})"
        );
    }
}
