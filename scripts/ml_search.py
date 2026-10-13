"""
Parameter search for the gold/FX ML strategy.

The only honest way to search parameters: every candidate is scored purely on
its out-of-sample folds, and because searching many configs inflates the best
result, the winner must also clear a multiple-testing correction (Bailey &
Lopez de Prado). A config that only looks good because it was one of hundreds
is a mirage, and this script is built to expose exactly that.

Usage:
    python scripts/ml_search.py                 # default grid
    python scripts/ml_search.py --quick          # fewer configs
"""

from __future__ import annotations

import argparse
import itertools
import math
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ml_research import (  # noqa: E402
    COMMISSION_FRAC, ROOT, SPREAD_FRAC, build_features, deflated_sharpe,
    purged_walk_forward, triple_barrier, Stats,
)

RAW = ROOT / "trading_memory" / "raw"
GOLD_5M = ROOT / "trading_memory" / "gold_archive" / "XAU_5m_data.jsonl"
MULTIPLIER = 100.0
MIN_STAKE = 1.0
MAX_STAKE_FRAC = 0.10

SYMBOLS = {
    "XAU_USD": GOLD_5M,
    "EUR_USD": RAW / "eurusd_1m.parquet",
    "GBP_USD": RAW / "gbpusd_1m.parquet",
}


def load_any(path: Path, timeframe: str, max_bars: int = 600_000) -> pd.DataFrame:
    """Load candles from either the jsonl archive or a parquet raw file, then
    resample to `timeframe`. Keeps memory bounded by `max_bars`."""
    tf_s = {"1m": 60, "5m": 300, "15m": 900, "30m": 1800, "60m": 3600}[timeframe]

    if path.suffix == ".parquet":
        df = pd.read_parquet(path, columns=["ts", "open", "high", "low", "close", "volume"])
        df["ts"] = pd.to_datetime(df["ts"], utc=True)
        df = df.rename(columns=str.lower)
    else:
        df = pd.read_json(path, lines=True, convert_dates={"Date": "%Y.%m.%d %H:%M"})
        df = df.rename(columns=str.title)
        df = df[["Date", "Open", "High", "Low", "Close", "Volume"]]
        df.columns = ["ts", "open", "high", "low", "close", "volume"]
        df["ts"] = pd.to_datetime(df["ts"], utc=True)

    df = df.dropna().sort_values("ts").drop_duplicates("ts")
    if tf_s > 60:
        df = (df.set_index("ts")
                .resample(f"{tf_s}s")
                .agg({"open": "first", "high": "max", "low": "min",
                      "close": "last", "volume": "sum"})
                .dropna()
                .reset_index())
    df = df.reset_index(drop=True)
    if len(df) > max_bars:
        df = df.iloc[-max_bars:].reset_index(drop=True)
    for c in ["open", "high", "low", "close", "volume"]:
        df[c] = df[c].astype("float64")
    return df


def atr_pct_of(df: pd.DataFrame) -> pd.Series:
    tr = pd.concat([
        df["high"] - df["low"],
        (df["high"] - df["close"].shift()).abs(),
        (df["low"] - df["close"].shift()).abs(),
    ], axis=1).max(axis=1)
    return tr.rolling(14).mean() / df["close"]


def evaluate(clf_factory, X, y, r_true, t_hit, ts, vertical: int,
             thr: float, folds: int) -> tuple[Stats, list, list]:
    from sklearn.metrics import roc_auc_score

    oos = Stats()
    trades = []
    aucs = []
    n = len(y)
    for f in purged_walk_forward(n, vertical, folds=folds):
        mtr = y[f.tr_lo:f.tr_hi] != 0
        mte = y[f.te_lo:f.te_hi] != 0
        if mtr.sum() < 500 or mte.sum() < 200:
            continue
        clf = clf_factory()
        clf.fit(X[f.tr_lo:f.tr_hi][mtr], (y[f.tr_lo:f.tr_hi][mtr] > 0).astype(int))
        p_te_full = clf.predict_proba(X[f.te_lo:f.te_hi])[:, 1]
        yt = y[f.te_lo:f.te_hi]
        if len(np.unique(yt[mte])) == 2:
            aucs.append(roc_auc_score((yt[mte] > 0).astype(int), p_te_full[mte]))

        # Walk the test window sequentially, one position at a time.
        idx = 0
        n_te = len(y[f.te_lo:f.te_hi])
        while idx < n_te:
            gi = f.te_lo + idx
            pi = p_te_full[idx]
            if pi >= thr:
                oos.add(float(r_true[gi]))
                trades.append((float(r_true[gi]), +1, ts[gi]))
                idx += max(int(t_hit[gi]), 1)
            elif pi <= 1.0 - thr:
                oos.add(float(-r_true[gi]))
                trades.append((float(r_true[gi]), -1, ts[gi]))
                idx += max(int(t_hit[gi]), 1)
            else:
                idx += 1
    return oos, trades, aucs


