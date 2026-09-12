//! SQLite persistence layer (rusqlite).

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use std::sync::{Arc, Mutex};
use trading_core::{Config, Market, OrderSide, Trade, TradeStatus};

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(cfg: &Config) -> Result<Self> {
        let conn = Connection::open(&cfg.db_path)
            .with_context(|| format!("open db {:?}", cfg.db_path))?;
        let db = Self { conn: Arc::new(Mutex::new(conn)) };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_path(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn: Arc::new(Mutex::new(conn)) };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;

            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                symbol TEXT NOT NULL,
                side TEXT NOT NULL,
                entry_price REAL NOT NULL,
                exit_price REAL,
                amount REAL NOT NULL,
                stop_loss REAL NOT NULL DEFAULT 0,
                take_profit REAL NOT NULL DEFAULT 0,
                pnl REAL,
                status TEXT NOT NULL DEFAULT 'open',
                regime TEXT DEFAULT '',
                confidence REAL DEFAULT 0,
                reason TEXT DEFAULT '',
                ai_verdict TEXT DEFAULT '',
                ai_reason TEXT DEFAULT '',
                quality_score REAL DEFAULT 0,
                market TEXT DEFAULT 'forex',
                broker TEXT DEFAULT 'oanda',
                external_id TEXT,
                entry_time TEXT NOT NULL,
                exit_time TEXT
            );

            CREATE TABLE IF NOT EXISTS app_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                equity_crypto REAL DEFAULT 10000,
                equity_fx REAL DEFAULT 10000,
                daily_pnl REAL DEFAULT 0,
                total_pnl REAL DEFAULT 0,
                consecutive_losses INTEGER DEFAULT 0,
                kill_switch INTEGER DEFAULT 0,
                crypto_enabled INTEGER DEFAULT 0,
                fx_enabled INTEGER DEFAULT 1,
                active_engine TEXT DEFAULT 'hybrid',
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS performance (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT UNIQUE,
                total_pnl REAL,
                trades_count INTEGER,
                win_rate REAL
            );

            INSERT OR IGNORE INTO app_state (id) VALUES (1);
            "#,
        )?;
        Ok(())
    }

    // ── Trades ────────────────────────────────────────────────
    pub fn save_trade(&self, t: &Trade) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO trades (symbol, side, entry_price, exit_price, amount,
                stop_loss, take_profit, pnl, status, regime, confidence, reason,
                ai_verdict, ai_reason, quality_score, market, broker, external_id,
                entry_time, exit_time)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
            "#,
            rusqlite::params![
                t.symbol,
                t.side.as_str(),
                t.entry_price,
                t.exit_price,
                t.amount,
                t.stop_loss,
                t.take_profit,
                t.pnl,
                trade_status_str(t.status),
                t.regime,
                t.confidence,
                t.reason,
                t.ai_verdict,
                t.ai_reason,
                t.quality_score,
                t.market.as_str(),
                t.broker,
                t.external_id,
                t.entry_time,
                t.exit_time,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn close_trade(&self, id: i64, exit_price: f64, pnl: f64, reason: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            UPDATE trades SET exit_price = ?1, pnl = ?2, status = 'closed',
                exit_time = ?3, reason = CASE WHEN reason = '' THEN ?4 ELSE reason END
            WHERE id = ?5
            "#,
            rusqlite::params![exit_price, pnl, Utc::now().to_rfc3339(), reason, id],
        )?;
        Ok(())
    }

    pub fn load_recent_trades(&self, limit: i64) -> Result<Vec<Trade>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, symbol, side, entry_price, exit_price, amount, stop_loss,
                   take_profit, pnl, status, regime, confidence, reason, ai_verdict,
                   ai_reason, quality_score, market, broker, external_id, entry_time, exit_time
            FROM trades ORDER BY id DESC LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(Trade {
                id: Some(r.get(0)?),
                symbol: r.get(1)?,
                side: OrderSide::Buy,
                entry_price: r.get(3)?,
                exit_price: r.get(4)?,
                amount: r.get(5)?,
                stop_loss: r.get(6)?,
                take_profit: r.get(7)?,
                pnl: r.get(8)?,
                status: parse_trade_status(&r.get::<_, String>(9)?),
                regime: r.get(10)?,
                confidence: r.get(11)?,
                reason: r.get(12)?,
                ai_verdict: r.get(13)?,
                ai_reason: r.get(14)?,
                quality_score: r.get(15)?,
                market: Market::from_symbol(&r.get::<_, String>(17)?),
                broker: r.get(18)?,
                external_id: r.get(19)?,
                entry_time: r.get(20)?,
                exit_time: r.get(21)?,
                ..Default::default()
            })
        })?;

        let mut trades = Vec::new();
        for row in rows {
            let mut t = row?;
            let side_str: String = {
                let conn2 = self.conn.lock().unwrap();
                conn2.query_row(
                    "SELECT side FROM trades WHERE id = ?1",
                    [t.id.unwrap()],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .unwrap_or_else(|| "buy".into())
            };
            t.side = if side_str.eq_ignore_ascii_case("sell") {
                OrderSide::Sell
            } else {
                OrderSide::Buy
            };
            trades.push(t);
        }
        Ok(trades)
    }

    pub fn load_open_trades(&self) -> Result<Vec<Trade>> {
        let all = self.load_recent_trades(10_000)?;
        Ok(all.into_iter().filter(|t| t.status == TradeStatus::Open).collect())
    }

    // ── App state ─────────────────────────────────────────────
    pub fn load_app_state(&self) -> Result<AppState> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            r#"
            SELECT equity_crypto, equity_fx, daily_pnl, total_pnl, consecutive_losses,
                   kill_switch, crypto_enabled, fx_enabled, active_engine
            FROM app_state WHERE id = 1
            "#,
            [],
            |r| {
                Ok(AppState {
                    equity_forex: r.get(0)?,
                    equity_gold: r.get(1)?,
                    daily_pnl: r.get(2)?,
                    total_pnl: r.get(3)?,
                    consecutive_losses: r.get(4)?,
                    kill_switch: r.get::<_, i64>(5)? != 0,
                    forex_enabled: r.get::<_, i64>(6)? != 0,
                    gold_enabled: r.get::<_, i64>(7)? != 0,
                    active_engine: r.get(8)?,
                })
            },
        )
        .optional()
        .with_context(|| "load app_state")
        .map(|o| o.unwrap_or_default())
    }

    pub fn save_app_state(&self, st: &AppState) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            UPDATE app_state SET equity_crypto = ?1, equity_fx = ?2, daily_pnl = ?3,
                total_pnl = ?4, consecutive_losses = ?5, kill_switch = ?6,
                crypto_enabled = ?7, fx_enabled = ?8, active_engine = ?9, updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            rusqlite::params![
                st.equity_forex,
                st.equity_gold,
                st.daily_pnl,
                st.total_pnl,
                st.consecutive_losses,
                st.kill_switch as i64,
                st.forex_enabled as i64,
                st.gold_enabled as i64,
                st.active_engine,
            ],
        )?;
        Ok(())
    }

    pub fn save_daily_snapshot(&self, pnl: f64, trades: i64, win_rate: f64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO performance (date, total_pnl, trades_count, win_rate)
            VALUES (DATE('now'), ?1, ?2, ?3)
            ON CONFLICT(date) DO UPDATE SET total_pnl = ?1, trades_count = ?2, win_rate = ?3
            "#,
            rusqlite::params![pnl, trades, win_rate],
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub equity_forex: f64,
    pub equity_gold: f64,
    pub daily_pnl: f64,
    pub total_pnl: f64,
    pub consecutive_losses: i32,
    pub kill_switch: bool,
    pub forex_enabled: bool,
    pub gold_enabled: bool,
    pub active_engine: String,
}

fn trade_status_str(s: TradeStatus) -> &'static str {
    match s {
        TradeStatus::Open => "open",
        TradeStatus::Closed => "closed",
    }
}

fn parse_trade_status(s: &str) -> TradeStatus {
    if s == "closed" {
        TradeStatus::Closed
    } else {
        TradeStatus::Open
    }
}