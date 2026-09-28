//! Núcleo do `ali-coins-rust` — sem dependência de browser.
//!
//! Módulos previstos (fase 1 do roadmap): `config`, `crypto`, `session`,
//! `lock`, `report`, `notify`, `logging`, `time`, `exit`.
//! Este crate deve permanecer compilável e testável sem Chromium (ADR-0002).

#![forbid(unsafe_code)]

pub mod config;
pub mod crypto;
pub mod secure_fs;
pub mod session;

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
