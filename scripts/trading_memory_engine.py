import os
import sys
import time
from pathlib import Path
import duckdb
import numpy as np
import pandas as pd
import pyarrow.parquet as pq

BASE_DIR = Path(__file__).resolve().parent.parent
MEMORY_DIR = BASE_DIR / "trading_memory"
RAW_DIR = MEMORY_DIR / "raw"
FEATURES_DIR = MEMORY_DIR / "features"
SETUPS_DIR = MEMORY_DIR / "setups"
VECTOR_DIR = MEMORY_DIR / "vector_store"

for d in [FEATURES_DIR, SETUPS_DIR, VECTOR_DIR]:
    d.mkdir(parents=True, exist_ok=True)

DB_PATH = VECTOR_DIR / "trading_memory.duckdb"

def compute_indicators(df: pd.DataFrame) -> pd.DataFrame:
    """Computes technical indicators and market regime features for a dataframe."""
    df = df.copy()
    close = df['close']
    high = df['high']
    low = df['low']

    # 1. EMAs
    df['ema_20'] = close.ewm(span=20, adjust=False).mean()
    df['ema_50'] = close.ewm(span=50, adjust=False).mean()
    df['ema_200'] = close.ewm(span=200, adjust=False).mean()

    # 2. RSI (14)
    delta = close.diff()
    gain = delta.clip(lower=0)
    loss = -delta.clip(upper=0)
    avg_gain = gain.rolling(window=14, min_periods=14).mean()
    avg_loss = loss.rolling(window=14, min_periods=14).mean()
    rs = avg_gain / (avg_loss + 1e-9)
    df['rsi'] = 100 - (100 / (1 + rs))

    # 3. ATR (14)
    tr1 = high - low
    tr2 = (high - close.shift(1)).abs()
    tr3 = (low - close.shift(1)).abs()
    tr = pd.concat([tr1, tr2, tr3], axis=1).max(axis=1)
    df['atr'] = tr.rolling(window=14, min_periods=14).mean()
    df['atr_pct'] = (df['atr'] / (close + 1e-9)) * 100

    # 4. Price vs EMA / VWAP proxy
    df['price_vs_ema50_pct'] = ((close - df['ema_50']) / df['ema_50']) * 100
    df['ema20_vs_ema50_pct'] = ((df['ema_20'] - df['ema_50']) / df['ema_50']) * 100

    # 5. Market Regime
    # 0 = ranging, 1 = trending_up, -1 = trending_down
    conditions = [
        (df['close'] > df['ema_50']) & (df['ema_20'] > df['ema_50']) & (df['rsi'] > 52),
        (df['close'] < df['ema_50']) & (df['ema_20'] < df['ema_50']) & (df['rsi'] < 48),
    ]
    choices = [1, -1] # 1: Bullish, -1: Bearish
    df['regime'] = np.select(conditions, choices, default=0) # 0: Ranging

    # 6. Structure: Higher High / Lower Low rolling 20
    rolling_max = high.rolling(20).max()
    rolling_min = low.rolling(20).min()
    df['is_breakout_high'] = (close >= rolling_max.shift(1)).astype(int)
    df['is_breakout_low'] = (close <= rolling_min.shift(1)).astype(int)

    return df.dropna()

def resample_ohlc(df: pd.DataFrame, timeframe: str) -> pd.DataFrame:
    """Resamples 1m data to 5m, 15m, or 1h bars."""
    df = df.copy()
    if not isinstance(df.index, pd.DatetimeIndex):
        df['ts'] = pd.to_datetime(df['ts'])
        df.set_index('ts', inplace=True)
    
    rule = timeframe.replace('m', 'min')
    resampled = df.resample(rule).agg({
        'open': 'first',
        'high': 'max',
        'low': 'min',
        'close': 'last',
        'volume': 'sum'
    }).dropna()
    return resampled.reset_index()

