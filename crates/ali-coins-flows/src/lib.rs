//! Fluxos do site (check-in, tarefas) do `ali-coins-rust`.
//!
//! `balance` contém os parsers puros de saldo/streak/extrato; `checkin` e
//! `tasks` entram nos próximos incrementos da fase 3.

#![forbid(unsafe_code)]

pub mod balance;
pub mod checkin;
pub mod desktop;
pub mod login;
pub mod navigation;
pub mod tasks;
pub mod tasks_runner;

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
