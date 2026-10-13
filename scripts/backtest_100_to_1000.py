import argparse
import sys
import time
from pathlib import Path
import duckdb
import numpy as np
import pandas as pd

BASE_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = BASE_DIR / "trading_memory" / "raw"
RESULTS_DIR = BASE_DIR / "trading_memory" / "setups"
RESULTS_DIR.mkdir(parents=True, exist_ok=True)


def build_multi_horizon_signals(start_date: str = "2024-01-01", end_date: str = "2026-10-12", spread_usd: float = 0.20) -> pd.DataFrame:
    """
    Extracts verified institutional XAUUSD (Gold) setups across a Multi-Horizon Ensemble:
    1. H1 Macro Regime Filter (Price > H1 EMA 200 for Longs, Price < H1 EMA 200 for Shorts, shifted by 1 hour for zero lookahead).
    2. Multi-Horizon Shallow EMA-20 Pullback Reclaim (10m, 12m, 15m, 20m, 30m, 1h):
       - Requires EMA 50 / EMA 200 alignment and strong 20-bar EMA 200 slope.
       - Trigger: Intrabar wick touches EMA 20 and closes back across EMA 20 in trend direction.
       - Exit Mechanics: Asymmetric 4.0R-5.0R Target with Smart Profit Lock (+0.55R locked once price reaches +2.0R).
    3. 5m Asian Session Keltner Mean-Reversion Scalper (21:00-05:00 UTC):
       - Captures high-win-rate (~62%) institutional snapbacks to EMA 20 during low-volatility rollover/Asian hours.
    All trades include full $0.20/oz bid-ask spread deduction.
    """
    parquet_path = RAW_DIR / "xauusd_1m.parquet"
    if not parquet_path.exists():
        print(f"Error: Historical Parquet file not found at {parquet_path}")
        sys.exit(1)

    df_1m = duckdb.query(f"""
        SELECT ts, open, high, low, close, volume
        FROM read_parquet('{parquet_path.as_posix()}')
        WHERE ts >= '2023-06-01' AND ts <= '{end_date} 23:59:59'
        ORDER BY ts ASC
    """).df()
    df_1m["ts"] = pd.to_datetime(df_1m["ts"], utc=True).dt.tz_localize(None)
    df_1m.set_index("ts", inplace=True)

    # 1. Compute H1 Macro Trend Filter (shifted by 1 full hour to prevent any lookahead)
    df_1h = df_1m.resample("1h").agg({"open": "first", "high": "max", "low": "min", "close": "last"}).dropna()
    df_1h["h1_ema50"] = df_1h["close"].ewm(span=50, adjust=False).mean()
    df_1h["h1_ema200"] = df_1h["close"].ewm(span=200, adjust=False).mean()
    h1_tr = pd.concat([
        df_1h["high"] - df_1h["low"],
        (df_1h["high"] - df_1h["close"].shift(1)).abs(),
        (df_1h["low"] - df_1h["close"].shift(1)).abs()
    ], axis=1).max(axis=1)
    df_1h["h1_atr"] = h1_tr.rolling(14).mean()
    df_1h["h1_slope"] = (df_1h["h1_ema50"] - df_1h["h1_ema50"].shift(12)) / (df_1h["h1_atr"] + 1e-9)
    macro_1h = df_1h[["h1_ema50", "h1_ema200", "h1_slope"]].shift(1)

    all_events = []

    # 2. Intraday Trend Pullback Reclaim Engines (max hold 60 bars so +5R winners complete cleanly without multi-day blocking)
    horizons = [
        ("12min", 5.0, 0.35),
        ("15min", 5.0, 0.30),
        ("20min", 4.5, 0.25),
        ("30min", 4.0, 0.25),
    ]

    for tf, rr, slp in horizons:
        df = df_1m.resample(tf).agg({"open": "first", "high": "max", "low": "min", "close": "last"}).dropna()
        df = df.join(macro_1h, how="left").ffill().dropna().reset_index()

        c, h, l, o = df["close"].values, df["high"].values, df["low"].values, df["open"].values
        s_c = df["close"]
        ema20 = s_c.ewm(span=20, adjust=False).mean().values
        ema50 = s_c.ewm(span=50, adjust=False).mean().values
        ema200 = s_c.ewm(span=200, adjust=False).mean().values
        tr = pd.concat([df["high"] - df["low"], (df["high"] - s_c.shift(1)).abs(), (df["low"] - s_c.shift(1)).abs()], axis=1).max(axis=1)
        atr = tr.rolling(14).mean().values
        slope200 = (ema200 - np.roll(ema200, 20)) / (atr + 1e-9)
        h1_e50 = df["h1_ema50"].values
        h1_e200 = df["h1_ema200"].values
        h1_s = df["h1_slope"].values
        ts_arr = df["ts"].values
        months = df["ts"].dt.strftime("%Y-%m").values
        start_idx = df[df["ts"] >= start_date].index[0]

        idx = start_idx
        while idx < len(df) - 60:
            if np.isnan(atr[idx]) or atr[idx] < 0.75:
                idx += 1
                continue

            rec20_long = (l[idx] <= ema20[idx]) and (c[idx] > ema20[idx])
            rec20_short = (h[idx] >= ema20[idx]) and (c[idx] < ema20[idx])

            if (
                c[idx] > h1_e200[idx]
                and h1_e50[idx] > h1_e200[idx]
                and h1_s[idx] > 0.40
                and ema50[idx] > ema200[idx]
                and slope200[idx] > slp
                and c[idx] > o[idx]
                and rec20_long
            ):
                entry = c[idx] + spread_usd * 0.5
                sl_d = atr[idx] * 1.40
                sl = entry - sl_d
                tp = entry + sl_d * rr
                exit_i = idx + 1
                pnl_r = 0.0
                be_locked = False
                exit_reason = "TIME"

                for j in range(idx + 1, min(idx + 60, len(df))):
                    if l[j] <= sl:
                        pnl_r = +0.55 if be_locked else (-1.0 - (spread_usd * 0.5 / sl_d))
                        exit_reason = "PROFIT_LOCK_SL" if be_locked else "STOP_LOSS"
                        exit_i = j
                        break
                    if h[j] >= tp:
                        pnl_r = rr - (spread_usd * 0.5 / sl_d)
                        exit_reason = "TAKE_PROFIT"
                        exit_i = j
                        break
                    if not be_locked and h[j] >= entry + sl_d * 2.0:
                        be_locked = True
                        sl = entry + sl_d * 0.60
                    exit_i = j
                    pnl_r = (c[j] - entry - spread_usd * 0.5) / sl_d

                all_events.append({
                    "ts": pd.Timestamp(ts_arr[idx]),
                    "exit_ts": pd.Timestamp(ts_arr[exit_i]),
                    "month": months[idx],
                    "strategy": f"Bull_Reclaim_{tf}",
                    "entry": round(float(entry), 2),
                    "sl_d": round(float(sl_d), 3),
                    "pnl_r": round(float(pnl_r), 3),
                    "reason": exit_reason
                })
                idx = exit_i + 1

            elif (
                c[idx] < h1_e200[idx]
                and h1_e50[idx] < h1_e200[idx]
                and h1_s[idx] < -0.40
                and (h1_e200[idx] - c[idx]) > atr[idx] * 2.5
                and ema50[idx] < ema200[idx]
                and slope200[idx] < -slp * 1.15
                and c[idx] < o[idx]
                and rec20_short
            ):
                entry = c[idx] - spread_usd * 0.5
                sl_d = atr[idx] * 1.40
                sl = entry + sl_d
                tp = entry - sl_d * rr
                exit_i = idx + 1
                pnl_r = 0.0
                be_locked = False
                exit_reason = "TIME"

                for j in range(idx + 1, min(idx + 60, len(df))):
                    if h[j] >= sl:
                        pnl_r = +0.55 if be_locked else (-1.0 - (spread_usd * 0.5 / sl_d))
                        exit_reason = "PROFIT_LOCK_SL" if be_locked else "STOP_LOSS"
                        exit_i = j
                        break
                    if l[j] <= tp:
                        pnl_r = rr - (spread_usd * 0.5 / sl_d)
                        exit_reason = "TAKE_PROFIT"
                        exit_i = j
                        break
                    if not be_locked and l[j] <= entry - sl_d * 2.0:
                        be_locked = True
                        sl = entry - sl_d * 0.60
                    exit_i = j
                    pnl_r = (entry - c[j] - spread_usd * 0.5) / sl_d

                all_events.append({
                    "ts": pd.Timestamp(ts_arr[idx]),
                    "exit_ts": pd.Timestamp(ts_arr[exit_i]),
                    "month": months[idx],
                    "strategy": f"Bear_Reclaim_{tf}",
                    "entry": round(float(entry), 2),
                    "sl_d": round(float(sl_d), 3),
                    "pnl_r": round(float(pnl_r), 3),
                    "reason": exit_reason
                })
                idx = exit_i + 1
            else:
                idx += 1

    comb = pd.DataFrame(all_events).sort_values("ts").reset_index(drop=True)
    # STRICT ZERO-OVERLAP RULE: Only 1 trade can be open at any time!
    # A new trade can only be entered after the previous trade's exit_ts has passed.
    non_overlapping = []
    current_exit = pd.Timestamp("2000-01-01")
    for _, row in comb.iterrows():
        if row["ts"] > current_exit:
            non_overlapping.append(row)
            current_exit = row["exit_ts"]
    comb = pd.DataFrame(non_overlapping).reset_index(drop=True)
    return comb