def extract_setups(df: pd.DataFrame, symbol: str, timeframe: str, forward_bars: int = 15) -> list:
    """
    Scans the historical series and extracts defined trading setups
    along with their forward outcome (Win/Loss and R-multiple).
    """
    setups = []
    closes = df['close'].values
    highs = df['high'].values
    lows = df['low'].values
    rsis = df['rsi'].values
    atrs = df['atr'].values
    regimes = df['regime'].values
    ema_dists = df['price_vs_ema50_pct'].values
    breakouts_h = df['is_breakout_high'].values
    breakouts_l = df['is_breakout_low'].values
    timestamps = df['ts'].values

    n = len(df)
    # Stride by 3 bars to avoid overlapping almost identical entries
    for i in range(50, n - forward_bars, 3):
        rsi = rsis[i]
        atr = atrs[i]
        regime = regimes[i]
        ema_dist = ema_dists[i]
        c = closes[i]

        setup_type = None
        direction = 0 # 1 = Long, -1 = Short

        # Setup A: Breakout Retest Long (Bullish trend, fresh breakout)
        if regime == 1 and breakouts_h[i] == 1 and rsi > 55 and rsi < 75:
            setup_type = "breakout_long"
            direction = 1

        # Setup B: Breakout Retest Short (Bearish trend, fresh breakdown)
        elif regime == -1 and breakouts_l[i] == 1 and rsi < 45 and rsi > 25:
            setup_type = "breakout_short"
            direction = -1

        # Setup C: Pullback to Trend (Bullish pull back near EMA50)
        elif regime == 1 and abs(ema_dist) < 0.25 and 45 <= rsi <= 55:
            setup_type = "trend_pullback_long"
            direction = 1

        # Setup D: Pullback to Trend Short
        elif regime == -1 and abs(ema_dist) < 0.25 and 45 <= rsi <= 55:
            setup_type = "trend_pullback_short"
            direction = -1

        if setup_type and atr > 0:
            # Evaluate outcome over next forward_bars
            sl_dist = atr * 1.5
            tp_dist = atr * 3.0 # Target 2:1 R:R

            sl_price = c - sl_dist if direction == 1 else c + sl_dist
            tp_price = c + tp_dist if direction == 1 else c - tp_dist

            result_r = 0.0
            outcome = "scratch"

            future_highs = highs[i+1 : i+1+forward_bars]
            future_lows = lows[i+1 : i+1+forward_bars]

            for fh, fl in zip(future_highs, future_lows):
                if direction == 1:
                    if fl <= sl_price:
                        result_r = -1.0
                        outcome = "loss"
                        break
                    if fh >= tp_price:
                        result_r = 2.0
                        outcome = "win"
                        break
                else:
                    if fh >= sl_price:
                        result_r = -1.0
                        outcome = "loss"
                        break
                    if fl <= tp_price:
                        result_r = 2.0
                        outcome = "win"
                        break
            else:
                # If neither SL nor TP hit within forward_bars, calculate partial R
                final_c = closes[min(i + forward_bars, n - 1)]
                raw_diff = (final_c - c) if direction == 1 else (c - final_c)
                result_r = round(raw_diff / (sl_dist + 1e-9), 2)
                outcome = "win" if result_r > 0.2 else ("loss" if result_r < -0.2 else "scratch")

            setups.append({
                "symbol": symbol.upper(),
                "timeframe": timeframe,
                "timestamp": str(timestamps[i]),
                "setup_type": setup_type,
                "direction": direction,
                "rsi": float(rsi),
                "atr": float(atr),
                "ema_dist": float(ema_dist),
                "regime": int(regime),
                "entry_price": float(c),
                "outcome": outcome,
                "result_r": float(result_r),
                # Normalized embedding vector components for instant cosine distance
                "vec_rsi": float(rsi / 100.0),
                "vec_ema_dist": float(np.clip(ema_dist / 2.0, -1.0, 1.0)),
                "vec_regime": float(regime),
                "vec_direction": float(direction),
            })

    return setups

def build_memory():
    """Builds features, extracts setups, and populates the DuckDB Vector Store."""
    print("=" * 65)
    print("      BUILDING 5GB+ TRADING MEMORY & VECTOR DATABASE")
    print("=" * 65)

    conn = duckdb.connect(str(DB_PATH))
    conn.execute("""
        CREATE TABLE IF NOT EXISTS historical_setups (
            symbol VARCHAR,
            timeframe VARCHAR,
            timestamp VARCHAR,
            setup_type VARCHAR,
            direction INTEGER,
            rsi DOUBLE,
            atr DOUBLE,
            ema_dist DOUBLE,
            regime INTEGER,
            entry_price DOUBLE,
            outcome VARCHAR,
            result_r DOUBLE,
            vec_rsi DOUBLE,
            vec_ema_dist DOUBLE,
            vec_regime DOUBLE,
            vec_direction DOUBLE
        );
    """)

    # Check if setups already populated
    count = conn.execute("SELECT count(*) FROM historical_setups").fetchone()[0]
    if count > 5000:
        print(f"[FOUND] Trading Memory already contains {count:,} indexed setups in {DB_PATH.name}!")
        return conn

    symbols = ["xauusd", "eurusd", "gbpusd", "usdjpy"]
    timeframes = ["15m", "1h"]

    all_setups = []

    for sym in symbols:
        raw_file = RAW_DIR / f"{sym}_1m.parquet"
        if not raw_file.exists():
            print(f"Skipping {sym}, file not found.")
            continue

        print(f"\nProcessing {sym.upper()} historical data...")
        # Use DuckDB to query only recent 1.5 million rows per symbol for rapid feature engineering
        df_1m = duckdb.query(f"SELECT ts, open, high, low, close, volume FROM read_parquet('{raw_file.as_posix()}') ORDER BY ts DESC LIMIT 1500000").df()
        df_1m.sort_values('ts', inplace=True)
        print(f"  Loaded {len(df_1m):,} 1m bars.")

        for tf in timeframes:
            print(f"  Resampling to {tf}...")
            df_tf = resample_ohlc(df_1m, tf)
            df_feat = compute_indicators(df_tf)
            setups = extract_setups(df_feat, sym, tf)
            print(f"  Extracted {len(setups):,} verified trade setups on {sym.upper()} {tf}!")
            all_setups.extend(setups)

    print(f"\nInserting {len(all_setups):,} setups into DuckDB Vector Store...")
    df_setups = pd.DataFrame(all_setups)
    conn.register("df_temp", df_setups)
    conn.execute("INSERT INTO historical_setups SELECT * FROM df_temp")
    
    total = conn.execute("SELECT count(*) FROM historical_setups").fetchone()[0]
    print(f"[SUCCESS] Trading Memory ready! Total indexed setups: {total:,}")
    return conn

