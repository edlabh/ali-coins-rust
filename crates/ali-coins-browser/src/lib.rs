//! Camada de browser do `ali-coins-rust`.
//!
//! `driver` define a fronteira (traits) usada pelos fluxos; `launch` contém as
//! decisões puras de inicialização do Chromium; `mock` fornece um driver em
//! memória para testes. O `CdpDriver` (chromiumoxide) entra na próxima entrega.

#![forbid(unsafe_code)]

pub mod cdp;
pub mod driver;
pub mod launch;
pub mod mock;

/// Versão do crate, herdada do workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
