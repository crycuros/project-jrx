import asyncio
import datetime
import time
from pathlib import Path
import duckdb
import httpx
import pandas as pd

BASE_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = BASE_DIR / "trading_memory" / "raw"
PARQUET_PATH = RAW_DIR / "xauusd_1m.parquet"


async def fetch_chunk(client: httpx.AsyncClient, sem: asyncio.Semaphore, start_ms: int, end_ms: int):
    url = f"https://api.binance.com/api/v3/klines?symbol=PAXGUSDT&interval=1m&startTime={start_ms}&endTime={end_ms}&limit=1000"
    async with sem:
        for attempt in range(4):
            try:
                resp = await client.get(url, timeout=20.0)
                if resp.status_code == 200:
                    return resp.json()
                await asyncio.sleep(0.5 * (attempt + 1))
            except Exception:
                await asyncio.sleep(0.5 * (attempt + 1))
    return []


async def download_2025_2026_gold():
    t0 = time.time()

    start_dt = datetime.datetime(2025, 3, 22, 5, 0, tzinfo=datetime.timezone.utc)
    end_dt = datetime.datetime.now(datetime.timezone.utc)

    start_ms = int(start_dt.timestamp() * 1000)
    end_ms = int(end_dt.timestamp() * 1000)
    step_ms = 1000 * 60 * 1000  # 1000 minutes per chunk

    tasks = []
    sem = asyncio.Semaphore(16)
    limits = httpx.Limits(max_keepalive_connections=20, max_connections=30)

    async with httpx.AsyncClient(limits=limits) as client:
        cur = start_ms
        while cur < end_ms:
            nxt = min(cur + step_ms - 1, end_ms)
            tasks.append(fetch_chunk(client, sem, cur, nxt))
            cur += step_ms

        print(f"Fetching {len(tasks)} chunks of 1-minute Gold data (2025-03-22 to {end_dt.strftime('%Y-%m-%d')})...")
        results = await asyncio.gather(*tasks)

    rows = []
    for chunk in results:
        for r in chunk:
            rows.append((
                pd.Timestamp(r[0], unit="ms"),
                float(r[1]),
                float(r[2]),
                float(r[3]),
                float(r[4]),
                float(r[5]),
            ))

    print(f"Downloaded {len(rows):,} new 1-minute Gold candles in {time.time() - t0:.1f}s.")
    if not rows:
        return

    df_new = pd.DataFrame(rows, columns=["ts", "open", "high", "low", "close", "volume"])
    df_new["ts"] = pd.to_datetime(df_new["ts"]).dt.tz_localize("UTC")

    # Load existing parquet via pandas/pyarrow
    df_old = pd.read_parquet(PARQUET_PATH)
    df_old["ts"] = pd.to_datetime(df_old["ts"], utc=True)

    df_combined = pd.concat([df_old, df_new], ignore_index=True)
    df_combined.drop_duplicates(subset=["ts"], keep="last", inplace=True)
    df_combined.sort_values("ts", inplace=True)
    df_combined.reset_index(drop=True, inplace=True)

    tmp_path = RAW_DIR / "xauusd_1m_updated.parquet"
    df_combined.to_parquet(tmp_path, index=False)
    tmp_path.replace(PARQUET_PATH)
    print(f"Updated {PARQUET_PATH.name}: Total rows = {len(df_combined):,} | Range: {df_combined['ts'].min()} -> {df_combined['ts'].max()}")


if __name__ == "__main__":
    asyncio.run(download_2025_2026_gold())