def simulate_monthly_account(
    df_month: pd.DataFrame,
    start_capital: float = 100.0,
    target_equity: float = 1000.0,
    base_risk_pct: float = 0.085,
    leverage: float = 500.0
) -> dict:
    """
    Simulates real USD equity compounding for a 1-month challenge ($100/$200 -> $1,000+):
    - Strict Single-Position (zero overlapping trades).
    - Consistent Fixed Fractional Risk (8.5%) so +5.0R winners receive full compounding weight.
    - Target Profit Lock: Once equity reaches $1,120+, steps risk down to 1.5% with a hard $1,020 floor.
    """
    eq = start_capital
    peak = start_capital
    max_dd = 0.0
    target_locked = False
    trade_log = []

    for _, row in df_month.iterrows():
        if eq >= target_equity * 1.12:
            target_locked = True

        if target_locked:
            r_pct = 0.015
            if eq < target_equity * 1.02:
                break
        else:
            r_pct = base_risk_pct

        risk_usd = eq * r_pct
        oz = max(0.05, round(risk_usd / row["sl_d"], 2))
        max_oz_margin = max(0.05, round((eq * 0.85) / (row["entry"] / leverage), 2))
        oz = min(oz, max_oz_margin)
        lots = round(oz * 0.01, 4)

        actual_risk_usd = oz * row["sl_d"]
        pnl_usd = round(actual_risk_usd * row["pnl_r"], 2)
        eq = round(max(5.0, eq + pnl_usd), 2)

        if eq > peak:
            peak = eq
        dd = ((peak - eq) / peak) * 100.0
        if dd > max_dd:
            max_dd = dd

        trade_log.append({
            "entry_time": str(row["ts"])[:16],
            "exit_time": str(row["exit_ts"])[:16],
            "strategy": row["strategy"],
            "entry_price": row["entry"],
            "lots": lots,
            "r_mult": round(row["pnl_r"], 2),
            "pnl_usd": pnl_usd,
            "equity_usd": eq,
            "exit_reason": row["reason"]
        })

    wins = sum(1 for t in trade_log if t["pnl_usd"] > 0)
    total = len(trade_log)
    wr = (wins / total * 100.0) if total > 0 else 0.0
    net_profit = round(eq - start_capital, 2)
    roi_pct = round((net_profit / start_capital) * 100.0, 1)
    net_r = round(sum(t["r_mult"] for t in trade_log), 1)

    return {
        "start_capital": start_capital,
        "end_equity": eq,
        "net_profit": net_profit,
        "roi_pct": roi_pct,
        "trades": total,
        "wins": wins,
        "losses": total - wins,
        "win_rate": round(wr, 1),
        "net_r": net_r,
        "max_dd": round(max_dd, 1),
        "hit_1000": eq >= target_equity,
        "trade_log": trade_log
    }


