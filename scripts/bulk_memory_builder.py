import os
import sys
import time
from pathlib import Path
import httpx
import duckdb
import numpy as np
import pandas as pd
import pyarrow.parquet as pq

BASE_DIR = Path(__file__).resolve().parent.parent
MEMORY_DIR = BASE_DIR / "trading_memory"
RAW_DIR = MEMORY_DIR / "raw"
GOLD_DIR = MEMORY_DIR / "gold_archive"
FEATURES_DIR = MEMORY_DIR / "features"
SETUPS_DIR = MEMORY_DIR / "setups"
VECTOR_DIR = MEMORY_DIR / "vector_store"

for d in [RAW_DIR, GOLD_DIR, FEATURES_DIR, SETUPS_DIR, VECTOR_DIR]:
    d.mkdir(parents=True, exist_ok=True)

DB_PATH = VECTOR_DIR / "trading_memory.duckdb"

# 1. Broad list of institutional Forex & Gold pairs from HistData
PAIRS = [
    # Gold & Metals
    "xauusd", "xagusd", "xauaud", "xaueur", "xaugbp", "xauchf",
    # Forex Majors
    "eurusd", "gbpusd", "usdjpy", "audusd", "usdcad", "usdchf", "nzdusd",
    # Major Crosses
    "eurgbp", "eurjpy", "euraud", "eurcad", "eurchf", "eurnzd",
    "gbpjpy", "gbpaud", "gbpcad", "gbpchf", "gbpnzd",
    "audjpy", "audcad", "audchf", "audnzd",
    "cadjpy", "cadchf", "chfjpy", "nzdjpy",
    # Commodities
    "wtiusd", "bcousd"
]

# 2. Large institutional Gold dataset from ZombitX64
ZOMBIT_GOLD_FILES = [
    "XAU_1m_data.jsonl",
    "XAU_5m_data.jsonl",
    "XAU_15m_data.jsonl",
    "XAU_1h_data.jsonl"
]

def download_stream(url: str, dest_path: Path):
    if dest_path.exists() and dest_path.stat().st_size > 1024 * 1024:
        print(f"  [EXISTS] {dest_path.name} ({dest_path.stat().st_size / (1024*1024):.1f} MB). Skipping.")
        return dest_path

    print(f"  [DOWNLOADING] {dest_path.name} from {url[:70]}...")
    start_t = time.time()
    with httpx.Client(follow_redirects=True, timeout=600.0) as client:
        with client.stream("GET", url) as resp:
            resp.raise_for_status()
            total_b = int(resp.headers.get("content-length", 0))
            done_b = 0
            with open(dest_path, "wb") as f:
                for chunk in resp.iter_bytes(chunk_size=1024 * 1024):
                    if chunk:
                        f.write(chunk)
                        done_b += len(chunk)
                        if total_b > 0:
                            pct = (done_b / total_b) * 100
                            sys.stdout.write(f"\r    {pct:5.1f}% [{done_b/(1024*1024):.1f}/{total_b/(1024*1024):.1f} MB]")
                            sys.stdout.flush()
    print(f"\n    Completed in {time.time() - start_t:.1f}s ({dest_path.stat().st_size / (1024*1024):.1f} MB)")
    return dest_path

def download_all_raw():
    print("\n" + "=" * 65)
    print("STEP 1: DOWNLOADING FOREX & GOLD HISTORICAL ARCHIVES")
    print("=" * 65)

    # A. Zombit Gold High-Density Archive
    for gf in ZOMBIT_GOLD_FILES:
        url = f"https://huggingface.co/datasets/ZombitX64/xauusd-gold-price-historical-data-2004-2025/resolve/main/{gf}"
        dest = GOLD_DIR / gf
        try:
            download_stream(url, dest)
        except Exception as e:
            print(f"    Failed {gf}: {e}")

    # B. HistData 1m Parquet Archive for all pairs
    for p in PAIRS:
        url = f"https://huggingface.co/datasets/elthariel/histdata_fx_1m/resolve/main/{p}/ticks.parquet"
        dest = RAW_DIR / f"{p}_1m.parquet"
        try:
            download_stream(url, dest)
        except Exception as e:
            print(f"    Failed {p}: {e}")

