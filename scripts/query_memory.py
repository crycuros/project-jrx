import argparse
import sys
import time
from pathlib import Path
import duckdb
import numpy as np
import pandas as pd

BASE_DIR = Path(__file__).resolve().parent.parent
MEMORY_DIR = BASE_DIR / "trading_memory"
DB_PATH = MEMORY_DIR / "vector_store" / "trading_memory.duckdb"

def query_memory(
    symbol: str = "XAUUSD",
    rsi: float = 62.5,
    ema_dist: float = 0.35,
    regime: int = 1,
    direction: int = 1,
    forward_bars: int = 15,
    top_k: int = 15
):
    if not DB_PATH.exists():
        print(f"Error: Database file not found at {DB_PATH}")
        sys.exit(1)

    t0 = time.time()
    conn = duckdb.connect(str(DB_PATH), read_only=True)
    conn.execute("PRAGMA threads=8;")

    sym = symbol.upper()
    dir_label = "LONG (BUY)" if direction == 1 else "SHORT (SELL)"
    regime_label = "BULLISH TREND" if regime == 1 else ("BEARISH TREND" if regime == -1 else "RANGING")

    sql = f"""
    WITH windowed AS (
        SELECT 
            symbol, ts, close, rsi_14, atr_14, regime, is_breakout_h, is_breakout_l,
            ((close - ema_50) / ema_50) * 100.0 as ema_dist,
            lead(close, {forward_bars}) OVER (PARTITION BY symbol ORDER BY ts) as f_close
        FROM full_candles
        WHERE symbol = '{sym}'
    ),
    filtered AS (
        SELECT 
            symbol, ts, close, rsi_14, atr_14, regime, ema_dist, f_close,
            round(
                abs(rsi_14 - {rsi}) * 0.4 +
                abs(ema_dist - {ema_dist}) * 2.0 +
                abs(regime - {regime}) * 3.0,
                3
            ) as distance,
            case 
                when {direction} = 1 then
                    case when f_close > close then 'WIN' else 'LOSS' end
                else
                    case when f_close < close then 'WIN' else 'LOSS' end
            end as outcome,
            case 
                when {direction} = 1 then
                    round((f_close - close) / nullif(atr_14 * 1.5, 0.000001), 2)
                else
                    round((close - f_close) / nullif(atr_14 * 1.5, 0.000001), 2)
            end as result_r
        FROM windowed
        WHERE regime = {regime} 
          AND rsi_14 BETWEEN {max(0.0, rsi - 15.0)} AND {min(100.0, rsi + 15.0)}
          AND f_close IS NOT NULL
    )
    SELECT 
        symbol, ts, close, rsi_14, atr_14, ema_dist, outcome, result_r, distance
    FROM filtered
    ORDER BY distance ASC
    LIMIT {top_k};
    """

    df = conn.execute(sql).df()
    conn.close()
    elapsed = time.time() - t0

    if df.empty:
        print(f"[!] No matching historical patterns found for {sym} under current filters.")
        return

    wins = int((df['outcome'] == 'WIN').sum())
    losses = int((df['outcome'] == 'LOSS').sum())
    total = wins + losses
    win_rate = (wins / total * 100.0) if total > 0 else 0.0
    avg_r = float(df['result_r'].mean())

    print("=" * 80)
    print(f"      TRADING MEMORY RETRIEVAL (12.5GB+ INSTITUTIONAL DATABASE)")
    print(f"      Target: {sym} | RSI: {rsi:.1f} | EMA Spread: {ema_dist:+.2f}% | Regime: {regime_label}")
    print(f"      Proposed Trade: {dir_label} | Forward Horizon: {forward_bars} Bars")
    print("=" * 80)
    print(f"  • Matched Historical Setups : {len(df)} nearest situations across 2000–2025")
    print(f"  • Win / Loss Ratio          : {wins} WINS / {losses} LOSSES")
    print(f"  • Historical Win Rate       : {win_rate:.1f}%")
    print(f"  • Expected Return per Trade : {avg_r:+.2f}R")
    print(f"  • Query Retrieval Speed     : {elapsed:.2f} seconds")
    print("-" * 80)
    print(f"{'#':<3} {'Date / Time':<18} {'Price':<10} {'RSI':<6} {'EMA Dist':<10} {'ATR':<8} {'Outcome':<8} {'Payoff (R)'}")
    print("-" * 80)
    for idx, row in df.iterrows():
        outcome_color = row['outcome']
        print(f"{idx+1:<3} {str(row['ts'])[:16]:<18} {row['close']:<10.3f} {row['rsi_14']:<6.1f} {row['ema_dist']:+7.2f}%   {row['atr_14']:<8.4f} {outcome_color:<8} {row['result_r']:+6.2f}R")
    print("=" * 80)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Query 12.5GB Institutional Trading Memory")
    parser.add_argument("--symbol", default="XAUUSD", help="Symbol (e.g. XAUUSD, EURUSD, GBPUSD, USDJPY)")
    parser.add_argument("--rsi", type=float, default=62.5, help="Current RSI level")
    parser.add_argument("--ema_dist", type=float, default=0.35, help="Distance from EMA50 in %")
    parser.add_argument("--regime", type=int, default=1, help="Market Regime: 1 (Bullish), -1 (Bearish), 0 (Ranging)")
    parser.add_argument("--direction", type=int, default=1, help="Trade Direction: 1 (Long), -1 (Short)")
    parser.add_argument("--forward_bars", type=int, default=15, help="Forward evaluation horizon in bars")
    parser.add_argument("--top_k", type=int, default=15, help="Number of closest setups to retrieve")

    args = parser.parse_args()
    query_memory(
        symbol=args.symbol,
        rsi=args.rsi,
        ema_dist=args.ema_dist,
        regime=args.regime,
        direction=args.direction,
        forward_bars=args.forward_bars,
        top_k=args.top_k
    )
