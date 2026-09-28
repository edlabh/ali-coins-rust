//! Paridade de tempo (`time_utils.js`) e SSRF (`url_guard.js`) com o oráculo.
//!
//! Fixtures: `tools/parity/fixtures/common/{time,url_guard}.json`
//! (gere com `./tools/parity/generate-fixtures.sh`).

use ali_coins_core::config::EnvSource;
use ali_coins_core::time::{
    calculate_account_backoff, compose_account_wait_ms, format_date, format_date_time,
    format_duration, format_time, get_report_timezone_label, pick_pause_ms,
};
use ali_coins_core::url_guard::{is_private_ip, validate_external_url};
use chrono::DateTime;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity/fixtures/common")
}

fn load(name: &str) -> Value {
    let path = fixtures_dir().join(name);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "fixture {} ausente ({err}); rode ./tools/parity/generate-fixtures.sh",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("fixture JSON")
}

#[test]
fn formatos_de_tempo_iguais_ao_oraculo() {
    let doc = load("time.json");
    if std::env::var("REPORT_TIMEZONE").is_ok() {
        eprintln!("pulando: REPORT_TIMEZONE definido no ambiente do teste");
        return;
    }

    for case in doc["cases"].as_array().expect("cases") {
        let iso = case["iso"].as_str().expect("iso");
        let dt = DateTime::parse_from_rfc3339(iso)
            .expect("ISO válido")
            .with_timezone(&chrono::Utc);
        assert_eq!(format_date(dt), case["date"].as_str().unwrap(), "{iso}");
        assert_eq!(format_time(dt), case["time"].as_str().unwrap(), "{iso}");
        assert_eq!(
            format_date_time(dt),
            case["dateTime"].as_str().unwrap(),
            "{iso}"
        );
        assert_eq!(
            get_report_timezone_label(dt),
            case["label"].as_str().unwrap(),
            "{iso}"
        );
    }

    let durations: Vec<i64> = vec![0, 45_000, 80_000, 3_665_000, -5, 59_999, 3_600_000];
    for (index, expected) in doc["durations"]
        .as_array()
        .expect("durations")
        .iter()
        .enumerate()
    {
        assert_eq!(
            format_duration(durations[index]),
            expected.as_str().unwrap(),
            "duration[{}]",
            durations[index]
        );
    }

    let env = EnvSource::default();
    for (index, attempt) in [0_i64, 1, 10].iter().enumerate() {
        assert_eq!(
            calculate_account_backoff(*attempt, None, 30_000, 0.5, &env),
            doc["backoff"][index].as_u64().unwrap(),
            "backoff attempt={attempt}"
        );
    }
    let env_base = EnvSource::from_pairs([("ACCOUNT_BACKOFF_BASE_MS", "1000")]);
    assert_eq!(
        calculate_account_backoff(0, None, 30_000, 0.5, &env_base),
        doc["backoffEnv"].as_u64().unwrap()
    );

    let pauses = [
        (0.0_f64, 0.0_f64, 0.0_f64),
        (1000.0, 2000.0, 0.0),
        (1000.0, 2000.0, 0.999),
        (5000.0, 1000.0, 0.0),
        (0.5, 3.5, 0.5),
    ];
    for (index, (min, max, random)) in pauses.iter().enumerate() {
        assert_eq!(
            pick_pause_ms(*min, *max, *random),
            doc["pauses"][index].as_u64().unwrap(),
            "pause {index}"
        );
    }

    assert_eq!(
        compose_account_wait_ms(5000.0, 1000.0),
        doc["compose"][0].as_u64().unwrap()
    );
    assert_eq!(
        compose_account_wait_ms(0.0, 3000.0),
        doc["compose"][1].as_u64().unwrap()
    );
    assert_eq!(
        compose_account_wait_ms(f64::NAN, -5.0),
        doc["compose"][2].as_u64().unwrap()
    );
}

#[test]
fn classificacao_de_ips_igual_ao_oraculo() {
    let doc = load("url_guard.json");
    for (ip, expected) in doc["isPrivateIp"].as_object().expect("isPrivateIp") {
        assert_eq!(
            is_private_ip(ip),
            expected.as_bool().unwrap(),
            "isPrivateIp({ip:?})"
        );
    }
}

#[test]
fn validacao_de_urls_igual_ao_oraculo() {
    let doc = load("url_guard.json");
    let env = EnvSource::default();
    for (raw, expected) in doc["validateExternalUrl"].as_object().expect("urls") {
        let result = validate_external_url(Some(raw), Some(false), false, &env);
        assert_eq!(result.ok, expected["ok"].as_bool().unwrap(), "ok({raw:?})");
        if !result.ok {
            assert_eq!(
                result.reason.as_deref(),
                expected["reason"].as_str(),
                "reason({raw:?})"
            );
        }
    }
}
