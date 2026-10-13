//! Trading Engine — core execution, market polling, order management,
//! auto-compounding, and live risk enforcement.

pub mod config;
pub mod engine;

pub use config::AppConfig;
pub use engine::Engine;