def account_sim(trades, start: float, risk_frac: float, eff_stop: float,
                max_daily_loss_pct=0.05, max_consec=5, max_dd_pct=0.30):
    bal = start
    peak = start
    trades_done = 0
    day_pnl = 0.0
    consec = 0
    cur_day = None
    max_dd_seen = 0.0
    halted = None

    for r, side, ts in trades:
        if halted:
            break
        if bal < MIN_STAKE:
            halted = "bankrupt"
            break
        day = int(ts // 86_400_000)
        if day != cur_day:
            cur_day, day_pnl, consec = day, 0.0, 0
        dd = 100.0 * (peak - bal) / peak if peak > 0 else 0.0
        if dd >= max_dd_pct * 100:
            halted = f"drawdown {dd:.0f}%"
            break
        if day_pnl <= -peak * max_daily_loss_pct or consec >= max_consec:
            continue
        stake = (bal * risk_frac) / (eff_stop * MULTIPLIER)
        stake = min(stake, bal, bal * MAX_STAKE_FRAC)
        stake = math.floor(max(stake, MIN_STAKE))
        if stake < MIN_STAKE:
            continue
        pnl = (r * eff_stop * side) * MULTIPLIER * stake - stake * COMMISSION_FRAC
        bal += pnl
        trades_done += 1
        day_pnl += pnl
        consec = consec + 1 if pnl < 0 else 0
        peak = max(peak, bal)
        max_dd_seen = max(max_dd_seen, 100.0 * (peak - bal) / peak if peak > 0 else 0)
    return bal, trades_done, max_dd_seen, halted


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--balance", type=float, default=200.0)
    a = ap.parse_args()

    from sklearn.ensemble import HistGradientBoostingClassifier

    t0 = time.time()
    tf = "15m"
    folds = 4
    grid = {
        "pt": [0.004, 0.006],
        "sl": [0.004, 0.006],
        "vertical": [24, 96],
        "thr": [0.53, 0.57],
    }
    if a.quick:
        grid = {k: v[:1] for k, v in grid.items()}
        grid["vertical"] = [48]
    combos = list(itertools.product(*grid.values()))
    pt_l, sl_l, v_l, thr_l = grid["pt"], grid["sl"], grid["vertical"], grid["thr"]
    print(f"grid: {len(combos)} configs x {len(SYMBOLS)} symbols = {len(combos)*len(SYMBOLS)} runs")

    results = []
    for sym, path in SYMBOLS.items():
        if not path.exists():
            print(f"  skip {sym}: {path.name} missing")
            continue
        df = load_any(path, tf)
        feats = build_features(df)
        atr = atr_pct_of(df)
        n_raw = len(df)
        print(f"\n  {sym}: {n_raw:,} {tf} bars  {df.ts.iloc[0].date()} .. {df.ts.iloc[-1].date()}")

        for pt, sl, vertical, thr in combos:
            lab = triple_barrier(df, atr, pt, sl, vertical)
            data = feats.join(lab)
            data["ret_r"] = data["ret_r"].clip(-3, 3)
            data = data.replace([np.inf, -np.inf], np.nan)
            keep = data.index[data.notna().all(axis=1)]
            data = data.loc[keep].reset_index(drop=True)
            ts = df.loc[keep, "ts"].astype("int64").to_numpy()

            X = data.drop(columns=["label", "t_hit", "ret_r"]).to_numpy(np.float32)
            y = data["label"].to_numpy()
            r = data["ret_r"].to_numpy(np.float32)
            th = data["t_hit"].to_numpy()

            oos, trades, aucs = evaluate(
                lambda: HistGradientBoostingClassifier(
                    max_depth=5, max_iter=150, learning_rate=0.06,
                    min_samples_leaf=200, l2_regularization=1.0, random_state=42),
                X, y, r, th, ts, vertical, thr, folds)

            auc = float(np.mean(aucs)) if aucs else 0.5
            # Sharpe on per-trade returns
            sharpe = 0.0
            if oos.trades > 10:
                arr = np.array(oos.curve)
                s = np.diff(arr).std(ddof=1)
                if s > 1e-12:
                    tpy = 252 * (24 * 60 / (vertical * (15 if tf == "15m" else 1)))
                    sharpe = np.diff(arr).mean() / s * math.sqrt(tpy)

            eff_stop = max(sl, 2 * (SPREAD_FRAC + COMMISSION_FRAC))
            bal, n_done, dd, halt = account_sim(trades, a.balance, 0.005, eff_stop)
            ret_pct = 100.0 * (bal - a.balance) / a.balance

            results.append(dict(sym=sym, pt=pt, sl=sl, vertical=vertical, thr=thr,
                                auc=auc, trades=oos.trades, exp=oos.expectancy,
                                pf=oos.pf, sharpe=sharpe, bal=bal, ret=ret_pct,
                                dd=dd, halt=halt))

            print(f"    pt{pt} sl{sl} v{vertical} thr{thr}  AUC {auc:.4f}  "
                  f"n={oos.trades:<6,} exp {oos.expectancy:+.4f}  PF {oos.pf:.3f}  "
                  f"bal ${bal:,.2f} ({ret_pct:+.1f}%) dd {dd:.0f}%{(' HALT:'+halt) if halt else ''}")

    if not results:
        return
    trials = len(results)
    print(f"\n{'='*94}")
    print(f"  RANKING (out-of-sample only, {trials} configs tried)")
    print(f"{'='*94}")
    results.sort(key=lambda r: r["bal"], reverse=True)
    print(f"  {'sym':<9}{'pt':<7}{'sl':<7}{'v':<5}{'thr':<7}{'AUC':<9}"
          f"{'trades':<9}{'exp':<10}{'PF':<8}{'$end':<11}{'ret%':<9}{'DD':<7}")
    for r in results[:15]:
        print(f"  {r['sym']:<9}{r['pt']:<7}{r['sl']:<7}{r['vertical']:<5}{r['thr']:<7}"
              f"{r['auc']:<9.4f}{r['trades']:<9,}{r['exp']:<+10.4f}{r['pf']:<8.3f}"
              f"${r['bal']:<10,.2f}{r['ret']:<+9.1f}{r['dd']:<7.0f}")

    # Multiple-testing correction on the top configs
    print(f"\n{'='*94}")
    print("  DEFLATION (multiple-testing correction)")
    print(f"{'='*94}")
    top = results[0]
    dsr = deflated_sharpe(top["sharpe"], trials, top["trades"])
    print(f"  best config: {top['sym']} pt{top['pt']} sl{top['sl']} v{top['vertical']} thr{top['thr']}")
    print(f"  Sharpe {top['sharpe']:+.2f} across {trials} trials, {top['trades']:,} trades")
    print(f"  deflated p-value: {dsr:.4f}")
    # A negative Sharpe is never significant, regardless of the p-value.
    if top["sharpe"] <= 0:
        print("  ✗ WORST config still has a negative Sharpe — nothing here is an edge.")
    elif dsr < 0.05:
        print(f"  ✓ significant even after correcting for {trials} trials")
    else:
        print(f"  ✗ NOT significant after correcting for {trials} trials")
        print("    The best result is likely selection bias, not a real edge.")
    print(f"\n  elapsed {time.time()-t0:.0f}s")


if __name__ == "__main__":
    main()