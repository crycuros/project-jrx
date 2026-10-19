# Trading AI Terminal (Rust)

![Project-JRX](https://github.com/crycuros/project-jrx/releases/download/v1.0/Screenshot_3.png)

AI-powered desktop trading terminal for forex/gold on **Deriv Options** (primary),
with optional OANDA backup and Yahoo FX fallback. Runs real account P/L
reconciliation directly from the broker statement and renders a
Bloomberg-style dark UI sized for 1440×900.

## What it does

- **Real Deriv account integration** — connects via WebSocket to Deriv's
  trading API, fetches live account balance, open contracts, profit_table,
  and full transaction statement.
- **NET PROFIT = balance − net deposits** — the portfolio panel shows actual
  account profit, not a paper-trades accumulator. Statement buy/sell rows are
  matched per contract to compute realized P/L; deposit and withdrawal rows are
  excluded from the profit calculation.
- **AI Memory Graph** — a three-tier orbital graph of mined market patterns:
  live Signal nodes (inner ring), Regime nodes (mid ring), and Asset nodes
  (outer ring). Nodes are interactable — hover to see the node label, AI
  confidence, and the pattern context in a tooltip.
- **Strategy engine** — SMA/EMA/RSI/ATR/MACD indicators with weighted scoring,
  filtered by a risk gate (max daily loss, max consecutive losses, max open
  positions, risk per trade).
- **AI verdict gate** — Ollama (llama3.2:3b) can veto any proposed trade;
  verdicts feed the memory graph and accumulate as historical patterns.
- **Portfolio & Risk panel** — live balance, equity, NET PROFIT (realized +
  unrealized), win rate, open positions table, and factor weights.
- **SQLite persistence** — trades, app-state, and equity snapshots survive
  restarts.

## The ReAct agent gate

Every trade the bot places is put to the ReAct agent first — the Next.js
terminal in `../ai-react-trader`. The gate **fails closed**:

| the agent says | the bot does |
|---|---|
| APPROVE (confidence ≥ `AGENT_MIN_CONFIDENCE`) | trades, with the agent's stake cap |
| HOLD | does not trade |
| confidence below the gate | does not trade |
| unreachable / timed out / unreadable | **does not trade** |

```bash
# in .env
AGENT_API_URL=http://127.0.0.1:3000
AGENT_TIMEOUT_SECS=45
AGENT_MIN_CONFIDENCE=0.6
AGENT_REQUIRED=true          # do not set this to false in production
AGENT_API_SECRET=            # the terminal's API_SECRET, if it has one
```

The bot sends its own live indicators with the request, so the agent reasons
about the market as it is rather than about the archive.

Check the wiring at any time:

```bash
cargo run -p trading-ai --example ask_agent
```

```
asking http://127.0.0.1:3000 …
APPROVE  confidence 0.64
  reason: The conditioned cell is positive (+0.07R) ...
  stake cap: $10.00
```

Stop the terminal and run it again: it reports `UNAVAILABLE`, and the bot
trades nothing.

---

## Layout

```
crates/
  trading-core     config (.env loader), domain models, AI memory graph models
  trading-exchange DerivFx + OandaClient + YahooFx (async reqwest + websockets)
  trading-strategy indicators + signal scoring + pattern engine
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

## Notes

- `fonts/` — JetBrains Mono TTFs embedded at compile time.
- `icons/app.ico` — window/taskbar icon drawn at runtime via iced's `from_file_data`.
- Market hours are detected automatically (Deriv's market_status endpoint);
  candles fall back from 5m → 1m → 1h when the market is closed.
- Contract deduplication: open positions are reconciled against the local
  portfolio on every tick to avoid phantom paper trades.
