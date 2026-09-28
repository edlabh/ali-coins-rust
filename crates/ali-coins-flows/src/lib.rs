//! Fluxos do site (check-in, tarefas) do `ali-coins-rust`.
//!
//! `balance` contém os parsers puros de saldo/streak/extrato; `checkin` e
//! `tasks` entram nos próximos incrementos da fase 3.

#![forbid(unsafe_code)]

pub mod balance;
pub mod login;
pub mod navigation;

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
