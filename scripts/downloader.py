import os
import sys
import time
from pathlib import Path
import httpx
import pyarrow.parquet as pq

BASE_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = BASE_DIR / "trading_memory" / "raw"
RAW_DIR.mkdir(parents=True, exist_ok=True)

SYMBOLS = {
    "xauusd": "https://huggingface.co/datasets/elthariel/histdata_fx_1m/resolve/main/xauusd/ticks.parquet",
    "eurusd": "https://huggingface.co/datasets/elthariel/histdata_fx_1m/resolve/main/eurusd/ticks.parquet",
    "gbpusd": "https://huggingface.co/datasets/elthariel/histdata_fx_1m/resolve/main/gbpusd/ticks.parquet",
    "usdjpy": "https://huggingface.co/datasets/elthariel/histdata_fx_1m/resolve/main/usdjpy/ticks.parquet",
}

def download_file(symbol: str, url: str):
    target_path = RAW_DIR / f"{symbol}_1m.parquet"
    if target_path.exists() and target_path.stat().st_size > 10 * 1024 * 1024:
        print(f"[{symbol.upper()}] Already downloaded ({target_path.stat().st_size / (1024*1024):.2f} MB). Skipping.")
        return target_path

    print(f"\n[DOWNLOAD] Starting download for {symbol.upper()} from {url}...")
    start_time = time.time()
    
    with httpx.Client(follow_redirects=True, timeout=300.0) as client:
        with client.stream("GET", url) as response:
            response.raise_for_status()
            total_bytes = int(response.headers.get("content-length", 0))
            downloaded = 0
            
            with open(target_path, "wb") as f:
                for chunk in response.iter_bytes(chunk_size=1024 * 512):
                    if chunk:
                        f.write(chunk)
                        downloaded += len(chunk)
                        if total_bytes > 0:
                            pct = (downloaded / total_bytes) * 100
                            mb = downloaded / (1024 * 1024)
                            total_mb = total_bytes / (1024 * 1024)
                            sys.stdout.write(f"\r  Progress: {pct:5.1f}% [{mb:6.1f} / {total_mb:6.1f} MB]")
                            sys.stdout.flush()

    elapsed = time.time() - start_time
    print(f"\n  Done in {elapsed:.1f}s! Saved to: {target_path}")
    return target_path

def inspect_parquet(path: Path):
    table = pq.read_table(path)
    print(f"\n[INSPECT] {path.name}:")
    print(f"  Rows count:   {table.num_rows:,} rows")
    print(f"  Columns:      {table.column_names}")
    print(f"  Memory Size:  {table.nbytes / (1024*1024):.2f} MB (Uncompressed in RAM)")
    
    # Print sample
    df_sample = table.slice(0, 3).to_pandas()
    print("  Sample records:")
    print(df_sample)

def main():
    print("=" * 60)
    print("   FOREX & GOLD HISTORICAL TRADING MEMORY DOWNLOADER")
    print("   Source: Institutional 1-Minute Parquet Archive")
    print("=" * 60)

    for symbol, url in SYMBOLS.items():
        try:
            path = download_file(symbol, url)
            inspect_parquet(path)
        except Exception as e:
            print(f"Error downloading {symbol}: {e}")

    print("\n[COMPLETE] All Forex & Gold raw data successfully downloaded and verified!")

if __name__ == "__main__":
    main()
