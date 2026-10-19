//! Configuration from environment variables (.env)

use std::env;
use std::path::PathBuf;

/// Runtime configuration loaded from environment / .env file.
///
/// Mirrors the original Python config but Rust-native.
#[derive(Debug, Clone)]
pub struct Config {
    // OANDA (Forex / Gold) — optional trading account for live orders
    pub oanda_token: String,
    pub oanda_account_id: String,
    pub oanda_practice: bool,

    // Data feed: "yahoo" (free, no key), "oanda" (demo/live account) or "deriv" (real demo orders)
    pub fx_broker: String,

    // Deriv (Options demo) — real multiplier orders (MULTUP/MULTDOWN) + SL/TP
    pub deriv_token: String,
    pub deriv_app_id: String,
    pub deriv_account_id: String,

    // AI / Ollama
    pub ollama_base_url: String,
    pub ollama_model: String,

    // ── The ReAct agent gate ───────────────────────────────────────────────
    // The bot asks this service for a verdict before every trade. The gate
    // fails CLOSED: if the agent cannot be reached, no trade happens.
    pub agent_api_url: String,
    pub agent_timeout_secs: u64,
    pub agent_min_confidence: f64,
    pub agent_required: bool,
    pub agent_api_secret: String,

    // Trading
    pub default_asset: String,
    pub trading_mode: String,
    pub forex_enabled: bool,
    pub gold_enabled: bool,
    pub synthetic_enabled: bool,
    pub default_engine: String,
    pub contract_mode: String,
    pub rise_fall_stake: f64,
    pub default_tick_duration: u32,
    pub interval_seconds: u64,
    pub default_timeframe: String,

    // Asset universe
    pub forex_assets: Vec<String>,
    pub gold_assets: Vec<String>,
    pub synthetic_assets: Vec<String>,

    // Risk
    pub max_daily_loss: f64,
    pub max_consecutive_losses: i32,
    pub max_open_positions: i32,
    pub risk_per_trade_pct: f64,

    // Paths
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub log_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = PathBuf::from(dotenv_var("DATA_DIR", "data"));
        let _ = std::fs::create_dir_all(&data_dir);

        Self {
            oanda_token: dotenv_var("OANDA_TOKEN", ""),
            oanda_account_id: dotenv_var("OANDA_ACCOUNT_ID", ""),
            oanda_practice: dotenv_bool("OANDA_PRACTICE", true),
            fx_broker: dotenv_var("FX_BROKER", "deriv"),

            deriv_token: dotenv_var("DERIV_TOKEN", ""),
            deriv_app_id: dotenv_var("DERIV_APP_ID", ""),
            deriv_account_id: dotenv_var("DERIV_ACCOUNT_ID", ""),

            ollama_base_url: dotenv_var("OLLAMA_BASE_URL", "http://localhost:11434"),
            ollama_model: dotenv_var("OLLAMA_MODEL", "llama3.2:3b"),

            // The ReAct agent gate (the Next.js terminal).
            agent_api_url: dotenv_var("AGENT_API_URL", "http://127.0.0.1:3000"),
            agent_timeout_secs: dotenv_var("AGENT_TIMEOUT_SECS", "45")
                .parse()
                .unwrap_or(45),
            agent_min_confidence: dotenv_var("AGENT_MIN_CONFIDENCE", "0.6")
                .parse()
                .unwrap_or(0.6),
            // The default is the safe one: without the agent, no trades.
            agent_required: dotenv_bool("AGENT_REQUIRED", true),
            agent_api_secret: dotenv_var("AGENT_API_SECRET", ""),

            default_asset: dotenv_var("DEFAULT_ASSET", "XAU_USD"),
            trading_mode: dotenv_var("TRADING_MODE", "paper"),
            forex_enabled: dotenv_bool("FOREX_ENABLED", true),
            gold_enabled: dotenv_bool("GOLD_ENABLED", true),
            synthetic_enabled: dotenv_bool("SYNTHETIC_ENABLED", false),
            default_engine: dotenv_var("DEFAULT_ENGINE", "gold"),
            // Multiplier only. Rise/Fall (binary) is deliberately disabled:
            // it has no real stop-loss, cannot be risk-managed, and its contracts
            // never resolve through the engine's SL/TP path.
            contract_mode: dotenv_var("CONTRACT_MODE", "multiplier"),
            rise_fall_stake: 0.0,
            default_tick_duration: 5,
            interval_seconds: dotenv_var("INTERVAL_SECONDS", "1").parse().unwrap_or(1),
            default_timeframe: dotenv_var("DEFAULT_TIMEFRAME", "5m"),

            forex_assets: dotenv_list("FOREX_ASSETS", &["EUR_USD", "GBP_USD", "USD_JPY"]),
            gold_assets: dotenv_list("GOLD_ASSETS", &["XAU_USD"]),
            // Synthetic indices are Rise/Fall-only on Deriv; empty by default.
            synthetic_assets: dotenv_list("SYNTHETIC_ASSETS", &[]),

            max_daily_loss: dotenv_var("MAX_DAILY_LOSS", "5.0").parse().unwrap_or(5.0),
            max_consecutive_losses: dotenv_var("MAX_CONSECUTIVE_LOSSES", "5").parse().unwrap_or(5),
            max_open_positions: dotenv_var("MAX_OPEN_POSITIONS", "5").parse().unwrap_or(5),
            risk_per_trade_pct: dotenv_var("RISK_PER_TRADE_PCT", "0.5").parse().unwrap_or(0.5),

            db_path: data_dir.join("trading.db"),
            log_dir: PathBuf::from(dotenv_var("LOG_DIR", "logs")),
            data_dir,
        }
    }

    /// True when real-money mode is NOT active.
    ///
    /// Safe by design: live trading is opt-in and must be spelled exactly
    /// `live`. Anything else (including a typo, or a blank value) stays paper.
    /// The old version fell through to `oanda_practice` for unknown modes,
    /// which meant a typo in TRADING_MODE could silently arm real orders.
    pub fn is_paper(&self) -> bool {
        self.trading_mode.trim().to_lowercase() != "live"
    }

    pub fn is_live(&self) -> bool {
        !self.is_paper()
    }
}

fn dotenv_var(key: &str, default: &str) -> String {
    load_dotenv();
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn dotenv_bool(key: &str, default: bool) -> bool {
    load_dotenv();
    match env::var(key) {
        Ok(v) => matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

fn dotenv_list(key: &str, defaults: &[&str]) -> Vec<String> {
    load_dotenv();
    match env::var(key) {
        Ok(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => defaults.iter().map(|s| s.to_string()).collect(),
    }
}

fn load_dotenv() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let mut cur = std::env::current_dir().ok();
        while let Some(dir) = cur {
            let env_path = dir.join(".env");
            if env_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&env_path) {
                    for line in content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                            continue;
                        }
                        if let Some((k, v)) = line.split_once('=') {
                            let k = k.trim().to_string();
                            let v = v.trim().trim_matches('"').to_string();
                            if !k.is_empty() && env::var(&k).is_err() {
                                env::set_var(k, v);
                            }
                        }
                    }
                }
                return;
            }
            cur = dir.parent().map(|p| p.to_path_buf());
        }
    });
}