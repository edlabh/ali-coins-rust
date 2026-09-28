//! Camada de browser do `ali-coins-rust`.
//!
//! `launch` contém as decisões puras de inicialização do Chromium (args,
//! cascata, sanitização de ambiente e perfil mobile). O trait `BrowserDriver`,
//! o `MockDriver` e o `CdpDriver` entram nas próximas entregas da fase 2.

#![forbid(unsafe_code)]

pub mod launch;

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