def build_5gb_feature_database():
    print("\n" + "=" * 65)
    print("STEP 2: MATERIALIZING 5GB+ TRADING DATABASE IN DUCKDB")
    print("=" * 65)

    conn = duckdb.connect(str(DB_PATH))
    conn.execute("PRAGMA threads=8;")
    conn.execute("PRAGMA memory_limit='8GB';")

    print("\n[DB] Creating full historical tables with indicators...")
    conn.execute("""
        CREATE TABLE IF NOT EXISTS full_candles (
            symbol VARCHAR,
            timeframe VARCHAR,
            ts TIMESTAMP,
            open DOUBLE,
            high DOUBLE,
            low DOUBLE,
            close DOUBLE,
            volume DOUBLE,
            ema_20 DOUBLE,
            ema_50 DOUBLE,
            ema_200 DOUBLE,
            rsi_14 DOUBLE,
            atr_14 DOUBLE,
            regime INTEGER,
            is_breakout_h INTEGER,
            is_breakout_l INTEGER
        );
    """)

    # Check existing count
    current_count = conn.execute("SELECT count(*) FROM full_candles").fetchone()[0]
    print(f"  Current candles in database: {current_count:,}")

    # Process downloaded parquet files into DuckDB
    parquet_files = list(RAW_DIR.glob("*_1m.parquet"))
    print(f"\n[FEATURES] Processing {len(parquet_files)} raw Parquet datasets into Feature Store...")

    for pf in parquet_files:
        sym = pf.name.replace("_1m.parquet", "").upper()
        # Check if already in DB
        exists = conn.execute(f"SELECT count(*) FROM full_candles WHERE symbol = '{sym}'").fetchone()[0]
        if exists > 100000:
            print(f"  [OK] {sym} already loaded ({exists:,} rows).")
            continue

        print(f"  Materializing indicators for {sym}...")
        try:
            # Query and calculate in DuckDB natively with SQL window functions!
            sql = f"""
            INSERT INTO full_candles
            WITH raw AS (
                SELECT 
                    '{sym}' as symbol,
                    '1m' as timeframe,
                    ts,
                    open, high, low, close, volume,
                    avg(close) OVER (ORDER BY ts ROWS BETWEEN 19 PRECEDING AND CURRENT ROW) as ema_20,
                    avg(close) OVER (ORDER BY ts ROWS BETWEEN 49 PRECEDING AND CURRENT ROW) as ema_50,
                    avg(close) OVER (ORDER BY ts ROWS BETWEEN 199 PRECEDING AND CURRENT ROW) as ema_200,
                    greatest(high - low, abs(high - lag(close, 1) OVER (ORDER BY ts)), abs(low - lag(close, 1) OVER (ORDER BY ts))) as tr,
                    case when close > lag(close, 1) OVER (ORDER BY ts) then close - lag(close, 1) OVER (ORDER BY ts) else 0 end as gain,
                    case when close < lag(close, 1) OVER (ORDER BY ts) then lag(close, 1) OVER (ORDER BY ts) - close else 0 end as loss,
                    max(high) OVER (ORDER BY ts ROWS BETWEEN 20 PRECEDING AND 1 PRECEDING) as prev_max,
                    min(low) OVER (ORDER BY ts ROWS BETWEEN 20 PRECEDING AND 1 PRECEDING) as prev_min
                FROM read_parquet('{pf.as_posix()}')
                ORDER BY ts
            ),
            calc AS (
                SELECT 
                    symbol, timeframe, ts, open, high, low, close, volume,
                    ema_20, ema_50, ema_200,
                    avg(tr) OVER (ORDER BY ts ROWS BETWEEN 13 PRECEDING AND CURRENT ROW) as atr_14,
                    100.0 - (100.0 / (1.0 + (avg(gain) OVER (ORDER BY ts ROWS BETWEEN 13 PRECEDING AND CURRENT ROW) / 
                        nullif(avg(loss) OVER (ORDER BY ts ROWS BETWEEN 13 PRECEDING AND CURRENT ROW), 0.0000001)))) as rsi_14,
                    case 
                        when close > ema_50 and ema_20 > ema_50 then 1
                        when close < ema_50 and ema_20 < ema_50 then -1
                        else 0
                    end as regime,
                    case when close >= prev_max then 1 else 0 end as is_breakout_h,
                    case when close <= prev_min then 1 else 0 end as is_breakout_l
                FROM raw
            )
            SELECT 
                symbol, timeframe, ts, open, high, low, close, volume,
                ema_20, ema_50, ema_200,
                coalesce(rsi_14, 50.0),
                coalesce(atr_14, 0.001),
                regime,
                is_breakout_h,
                is_breakout_l
            FROM calc;
            """
            conn.execute(sql)
            loaded = conn.execute(f"SELECT count(*) FROM full_candles WHERE symbol = '{sym}'").fetchone()[0]
            print(f"    Loaded {loaded:,} feature rows for {sym}!")
        except Exception as e:
            print(f"    Error processing {sym}: {e}")

    total_rows = conn.execute("SELECT count(*) FROM full_candles").fetchone()[0]
    print(f"\n[COMPLETE] Database total rows: {total_rows:,}")

def print_memory_size():
    total_bytes = sum(f.stat().st_size for f in MEMORY_DIR.rglob("*") if f.is_file())
    gb = total_bytes / (1024 * 1024 * 1024)
    mb = total_bytes / (1024 * 1024)
    print("\n" + "=" * 65)
    print(f"   FINAL TRADING MEMORY TOTAL DISK USAGE: {gb:.2f} GB ({mb:,.1f} MB)")
    print("=" * 65)
    for sub in [RAW_DIR, GOLD_DIR, FEATURES_DIR, SETUPS_DIR, VECTOR_DIR]:
        sub_b = sum(f.stat().st_size for f in sub.rglob("*") if f.is_file())
        print(f"  • {sub.name:<15}: {sub_b / (1024*1024):8.1f} MB ({sub_b / (1024*1024*1024):.2f} GB)")

if __name__ == "__main__":
    download_all_raw()
    build_5gb_feature_database()
    print_memory_size()
