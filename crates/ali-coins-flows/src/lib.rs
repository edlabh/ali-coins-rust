//! Fluxos do site (check-in, tarefas) do `ali-coins-rust`.
//!
//! Módulos previstos: `selectors`, `login`, `balance`, `checkin`, `tasks/*`,
//! `ui/*`. Depende do browser apenas via traits (ADR-0001/0002).

#![forbid(unsafe_code)]

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
