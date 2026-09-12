# Trading AI Terminal (Rust)

AI-powered desktop trading terminal for forex/gold on **Deriv Options** (primary),
with optional OANDA backup and Yahoo FX fallback. Runs real account P/L
reconciliation directly from the broker statement, mines 15+ years of historical
Dukascopy CSV data to build a persistent AI memory graph, and renders a
Bloomberg-style dark UI sized for 1440×900.

## What it does

- **Real Deriv account integration** — connects via WebSocket to Deriv's
  trading API, fetches live account balance, open contracts, profit_table,
  and full transaction statement.
- **NET PROFIT = balance − net deposits** — the portfolio panel shows actual
  account profit, not a paper-trades accumulator. Statement buy/sell rows are
  matched per contract to compute realized P/L; deposit and withdrawal rows are
  excluded from the profit calculation.
- **AI Memory Graph** — historical Dukascopy m5 CSV data (EURUSD, GBPUSD,
  USDJPY, XAUUSD from 2010) is parsed on first launch, patterns are mined
  (SMA/EMA crossovers, support/resistance zones, volatility regimes), and the
  results are persisted in `data/historicaldatacopy_strategy.json` and an
  in-memory cache. The graph visualizes three orbital tiers: live Signal nodes
  (inner ring), Regime nodes (mid ring), and Asset nodes (outer ring). Nodes are
  interactable — hover to see the node label, AI confidence, and the historical
  pattern context in a tooltip.
- **Strategy engine** — SMA/EMA/RSI/ATR/MACD indicators with weighted scoring,
  filtered by a risk gate (max daily loss, max consecutive losses, max open
  positions, risk per trade).
- **AI verdict gate** — Ollama (llama3.2:3b) can veto any proposed trade;
  verdicts feed the memory graph and accumulate as historical patterns.
- **Portfolio & Risk panel** — live balance, equity, NET PROFIT (realized +
  unrealized), win rate, open positions table, and factor weights.
- **SQLite persistence** — trades, app-state, and equity snapshots survive
  restarts. Historical data is re-mined from the CSVs if the cache is missing.

## Layout

```
crates/
  trading-core     config (.env loader), domain models, AI memory graph models
  trading-exchange DerivFx + OandaClient + YahooFx (async reqwest + websockets)
  trading-strategy indicators + signal scoring + historical data parser
  trading-risk     risk gate + unit tests
  trading-ai       Ollama client + AI memory (patterns, cache, context)
  trading-db       rusqlite persistence (WAL), app-state snapshots
  trading-api      axum REST + WS gateway (AppState, /api/*, /ws)
  trading-desktop  iced 0.13 GUI + in-process background engine loop
```

## Build & run (Windows)

Prerequisites: Rust stable + MSVC toolchain, and a local
[Ollama](https://ollama.com/) instance (`ollama pull llama3.2:3b`).

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cargo build --workspace
# or to run directly:
cargo run -p trading-desktop
```

### Configuration

Copy `.env.example` → `.env` and fill in the Deriv credentials (required):

```
FX_BROKER=deriv
DERIV_TOKEN=<your Deriv API token>
DERIV_APP_ID=<your Deriv App ID>
DERIV_ACCOUNT_ID=<your Options account id>
```

Ollama must be running locally (`ollama serve`). All other settings have safe
defaults.

### Release build

```powershell
cargo build -p trading-desktop --release
.\target\release\trading-desktop.exe
```

Release profile: opt-level 3, LTO, codegen-units 1, panic=abort, stripped.

## Historical data

The AI memory is seeded from Dukascopy m5 CSV files stored in:

```
%USERPROFILE%\Downloads\dukascopy-data-downloader\data\
  EURUSD_m5_20100101_20260101.csv
  GBPUSD_m5_20100101_20260101.csv
  USDJPY_m5_20100101_20260101.csv
  XAUUSD_m5_20100101_20260101.csv
```

On first launch, the engine parses these files, mines patterns, and writes the
strategy to `data/historicaldatacopy_strategy.json` (~13 KB). The raw pattern
cache (`historicaldatacopy_memory.cache.json`) is written on every session.

## Notes

- `fonts/` — JetBrains Mono TTFs embedded at compile time.
- `icons/app.ico` — window/taskbar icon drawn at runtime via iced's `from_file_data`.
- Market hours are detected automatically (Deriv's market_status endpoint);
  candles fall back from 5m → 1m → 1h when the market is closed.
- Contract deduplication: open positions are reconciled against the local
  portfolio on every tick to avoid phantom paper trades.