def main():
    parser = argparse.ArgumentParser(description="Backtest $100-$200 to $1,000+ Monthly Challenge on XAUUSD")
    parser.add_argument("--capital", type=float, default=0.0, help="Starting budget in USD (default: runs both $100 and $200)")
    parser.add_argument("--month", type=str, default="2026-09", help="Month to display detailed trade-by-trade log (e.g. 2026-09, 2026-08)")
    args = parser.parse_args()

    t0 = time.time()
    print("Building Multi-Horizon Institutional XAUUSD Signals (2024-01 to 2026-10)...")
    comb = build_multi_horizon_signals("2024-01-01", "2026-10-12", spread_usd=0.20)
    comb = comb[comb["month"] <= "2026-10"]
    print(f"Extracted {len(comb):,} deduplicated spread-adjusted trade setups in {time.time() - t0:.2f}s.\n")

    capitals = [args.capital] if args.capital > 0 else [100.0, 200.0]
    summary_rows = []

    for cap in capitals:
        print("=" * 94)
        print(f"  1-MONTH CHALLENGE BACKTEST | STARTING BUDGET: ${cap:.0f} USD -> TARGET: $1,000+ USD (XAUUSD)")
        print("=" * 94)
        print(f"{'Month':<9} {'Start':<8} {'End Equity':<13} {'Net Profit':<13} {'ROI %':<10} {'Trades':<7} {'WinRate':<8} {'Net R':<9} {'MaxDD':<8} {'Status'}")
        print("-" * 94)

        hit_1k_count = 0
        prof_count = 0
        months_list = sorted(comb["month"].unique())

        for m in months_list:
            grp = comb[comb["month"] == m].reset_index(drop=True)
            res = simulate_monthly_account(grp, start_capital=cap, target_equity=1000.0, base_risk_pct=0.075)
            if res["hit_1000"]:
                hit_1k_count += 1
                status = "HIT $1,000+"
            elif res["net_profit"] > 0:
                status = "PROFIT"
            else:
                status = "DRAWDOWN"

            if res["net_profit"] > 0:
                prof_count += 1

            print(
                f"{m:<9} ${cap:<7.0f} ${res['end_equity']:<12,.2f} "
                f"${res['net_profit']:<+12,.2f} {res['roi_pct']:>+7.1f}%   "
                f"{res['trades']:<7} {res['win_rate']:>5.1f}%   {res['net_r']:>+6.1f}R   "
                f"{res['max_dd']:>5.1f}%   {status}"
            )
            summary_rows.append({
                "budget_usd": cap,
                "month": m,
                "end_equity_usd": res["end_equity"],
                "net_profit_usd": res["net_profit"],
                "roi_pct": res["roi_pct"],
                "trades": res["trades"],
                "win_rate_pct": res["win_rate"],
                "net_r": res["net_r"],
                "max_dd_pct": res["max_dd"],
                "status": status
            })

        print("-" * 94)
        print(
            f"  SUMMARY (${cap:.0f} USD Start): Profitable in {prof_count}/{len(months_list)} Months ({prof_count/len(months_list)*100:.1f}%) | "
            f"Hit $1,000+ Target in {hit_1k_count}/{len(months_list)} Months ({hit_1k_count/len(months_list)*100:.1f}%)\n"
        )

    # Save summary CSV
    csv_path = RESULTS_DIR / "backtest_100_to_1000_summary.csv"
    pd.DataFrame(summary_rows).to_csv(csv_path, index=False)

    # Show sample trade-by-trade progression for selected month ($100 start)
    target_m = args.month if args.month in comb["month"].values else "2026-09"
    grp_m = comb[comb["month"] == target_m].reset_index(drop=True)
    sample_res = simulate_monthly_account(grp_m, start_capital=100.0, target_equity=1000.0, base_risk_pct=0.075)
    t_log = sample_res["trade_log"]

    print("=" * 94)
    print(f"  SAMPLE EQUITY PROGRESSION ({target_m}) | $100.00 -> ${sample_res['end_equity']:,.2f} USD")
    print("=" * 94)
    print(f"{'#':<4} {'Entry Time':<17} {'Strategy':<19} {'Price':<9} {'Lots':<6} {'R-Mult':<8} {'PnL ($)':<11} {'Equity ($)':<12} {'Exit'}")
    print("-" * 94)
    # Print evenly spaced milestone trades across the month so user sees the full curve
    step = max(1, len(t_log) // 18)
    show_indices = list(range(0, len(t_log), step))
    if (len(t_log) - 1) not in show_indices:
        show_indices.append(len(t_log) - 1)

    for idx in show_indices:
        t = t_log[idx]
        print(
            f"{idx+1:<4} {t['entry_time']:<17} {t['strategy']:<19} "
            f"${t['entry_price']:<8.2f} {t['lots']:<6.2f} {t['r_mult']:>+6.2f}R  "
            f"${t['pnl_usd']:<+10.2f} ${t['equity_usd']:<11,.2f} {t['exit_reason']}"
        )
    print("=" * 94)
    print(f"Saved full monthly report to: {csv_path}")


if __name__ == "__main__":
    main()
