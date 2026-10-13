"""
Prop-firm backtest: can this strategy pass a funded-trader challenge?

This is the question that actually matters. In a prop firm the capital is not
yours, so the $100-$200 account ceiling stops being the binding constraint.
What binds instead is the firm's risk rules - typically a 10% maximum drawdown
and a 5% daily loss limit - against which the strategy either survives or
fails on day 3.

Two phases are simulated:
  1. CHALLENGE - hit the profit target without breaching the drawdown/daily
     limits. Fail = account blown, try again.
  2. FUNDED  - continue trading the same rules, take the profit split.
     This is where real income appears.

Usage:
    python scripts/ml_prop.py --capital 50000 --target 8 --maxdd 10
"""

from __future__ import annotations

import argparse
import math
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ml_research import (  # noqa: E402
    COMMISSION_FRAC, SPREAD_FRAC, build_features, load_gold, triple_barrier,
    purged_walk_forward,
)

MULTIPLIER = 100.0
MIN_STAKE = 1.0


# ══════════════════════════════════════════════════════════════════════════
# Prop firm rules
# ══════════════════════════════════════════════════════════════════════════
@dataclass
class FirmRules:
    capital: float = 50_000.0
    profit_target_pct: float = 8.0     # must reach this to pass
    max_dd_pct: float = 10.0           # account is blown if breached
    daily_loss_pct: float = 5.0        # per day, of starting capital
    trailing_dd: bool = False          # many firms use trailing instead of static
    min_days: int = 4
    split_pct: float = 0.80            # your share of profits once funded

    @property
    def max_dd_abs(self) -> float:
        return self.capital * self.max_dd_pct / 100.0

    @property
    def daily_loss_abs(self) -> float:
        return self.capital * self.daily_loss_pct / 100.0

    @property
    def target_abs(self) -> float:
        return self.capital * self.profit_target_pct / 100.0


@dataclass
class SimResult:
    passed: bool = False
    fail_reason: str = ""
    end_equity: float = 0.0
    trades: int = 0
    days: int = 0
    worst_day: float = 0.0
    min_dd_used_pct: float = 0.0
    equity_curve: list = field(default_factory=list)


def simulate(trades, rules: FirmRules, risk_frac: float, eff_stop: float,
             until_target: bool = True) -> SimResult:
    """Replay trades under firm rules. `trades` = (r, side, ts, day)."""
    res = SimResult()
    equity = rules.capital
    start_equity = rules.capital
    peak = rules.capital
    dd_floor = rules.capital - rules.max_dd_abs   # static rule
    cur_day = None
    day_start = equity
    days_traded = 0

    for r, side, ts, day in trades:
        if res.passed and until_target:
            break
        if day != cur_day:
            cur_day = day
            day_start = equity
            days_traded += 1
            # Trailing drawdown re-anchors to the running peak.
            if rules.trailing_dd:
                dd_floor = max(dd_floor, peak - rules.max_dd_abs)

        # Hard account-level breach.
        if equity < dd_floor:
            res.fail_reason = f"max drawdown breached (equity ${equity:,.0f})"
            res.end_equity = equity
            res.trades = res.trades
            res.days = days_traded
            res.equity_curve = list(res.equity_curve)
            res.min_dd_used_pct = 100.0 * (peak - equity) / peak if peak else 0.0
            return res

        day_pnl = equity - day_start
        if day_pnl <= -rules.daily_loss_abs:
            continue   # stop for the day, try again next session

        stake = (equity * risk_frac) / (eff_stop * MULTIPLIER)
        stake = min(stake, equity * 0.10, rules.capital)
        stake = math.floor(max(stake, MIN_STAKE))
        if stake < MIN_STAKE or equity < MIN_STAKE:
            continue

        pnl = (r * eff_stop * side) * MULTIPLIER * stake - stake * COMMISSION_FRAC
        equity += pnl
        res.trades += 1
        res.equity_curve.append(equity)
        peak = max(peak, equity)
        res.worst_day = min(res.worst_day, day_pnl)
        res.min_dd_used_pct = max(res.min_dd_used_pct,
                                  100.0 * (peak - equity) / peak if peak else 0.0)

        if until_target and equity - start_equity >= rules.target_abs:
            res.passed = True
            break

    res.end_equity = equity
    res.days = days_traded
    if not res.passed and not res.fail_reason:
        res.fail_reason = (f"target not reached (+${equity-start_equity:,.0f} of "
                           f"${rules.target_abs:,.0f} needed)")
    return res


