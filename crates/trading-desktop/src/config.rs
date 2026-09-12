//! Desktop app runtime config: bundles core Config + shared AppState.

use std::sync::Arc;

use trading_api::AppState;
use trading_core::Config;

#[derive(Clone)]
pub struct AppConfig {
    pub config: Arc<Config>,
    pub app: AppState,
}

impl AppConfig {
    pub fn new() -> Self {
        Self {
            config: Arc::new(Config::from_env()),
            app: AppState::new(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::new()
    }
}