def query_similar_setups(conn, symbol: str, timeframe: str, rsi: float, ema_dist: float, regime: int, direction: int, top_k: int = 20):
    """
    Vector similarity query:
    Calculates Euclidean/Cosine distance against all indexed historical setups
    and retrieves the Top-K most similar historical situations + outcome statistics.
    """
    q_rsi = rsi / 100.0
    q_dist = float(np.clip(ema_dist / 2.0, -1.0, 1.0))
    q_reg = float(regime)
    q_dir = float(direction)

    # Vector distance formula in DuckDB SQL:
    # sqrt((vec_rsi - q_rsi)^2 + (vec_ema_dist - q_dist)^2 + (vec_regime - q_reg)^2 + (vec_direction - q_dir)^2)
    sql = f"""
    SELECT 
        symbol, timeframe, timestamp, setup_type, rsi, atr, ema_dist, regime, outcome, result_r,
        sqrt(
            power(vec_rsi - {q_rsi}, 2) * 2.0 +
            power(vec_ema_dist - {q_dist}, 2) * 2.5 +
            power(vec_regime - {q_reg}, 2) * 3.0 +
            power(vec_direction - {q_dir}, 2) * 4.0
        ) as distance
    FROM historical_setups
    WHERE symbol = '{symbol.upper()}' AND timeframe = '{timeframe}'
    ORDER BY distance ASC
    LIMIT {top_k}
    """
    df_matches = conn.execute(sql).df()
    
    if df_matches.empty:
        # Fallback to any timeframe or symbol if exact matches are sparse
        sql_fallback = f"""
        SELECT 
            symbol, timeframe, timestamp, setup_type, rsi, atr, ema_dist, regime, outcome, result_r,
            sqrt(
                power(vec_rsi - {q_rsi}, 2) * 2.0 +
                power(vec_ema_dist - {q_dist}, 2) * 2.5 +
                power(vec_regime - {q_reg}, 2) * 3.0 +
                power(vec_direction - {q_dir}, 2) * 4.0
            ) as distance
        FROM historical_setups
        WHERE symbol = '{symbol.upper()}'
        ORDER BY distance ASC
        LIMIT {top_k}
        """
        df_matches = conn.execute(sql_fallback).df()

    wins = int((df_matches['outcome'] == 'win').sum())
    losses = int((df_matches['outcome'] == 'loss').sum())
    avg_r = float(df_matches['result_r'].mean()) if not df_matches.empty else 0.0
    win_rate = (wins / (wins + losses) * 100.0) if (wins + losses) > 0 else 0.0

    return {
        "symbol": symbol.upper(),
        "timeframe": timeframe,
        "matches_count": len(df_matches),
        "wins": wins,
        "losses": losses,
        "win_rate": round(win_rate, 1),
        "avg_r": round(avg_r, 2),
        "top_matches": df_matches.to_dict(orient="records")
    }

if __name__ == "__main__":
    conn = build_memory()

    # Demonstration: Live Query Simulation for Gold (XAUUSD)
    print("\n" + "=" * 65)
    print("   AI SIMULATION: QUERYING HISTORICAL TRADING MEMORY")
    print("   Situation: XAUUSD 15m, RSI 62.5, Bullish Trend, +0.35% above EMA")
    print("=" * 65)

    res = query_similar_setups(
        conn,
        symbol="XAUUSD",
        timeframe="15m",
        rsi=62.5,
        ema_dist=0.35,
        regime=1, # Bullish
        direction=1, # Long
        top_k=20
    )

    print(f"\n[QUERY RESULTS for {res['symbol']} {res['timeframe']}]:")
    print(f"  Similar Historical Setups Found: {res['matches_count']}")
    print(f"  Wins: {res['wins']}  |  Losses: {res['losses']}")
    print(f"  Historical Win Rate: {res['win_rate']}%")
    print(f"  Average Outcome:     {res['avg_r']:+.2f}R")
    print("\nTop 5 Closest Historical Matches in Memory:")
    for idx, m in enumerate(res['top_matches'][:5], 1):
        print(f"  #{idx} Date: {m['timestamp'][:16]} | Pattern: {m['setup_type']} | RSI: {m['rsi']:.1f} | Outcome: {m['outcome'].upper()} ({m['result_r']:+.1f}R)")