# ══════════════════════════════════════════════════════════════════════════
def collect_oos_trades(timeframe: str, pt: float, sl: float, vertical: int,
                       thr: float, folds: int, max_bars: int):
    """Train walk-forward and return every out-of-sample trade, in time order."""
    from sklearn.ensemble import HistGradientBoostingClassifier

    df = load_gold(timeframe)
    if len(df) > max_bars:
        df = df.iloc[-max_bars:].reset_index(drop=True)
    feats = build_features(df)
    tr_all = pd.concat([
        df["high"] - df["low"],
        (df["high"] - df["close"].shift()).abs(),
        (df["low"] - df["close"].shift()).abs(),
    ], axis=1).max(axis=1)
    atr = tr_all.rolling(14).mean() / df["close"]
    lab = triple_barrier(df, atr, pt, sl, vertical)
    data = feats.join(lab)
    data["ret_r"] = data["ret_r"].clip(-3, 3)
    data = data.replace([np.inf, -np.inf], np.nan)
    keep = data.index[data.notna().all(axis=1)]
    data = data.loc[keep].reset_index(drop=True)
    # Epoch seconds. pandas stores datetimes at microsecond resolution here, so
    # normalise explicitly rather than assuming a fixed unit.
    tser = df["ts"]
    if tser.dt.tz is not None:
        tser = tser.dt.tz_localize(None)
    ts = tser.loc[keep].to_numpy(dtype="datetime64[s]").astype("int64")

    X = data.drop(columns=["label", "t_hit", "ret_r"]).to_numpy(np.float32)
    y = data["label"].to_numpy()
    r = data["ret_r"].to_numpy(np.float32)
    th = data["t_hit"].to_numpy()
    days = (ts // 86_400).astype(np.int64)   # ts is in seconds

    out = []
    for f in purged_walk_forward(len(y), vertical, folds=folds):
        mtr = y[f.tr_lo:f.tr_hi] != 0
        if mtr.sum() < 500:
            continue
        clf = HistGradientBoostingClassifier(
            max_depth=5, max_iter=150, learning_rate=0.06,
            min_samples_leaf=200, l2_regularization=1.0, random_state=42)
        clf.fit(X[f.tr_lo:f.tr_hi][mtr], (y[f.tr_lo:f.tr_hi][mtr] > 0).astype(int))
        p = clf.predict_proba(X[f.te_lo:f.te_hi])[:, 1]

        idx = 0
        n_te = len(y[f.te_lo:f.te_hi])
        while idx < n_te:
            gi = f.te_lo + idx
            pi = p[idx]
            if pi >= thr:
                out.append((float(r[gi]), +1, int(ts[gi]), int(days[gi])))
                idx += max(int(th[gi]), 1)
            elif pi <= 1.0 - thr:
                out.append((float(r[gi]), -1, int(ts[gi]), int(days[gi])))
                idx += max(int(th[gi]), 1)
            else:
                idx += 1
    out.sort(key=lambda t: t[2])
    dvals = sorted({t[3] for t in out})
    print(f"  DEBUG: {len(out)} trades spanning {len(dvals)} distinct days "
          f"(day {dvals[0]} .. {dvals[-1]})")
    return out, (max(sl, 2 * (SPREAD_FRAC + COMMISSION_FRAC)))


def monthly_income(trades, rules: FirmRules, risk_frac: float, eff_stop: float,
                   months: int = 12) -> dict:
    """Once funded, what does a year actually pay? Resets monthly on equity."""
    equity = rules.capital
    peak = rules.capital
    monthly = []
    cur_month = None
    m_start = equity
    per_trade = []
    for r, side, ts, day in trades:
        m = ts // 2_592_000  # ~30-day months, ts in seconds
        if cur_month is not None and m != cur_month:
            monthly.append(equity - m_start)
            m_start = equity
        cur_month = m
        if equity < peak - rules.max_dd_abs:
            break
        stake = math.floor(max(min((equity * risk_frac) / (eff_stop * MULTIPLIER),
                                   equity * 0.10, rules.capital), MIN_STAKE))
        if stake < MIN_STAKE or equity < MIN_STAKE:
            continue
        pnl = (r * eff_stop * side) * MULTIPLIER * stake - stake * COMMISSION_FRAC
        equity += pnl
        peak = max(peak, equity)
        per_trade.append(pnl)
    if cur_month is not None:
        monthly.append(equity - m_start)
    return {"months": len(monthly), "monthly": monthly[:months],
            "avg_month": float(np.mean(monthly)) if monthly else 0.0,
            "best": float(np.max(monthly)) if monthly else 0.0,
            "worst": float(np.min(monthly)) if monthly else 0.0,
            "positive_months": int(sum(1 for x in monthly if x > 0)),
            "total": float(np.sum(monthly)) if monthly else 0.0}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--capital", type=float, default=50_000.0)
    ap.add_argument("--target", type=float, default=8.0)
    ap.add_argument("--maxdd", type=float, default=10.0)
    ap.add_argument("--daily", type=float, default=5.0)
    ap.add_argument("--trailing", action="store_true")
    ap.add_argument("--split", type=float, default=80.0)
    ap.add_argument("--timeframe", default="5m")
    ap.add_argument("--bars", type=int, default=400_000)
    a = ap.parse_args()

    t0 = time.time()
    rules = FirmRules(capital=a.capital, profit_target_pct=a.target,
                      max_dd_pct=a.maxdd, daily_loss_pct=a.daily,
                      trailing_dd=a.trailing, split_pct=a.split / 100.0)

    print("=" * 72)
    print(f"  PROP FIRM SIMULATION  ·  {a.timeframe} gold")
    print(f"  capital ${rules.capital:,.0f}  ·  target +{rules.profit_target_pct:.0f}%  "
          f"   max DD {rules.max_dd_pct:.0f}% {'(trailing)' if rules.trailing_dd else '(static)'}  "
          f"   daily loss {rules.daily_loss_pct:.0f}%  ·  split {a.split:.0f}%")
    print("=" * 72)

    trades, eff_stop = collect_oos_trades(a.timeframe, 0.004, 0.004, 48, 0.55, 5, a.bars)
    print(f"  out-of-sample trades: {len(trades):,}  "
          f"({time.time()-t0:.0f}s to prepare)")

    print(f"\n  ── CHALLENGE PHASE ───────────────────────────────────────")
    for risk in (0.0025, 0.005, 0.01):
        res = simulate(trades, rules, risk, eff_stop, until_target=True)
        tag = "PASSED" if res.passed else "FAILED"
        print(f"  risk {risk*100:.2f}%  [{tag}]  trades {res.trades:<6,}  "
              f"days {res.days:<4}  equity ${res.end_equity:,.0f} "
              f"({100*(res.end_equity/rules.capital-1):+.1f}%)  "
              f"DD used {res.min_dd_used_pct:.1f}%")
        if not res.passed:
            print(f"      reason: {res.fail_reason}")

    print(f"\n  ── FUNDED PHASE (what you'd actually earn) ──────────────")
    for risk in (0.0025, 0.005, 0.01):
        mi = monthly_income(trades, rules, risk, eff_stop, months=12)
        if mi["months"] == 0:
            continue
        take = mi["avg_month"] * rules.split_pct
        print(f"  risk {risk*100:.2f}%  months {mi['months']:>2}  "
              f"avg/mo ${mi['avg_month']:,.0f}  best ${mi['best']:,.0f}  "
              f"worst ${mi['worst']:,.0f}")
        print(f"      after {rules.split_pct*100:.0f}% split -> ${take:,.0f}/month "
              f"(${take*12:,.0f}/year)  positive months {mi['positive_months']}/{mi['months']}")

    print(f"\n  elapsed {time.time()-t0:.0f}s")


if __name__ == "__main__":
    main()
