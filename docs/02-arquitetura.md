# Etapa 2 — Arquitetura proposta para o port Rust

Complementa `docs/01-avaliacao.md`. As decisões estão registradas em `docs/adr/`.

## Princípios

1. **Paridade primeiro**: nenhuma mudança de comportamento/contrato sem teste que a comprove (Node = oráculo até o corte).
2. **Núcleo puro e testável**: config, cripto, sessão, lock, report, timing e SSRF não dependem de browser.
3. **Porta/adaptador no browser**: os fluxos dependem de um trait, não do CDP.
4. **Sem estado global mutável**: `Config` e dependências passadas explicitamente; timezone é campo, não singleton.
5. **Erros tipados → exit codes** num único mapeamento explícito.
6. **Saída determinística**: JSON canônico no stdout (`--json`), logs em stderr.

## Workspace proposto

```
ali-coins-rust/
├── Cargo.toml                  # [workspace] members + lints/perfis
├── rust-toolchain.toml         # 1.85+ (edge edition 2024)
├── crates/
│   ├── ali-coins-core/         # lib sem browser
│   │   ├── config/             # env, credentials.env, accounts.json, paths
│   │   ├── crypto/             # token v1/v2/v3, scrypt, AES-GCM, rotação
│   │   ├── session/            # persistência, meta, clear/prune, storage filter
│   │   ├── lock/               # lockfile hostname+PID, stale, refresh
│   │   ├── report/             # schemas serde, contabilidade, texto/JSON
│   │   ├── notify/             # Telegram, webhook, heartbeat, url_guard
│   │   ├── logging/            # tracing + redaction + formatos
│   │   ├── time/               # LA tz, streak, formatos, delays/backoff
│   │   └── exit/               # ExitCode, shutdown/flush, crash hook
│   ├── ali-coins-browser/      # lib: driver trait + implementação CDP
│   │   ├── driver.rs           # trait BrowserDriver / Page / Element
│   │   ├── cdp/                # chromiumoxide: launch, emulação, rede, trace
│   │   ├── mock/               # driver para testes (fixtures/DOM)
│   │   └── diagnostics/        # screenshots, trace, dom hash, scratch/
│   ├── ali-coins-flows/        # lib: regras de negócio do site
│   │   ├── selectors.rs        # dados (port 1:1)
│   │   ├── login.rs            # SPA, slider, 2FA fail-fast
│   │   ├── balance.rs          # saldo/streak/ledger
│   │   ├── checkin.rs          # collect.js
│   │   ├── tasks/              # dispatcher, state, verifier, surprise, search, prizeland
│   │   └── ui/                 # navigation helpers, closeModals, scroll
│   └── ali-coins-cli/          # bin único `ali-coins` + subcomandos
│       └── src/main.rs         # all | checkin | tasks | export-session | import-session
├── wrappers/                   # run*.sh/.bat/.ps1 adaptados (mesma UX)
├── docker/                     # Dockerfile multi-stage + compose
├── .github/workflows/          # CI, release, dependabot (cargo)
├── tests/                      # testes de contrato/oráculo (node+rust)
└── reference/ali-coins/        # clone Node (oráculo; gitignored)
```

**Subcomandos do binário** (substituem os entrypoints JS, mantendo todas as flags):

| Node | Rust |
|---|---|
| `node all.js` | `ali-coins` (padrão) ou `ali-coins all` |
| `node collect.js` | `ali-coins checkin` |
| `node do_tasks.js` | `ali-coins tasks` |
| `node export_session.js` | `ali-coins export-session` |
| `node import_session.js` | `ali-coins import-session` |

## Fronteira do browser (porta/adaptador)

