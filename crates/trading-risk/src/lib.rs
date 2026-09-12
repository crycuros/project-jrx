//! Risk management: position sizing, exposure limits, kill switch.

use trading_core::{Config, OrderSide, Position, Trade};

#[derive(Debug, Clone)]
pub struct RiskDecision {
    pub allowed: bool,
    pub reason: String,
    pub suggested_size: f64,
    pub stop_price: f64,
}

/// Central risk manager mirroring the Python `RiskManager`.
pub struct RiskManager {
    pub max_daily_loss: f64,
    pub max_consecutive_losses: i32,
    pub max_open_positions: i32,
    pub risk_per_trade_pct: f64,
    pub equity: f64,
    pub stop_percent: f64,
}

impl RiskManager {
    pub fn new(cfg: &Config) -> Self {
        Self {
            max_daily_loss: cfg.max_daily_loss,
            max_consecutive_losses: cfg.max_consecutive_losses,
            max_open_positions: cfg.max_open_positions,
            risk_per_trade_pct: cfg.risk_per_trade_pct,
            equity: 10_000.0,
            stop_percent: 2.0,
        }
    }

    /// Validate a proposed trade against all guards.
    pub fn validate(
        &self,
        symbol: &str,
        side: OrderSide,
        price: f64,
        open: &[Trade],
        consecutive_losses: i32,
        daily_pnl: f64,
        total_pnl: f64,
    ) -> RiskDecision {
        let mut reasons = Vec::new();

        if open.len() as i32 >= self.max_open_positions {
            return RiskDecision {
                allowed: false,
                reason: format!("max open positions ({}) reached", self.max_open_positions),
                suggested_size: 0.0,
                stop_price: 0.0,
            };
        }

        if consecutive_losses >= self.max_consecutive_losses {
            return RiskDecision {
                allowed: false,
                reason: "max consecutive losses hit — cooldown".to_string(),
                suggested_size: 0.0,
                stop_price: 0.0,
            };
        }

        if daily_pnl <= -self.max_daily_loss {
            return RiskDecision {
                allowed: false,
                reason: format!("daily loss limit reached ({:.2}%)", self.max_daily_loss),
                suggested_size: 0.0,
                stop_price: 0.0,
            };
        }

        // Position sizing: risk a fixed % of equity per trade.
        let risk_amount = self.equity * (self.risk_per_trade_pct / 100.0);
        let stop_distance = price * (self.stop_percent / 100.0);
        if stop_distance <= 0.0 {
            return RiskDecision {
                allowed: false,
                reason: "price too low for stop distance".to_string(),
                suggested_size: 0.0,
                stop_price: 0.0,
            };
        }
        let size = (risk_amount / stop_distance).max(0.0);
        let stop_price = match side {
            OrderSide::Buy => price - stop_distance,
            OrderSide::Sell => price + stop_distance,
        };

        // Deduplicate symbol guard
        if open.iter().any(|t| t.symbol == symbol) {
            reasons.push("symbol already open".to_string());
        }

        let _ = total_pnl;
        RiskDecision {
            allowed: true,
            reason: if reasons.is_empty() { "ok".to_string() } else { reasons.join("; ") },
            suggested_size: size,
            stop_price,
        }
    }

    /// Update equity used for sizing (call each tick with account balance).
    pub fn update_equity(&mut self, equity: f64) {
        self.equity = equity.max(0.0);
    }

    /// Compute unrealized PnL for a position based on current price.
    pub fn unrealized_pnl(pos: &Position, current: f64) -> f64 {
        let direction = match pos.side {
            OrderSide::Buy => (current - pos.entry_price) / pos.entry_price,
            OrderSide::Sell => (pos.entry_price - current) / pos.entry_price,
        };
        direction * pos.amount
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizing_respects_risk_pct() {
        let m = RiskManager {
            max_daily_loss: 5.0,
            max_consecutive_losses: 5,
            max_open_positions: 3,
            risk_per_trade_pct: 0.5,
            equity: 10_000.0,
            stop_percent: 2.0,
        };
        let d = m.validate("EUR_USD", OrderSide::Buy, 1.1, &[], 0, 0.0, 0.0);
        assert!(d.allowed);
        let expected = (50.0) / (60_000.0 * 0.02);
        assert!((d.suggested_size - expected).abs() < 0.001);
    }

    #[test]
    fn rejects_over_exposure() {
        let m = RiskManager {
            max_open_positions: 1,
            ..RiskManager::new(&Config::from_env())
        };
        let open = vec![Trade {
            symbol: "ETH/USDT".to_string(),
            ..Default::default()
        }];
        let d = m.validate("EUR_USD", OrderSide::Buy, 1.1, &open, 0, 0.0, 0.0);
        assert!(!d.allowed);
    }
}