//! Paridade do `--dry-run --json` contra fixtures geradas pelo oráculo Node.
//!
//! Cenários: `tools/parity/scenarios/*.env`
//! Fixtures: `tools/parity/fixtures/config/<cenário>.json|.exit`
//! (gere com `./tools/parity/generate-fixtures.sh`).

use ali_coins_core::config::{Config, DryRunSummary, EnvSource, load_accounts};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn parity_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse_env_file(path: &Path) -> EnvSource {
    let raw = std::fs::read_to_string(path).expect("ler cenário");
    let pairs = raw.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((
            key.trim().to_string(),
            value.trim().trim_matches('"').to_string(),
        ))
    });
    EnvSource::from_pairs(pairs)
}

#[test]
fn dry_run_json_igual_as_fixtures_do_oraculo() {
    let root = parity_root();
    let scenarios_dir = root.join("tools/parity/scenarios");
    let fixtures_dir = root.join("tools/parity/fixtures/config");

    let mut checked = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&scenarios_dir)
        .expect("scenarios dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "env"))
        .collect();
    entries.sort();

    for scenario in entries {
        let name = scenario
            .file_stem()
            .expect("nome do cenário")
            .to_string_lossy()
            .to_string();
        let env = parse_env_file(&scenario);
        let result = Config::load(&env, Path::new("/tmp/projeto"), true, None, None);

        let json_fixture = fixtures_dir.join(format!("{name}.json"));
        let exit_fixture = fixtures_dir.join(format!("{name}.exit"));

        if json_fixture.exists() {
            let config = result.unwrap_or_else(|err| {
                panic!("{name}: Rust rejeitou config válida no oráculo: {err:?}")
            });
            let accounts = load_accounts(&env, Path::new("/tmp/projeto"));
            let actual: Value =
                serde_json::from_str(&DryRunSummary::build(&config, &accounts).to_pretty_json())
                    .expect("JSON do resumo");
            let expected: Value = serde_json::from_str(
                &std::fs::read_to_string(&json_fixture).expect("fixture JSON"),
            )
            .expect("fixture JSON válida");
            assert_eq!(actual, expected, "divergência no cenário {name}");
        } else if exit_fixture.exists() {
            let expected_exit: i32 = std::fs::read_to_string(&exit_fixture)
                .expect("fixture exit")
                .trim()
                .parse()
                .expect("exit numérico");
            assert_ne!(expected_exit, 0, "cenário {name} deveria falhar");
            assert!(
                result.is_err(),
                "cenário {name}: Rust aceitou config rejeitada pelo oráculo"
            );
        } else {
            panic!("cenário {name} sem fixture; rode ./tools/parity/generate-fixtures.sh");
        }
        checked += 1;
    }

    assert!(
        checked >= 5,
        "esperava ao menos 5 cenários, encontrou {checked}"
    );
}