```rust
#[async_trait]
pub trait BrowserDriver: Send + Sync {
    async fn launch(&self, opts: LaunchOptions) -> Result<Box<dyn Browser>>;
}

#[async_trait]
pub trait Browser: Send + Sync {
    async fn new_context(&self, opts: ContextOptions) -> Result<Box<dyn BrowserContext>>;
    async fn close(self: Box<Self>) -> Result<()>;
}

#[async_trait]
pub trait BrowserContext: Send + Sync {
    async fn new_page(&self) -> Result<Box<dyn Page>>;
    async fn pages(&self) -> Result<Vec<Box<dyn Page>>>;
    async fn wait_for_new_page(&self, timeout: Duration) -> Result<Box<dyn Page>>;
    async fn storage_state(&self) -> Result<StorageState>;
    async fn cookies(&self) -> Result<Vec<Cookie>>;
    async fn close(self: Box<Self>) -> Result<()>;
}

#[async_trait]
pub trait Page: Send + Sync {
    async fn goto(&self, url: &str, opts: NavOptions) -> Result<()>;
    async fn go_back(&self) -> Result<()>;
    async fn url(&self) -> Result<String>;
    async fn content(&self) -> Result<String>;
    async fn wait_for_selector(&self, sel: &str, opts: WaitOptions) -> Result<Box<dyn Element>>;
    async fn query_all(&self, sel: &str) -> Result<Vec<Box<dyn Element>>>;
    async fn eval<T: DeserializeOwned>(&self, script: &str) -> Result<T>;
    async fn eval_on(&self, el: &dyn Element, script: &str) -> Result<Value>;
    async fn click(&self, el: &dyn Element) -> Result<()>;
    async fn scroll_by(&self, x: i64, y: i64) -> Result<()>;
    async fn screenshot(&self, opts: ScreenshotOptions) -> Result<Vec<u8>>;
    async fn set_input_files(&self, el: &dyn Element, paths: &[PathBuf]) -> Result<()>;
    async fn on_dialog(&self, handler: DialogHandler) -> Result<()>;
    async fn close(&self) -> Result<()>;
}
```

- **Impl. de produção**: `CdpDriver` sobre `chromiumoxide` (CDP) — cobre launch/args, emulação (`Emulation.setUserAgentOverride`, `setDeviceMetricsOverride`, `setTouchEmulationEnabled`), rede (`Fetch`/`Network`), `Runtime.evaluate`, `Page.captureScreenshot`, `Tracing`.
- **Impl. de teste**: `MockDriver` alimentado pelos fixtures já existentes em `tests/` (HTML da gaveta, respostas de saldo) — permite portar a maior parte dos testes sem browser.
- **Transição**: `SidecarPlaywrightDriver` (opcional, atrás de feature flag) para rodar o Node como oráculo e comparar resultado durante as fases 3–4.

## Modelo de execução

```
main() → parse CLI → Config::load(dry_run) → Accounts::load
        → CancellationToken (SIGINT/SIGTERM)
        → StartDelay (abortável)
        → Lock::acquire
        → Heartbeat::start
        → for account in accounts { Session::load → Checkin → Tasks → Report → Notify }
        → Heartbeat::success/fail → Shutdown::flush → ExitCode
```

- **Async**: `tokio` multi-thread; contas sequenciais (paridade); `tokio::time::timeout` substitui `withTimeout`; `CancellationToken` substitui handlers de sinal dispersos.
- **RNG/relógio**: injetados via `Clock`/`Rng` (seed em teste) — espelha os mocks do Node.
- **Exit codes**: `enum ExitCode { Success=0, Failure=1, NoAction=2, LockActive=3, StreakBroken=4, TwoFactor=5, Crash=6 }`; sinais → 130/143; mapeamento único e testado.

## Rastreabilidade Node → Rust (resumo)

| Node | Rust |
|---|---|
| `config.js` | `core::config` + `cli` (flags) |
| `security.js` | `core::crypto` |
| `libs/session.js`, `export_session.js`, `import_session.js` | `core::session` + subcomandos CLI |
| `lockfile.js` | `core::lock` |
| `logger.js`, `libs/exit.js`, `libs/crash.js` | `core::logging`, `core::exit` |
| `time_utils.js`, `libs/timing.js` | `core::time` |
| `libs/report.js` | `core::report` |
| `libs/notify.js`, `libs/heartbeat.js`, `libs/webhooks.js` | `core::notify` |
| `libs/url_guard.js` | `core::notify::ssrf` |
| `libs/storage_filter.js` | `core::session::storage_filter` |
| `browser.js` | `browser::cdp` |
| `libs/selectors.js` | `flows::selectors` |
| `libs/ui/balance.js`, `login.js`, `navigation.js`, `diagnostics.js` | `flows::balance`, `flows::login`, `flows::ui`, `browser::diagnostics` |
| `collect.js` | `flows::checkin` |
| `do_tasks.js`, `libs/tasks/*` | `flows::tasks::*` |
| `all.js` | `cli::all` |
| `Dockerfile`, `docker-compose.yml`, `run*`, `setup_*`, CI | `docker/`, `wrappers/`, `.github/workflows/` |
