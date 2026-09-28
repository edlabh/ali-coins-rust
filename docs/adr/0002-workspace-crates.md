# ADR-0002: Workspace Rust em 4 crates com tokio e erros tipados

**Status**: Accepted (2026-09-28)
**Data**: 2026-09-28

## Context

O projeto Node tem 15,9k linhas de código com dependências fortes entre config, cripto, browser e fluxos. A separação em crates precisa tornar o núcleo (config/cripto/sessão/lock/report/notify) testável sem browser, e isolar o driver CDP e as regras do site.

## Decision Drivers

- Testabilidade sem browser (maior parte do código de origem é lógica pura).
- Tempo de compilação e fronteiras claras entre domínios.
- Possibilidade de substituir o driver (ADR-0001) e de rodar o oráculo em testes.
- Builds de CI e Docker previsíveis.

## Considered Options

### Opção 1: crate único com módulos
- Prós: simples.
- Contras: qualquer teste de fluxo compila o driver CDP; fronteiras viram convenção.

### Opção 2: 4 crates — `core`, `browser`, `flows`, `cli` — **escolhida**
- Prós: `core` sem dependência de browser (testes rápidos); `browser` substituível; `flows` usa trait; `cli` fino; compilação incremental melhor.
- Contras: mais `Cargo.toml` e reexports; exige disciplina de dependências.

### Opção 3: micro-crates (8+)
- Prós: isolamento máximo.
- Contras: overhead de versão/CI sem ganho proporcional.

## Decision

Workspace com `ali-coins-core`, `ali-coins-browser`, `ali-coins-flows`, `ali-coins-cli` (binário `ali-coins`), `resolver = "3"`, MSRV **1.85**, edition **2024**, `tokio` (multi-thread) como runtime.

## Rationale

O caminho crítico do port (config/cripto/sessão/lock/report) fica compilável e testável sem Chromium — permite TDD desde a fase 1. O driver atrás de trait permite `MockDriver` para portar os testes de UI e `SidecarPlaywrightDriver` para paridade.

## Consequences

### Positive
- Testes do `core` rápidos e determinísticos (RNG/relógio injetáveis).
- Troca de driver sem tocar em fluxos; Docker/CI podem compilar `core` sozinho.
- `.cargo/config.toml` pode ajustar `-Z`-free flags de paralelismo e linker por alvo.

### Negative
- Um pouco mais de boilerplate de workspace e reexports do que um crate único.

### Risks
- `core` pode acabar importando tipos do browser por conveniência; mitigar com boundary test (`cargo tree -p ali-coins-core` sem `chromiumoxide`).

## Stack de crates proposta (núcleo)

| Área | Crates |
|---|---|
| Async/runtime | `tokio`, `tokio-util` (CancellationToken), `futures` |
| CLI | `clap` (derive; flags compatíveis com C-02) |
| Config/env | `dotenvy` + parser próprio de coerções (ADR-0003) |
| Cripto | `aes-gcm`, `scrypt`, `sha2`, `subtle`, `base64`, `rand` |
| Datas | `chrono`, `chrono-tz` |
| JSON/validação | `serde`, `serde_json` |
| HTTP | `reqwest` (rustls, sem OpenSSL), `hickory-resolver` se necessário para pinning |
| Logs | `tracing`, `tracing-subscriber` |
| Erros | `thiserror` (libs) + `anyhow` (bin) |
| SO/sinais | `nix` (unix), `windows-sys` (windows), `uuid`, `hostname`/`gethostname` |
| Browser | `chromiumoxide` (ADR-0001) |

## Implementation Notes

- `[workspace.lints]` com `clippy::all`/`pedantic` seletivo; `cargo fmt --check` e `clippy -D warnings` no CI desde o início.
- Perfis: `release` com `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"` (binário menor).
- Feature flags: `oracle-sidecar` (driver Node), `cdp-tracing`, `video-screencast`.

## Related Decisions

- ADR-0001 (driver), ADR-0003 (CLI/config), ADR-0005 (logs).
