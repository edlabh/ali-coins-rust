# Matriz de rastreabilidade — Node → Rust

Legenda: **Fase** do roadmap · **Contratos** conforme `docs/01-avaliacao.md` §7 · Tipo de teste: `U` unitário, `C` contrato Node×Rust, `G` golden/fixture, `S` smoke real.

| Node (referência) | Rust (destino) | Fase | Contratos | Teste |
|---|---|---|---|---|
| `config.js` (schema/credenciais) | `core::config` | 1 | C-04, C-20 | U, C, G |
| `config.js` (CLI commander) | `cli::args` (clap) | 1 | C-02, C-03 | U, C |
| `config.js` (loadAccounts/paths/hash) | `core::config::accounts` | 1 | C-07 | U, G |
| `security.js` (token v1/v2/v3) | `core::crypto::token` | 1 | C-05 | U, C (interop) |
| `security.js` (safe write 0600) | `core::fs::safe_write` | 1 | C-06, C-20 | U |
| `libs/session.js` | `core::session` | 1 | C-05…C-07 | U, G |
| `libs/storage_filter.js` | `core::session::storage_filter` | 1 | C-06 | U |
| `lockfile.js` | `core::lock` | 1 | C-08 | U, C |
| `logger.js` | `core::logging` | 1 | C-03, C-16 | U |
| `libs/exit.js`, `libs/crash.js` | `core::exit`, panic hook | 1 | C-01 | U, C |
| `time_utils.js`, `libs/timing.js` | `core::time` | 1 | C-11 | U, G |
| `libs/report.js` | `core::report` | 1 | C-09, C-10 | U, G |
| `libs/notify.js` | `core::notify::telegram` | 1 | C-12 | U, G |
| `libs/heartbeat.js` | `core::notify::heartbeat` | 1 | C-13 | U |
| `libs/webhooks.js` + cliente em `report.js` | `core::notify::{webhook,tracker}` | 1 | C-12, C-14 | U |
| `libs/url_guard.js` | `core::notify::ssrf` | 1 | C-14 | U, G |
| `export_session.js` | `cli::export_session` | 1 | C-02, C-05 | U, C |
| `import_session.js` | `cli::import_session` | 1 | C-02, C-05 | U, C |
| `browser.js` (launch/args/low-memory) | `browser::cdp::launch` | 2 | C-18 | U, S |
| `browser.js` (emulação mobile) | `browser::cdp::emulation` | 2 | C-17 | S |
| `browser.js` (bloqueio de recursos) | `browser::cdp::network` | 2 | C-18 | U, S |
| `browser.js` (retry/scroll) | `browser::cdp::helpers` | 2 | — | U |
| `libs/ui/diagnostics.js` | `browser::diagnostics` | 2 | C-19 | U, S |
| `libs/selectors.js` | `flows::selectors` | 2 | — | U |
| `libs/ui/navigation.js` | `flows::ui::navigation` | 3 | — | U, S |
| `libs/ui/login.js` | `flows::login` | 3 | C-16 | U, S |
| `libs/ui/balance.js` | `flows::balance` | 3 | C-10, C-11 | U, G |
| `collect.js` | `flows::checkin` | 3 | C-01, C-10 | U, G, S |
| `libs/tasks/state.js` | `flows::tasks::state` | 4 | C-15 | U, G |
| `libs/tasks/verifier.js` | `flows::tasks::verifier` | 4 | — | U, G |
| `libs/tasks/surprise.js` | `flows::tasks::surprise` | 4 | — | U, G |
| `libs/tasks/search.js` | `flows::tasks::search` | 4 | — | U |
| `libs/tasks/prizeland.js` | `flows::tasks::prizeland` | 4 | — | U |
| `libs/tasks/dispatcher.js` | `flows::tasks::dispatcher` | 4 | C-15 | U |
| `do_tasks.js` | `flows::tasks::runner` | 4 | C-01, C-10 | U, G, S |
| `all.js` | `cli::all` | 5 | C-01, C-09 | U, C, G |
| `Dockerfile`, `docker-compose.yml`, `docker-run.example.sh` | `docker/` | 5 | C-01, C-19 | S |
| `run*.sh|bat|ps1`, `setup_*`, `generate_secret.*` | `wrappers/`, `tools/setup/` | 5 | C-01, C-20 | U |
| `.github/workflows/*`, `release.yml`, `dependabot.yml` | `.github/workflows/*` | 5 | — | C |
| `README.md`, `INSTALL_*`, `CLOUD_SESSIONS.md`, `TELEGRAM.md` | docs do port | 6 | — | Revisão |
