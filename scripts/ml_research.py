"""
ML edge-discovery pipeline for gold/FX.

This is the research layer the existing strategy engine does not have. It asks a
single question honestly: is there an exploitable predictive edge in this data,
out of sample, after costs?

Methodology (deliberately conservative):
  1. Triple-barrier labelling (Lopez de Prado) - models the real trading
     problem: take-profit vs stop-loss vs time barrier, not raw return signs.
  2. Purged walk-forward CV with an embargo - removes the label-window overlap
     that produces look-ahead leakage between train and test.
  3. Multiple-testing correction via a deflated Sharpe - we test many configs,
     so the best raw result is biased upward. This adjusts for that.
  4. Costs are charged on every trade. A model that cannot beat cost is dead.

Everything runs locally on the parquet/jsonl history already in this repo.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path

# Windows consoles default to cp1252 and will crash on box-drawing characters.
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

import numpy as np
import pandas as pd

ROOT = Path(__file__).resolve().parent.parent
GOLD_5M = ROOT / "trading_memory" / "gold_archive" / "XAU_5m_data.jsonl"
GOLD_1M = ROOT / "trading_memory" / "gold_archive" / "XAU_1m_data.jsonl"
RAW = ROOT / "trading_memory" / "raw"

# ── Cost model (Deriv frxXAUUSD multiplier) ────────────────────────────────
SPREAD_FRAC = 0.0015      # 0.15% round trip
COMMISSION_FRAC = 0.0005  # commission on stake


# ══════════════════════════════════════════════════════════════════════════
# Data
# ══════════════════════════════════════════════════════════════════════════
def load_gold(timeframe: str = "5m") -> pd.DataFrame:
    """Load gold candles. 5m/15m/1h ship pre-aggregated; 1m is the raw file."""
    path = {"5m": GOLD_5M, "15m": ROOT / "trading_memory/gold_archive/XAU_15m_data.jsonl",
            "1h": ROOT / "trading_memory/gold_archive/XAU_1h_data.jsonl",
            "1m": GOLD_1M}[timeframe]
    if not path.exists():
        sys.exit(f"missing {path}")
    df = pd.read_json(
        path,
        lines=True,
        convert_dates={"Date": "%Y.%m.%d %H:%M"},
    )
    df = df.rename(columns=str.title)
    df = df[["Date", "Open", "High", "Low", "Close", "Volume"]]
    df.columns = ["ts", "open", "high", "low", "close", "volume"]
    df["ts"] = pd.to_datetime(df["ts"], utc=True)
    df = df.dropna().sort_values("ts").drop_duplicates("ts").reset_index(drop=True)
    return df.astype({c: "float64" for c in ["open", "high", "low", "close", "volume"]})


# ══════════════════════════════════════════════════════════════════════════
# Features
# ══════════════════════════════════════════════════════════════════════════
def build_features(df: pd.DataFrame) -> pd.DataFrame:
    c, h, l, v = df["close"], df["high"], df["low"], df["volume"]
    f = pd.DataFrame(index=df.index)

    # Multi-horizon momentum (log returns)
    for k in (1, 3, 6, 12, 48, 144, 288):
        f[f"mom_{k}"] = np.log(c / c.shift(k))

    # Volatility and its ratio (regime proxy)
    r1 = np.log(c / c.shift(1))
    f["vol_12"] = r1.rolling(12).std()
    f["vol_48"] = r1.rolling(48).std()
    f["vol_288"] = r1.rolling(288).std()
    f["vol_ratio"] = f["vol_48"] / (f["vol_288"] + 1e-12)
    f["vol_z"] = (f["vol_48"] - f["vol_48"].rolling(288).mean()) / (
        f["vol_48"].rolling(288).std() + 1e-12
    )

    # Normalised ATR
    tr = pd.concat([h - l, (h - c.shift()).abs(), (l - c.shift()).abs()], axis=1).max(axis=1)
    atr = tr.rolling(14).mean()
    f["atr_pct"] = atr / c

    # RSI
    up = r1.clip(lower=0).rolling(14).mean()
    dn = (-r1.clip(upper=0)).rolling(14).mean()
    f["rsi"] = 100 - 100 / (1 + up / (dn + 1e-12))

    # Distance from moving averages (trend strength)
    for k in (20, 50, 200):
        f[f"ema_d_{k}"] = c / c.rolling(k).mean() - 1

    # Bollinger position
    mid = c.rolling(20).mean()
    sd = c.rolling(20).std()
    f["bb_pos"] = (c - mid) / (sd + 1e-12)

    # Range and volume
    f["range_pct"] = (h - l) / (c + 1e-12)
    f["vol_z_score"] = (v - v.rolling(48).mean()) / (v.rolling(48).std() + 1e-12)
    f["body"] = (c - df["open"]) / (df["open"] + 1e-12)

    # Calendar effects (cyclical encoding so trees can split on it)
    hod = df["ts"].dt.hour + df["ts"].dt.minute / 60
    dow = df["ts"].dt.dayofweek
    f["hod_sin"] = np.sin(2 * np.pi * hod / 24)
    f["hod_cos"] = np.cos(2 * np.pi * hod / 24)
    f["dow"] = dow.astype(float)

    return f.replace([np.inf, -np.inf], np.nan)


# ══════════════════════════════════════════════════════════════════════════
# Triple-barrier labelling
# ══════════════════════════════════════════════════════════════════════════
def triple_barrier(
    df: pd.DataFrame,
    atr_pct: pd.Series,
    pt_pct: float,
    sl_pct: float,
    vertical: int,
) -> pd.DataFrame:
    """Label each bar with what a trade entered there would have done.

    Runs forward from bar i and records which barrier is touched first:
      +1 take-profit, -1 stop-loss, 0 neither (time barrier).

    Barriers are expressed as a fraction of price (not absolute ATR), because a
    5-minute ATR is typically 0.05% while round-trip cost is 0.20% - an
    ATR-scaled target is unreachable after costs by construction. Each barrier
    is floored at `min_barrier_pct` so the target always clears cost.
    """
    n = len(df)
    high = df["high"].to_numpy()
    low = df["low"].to_numpy()
    close = df["close"].to_numpy()
    apct = atr_pct.to_numpy()

    cost = SPREAD_FRAC + COMMISSION_FRAC
    min_barrier = cost * 2.0

    label = np.zeros(n, dtype=np.int8)
    t_hit = np.zeros(n, dtype=np.int32)
    ret_r = np.zeros(n, dtype=np.float32)

    for i in range(n - 1):
        c = close[i]
        if not np.isfinite(c) or c <= 0:
            continue
        # Volatility-scaled barrier, floored so cost is always covered.
        scale = apct[i] if np.isfinite(apct[i]) and apct[i] > 0 else 0.0
        pt_p = max(pt_pct, scale * 1.5, min_barrier)
        sl_p = max(sl_pct, scale * 1.5, min_barrier)
        risk = c * sl_p
        if risk <= 0:
            continue
        end = min(i + vertical, n)
        for j in range(i + 1, end):
            lo_hit = low[j] <= c * (1 - sl_p)
            hi_hit = high[j] >= c * (1 + pt_p)
            if lo_hit:  # pessimistic: stop assumed first
                label[i] = -1
                t_hit[i] = j - i
                ret_r[i] = -1.0
                break
            if hi_hit:
                label[i] = 1
                t_hit[i] = j - i
                # Net of the spread paid on entry and the target fill.
                ret_r[i] = ((pt_p - cost) / sl_p)
                break
        else:
            label[i] = 0
            t_hit[i] = end - i
            ret_r[i] = ((close[end - 1] - c) / c - cost) / sl_p
    return pd.DataFrame({"label": label, "t_hit": t_hit, "ret_r": ret_r}, index=df.index)


# ══════════════════════════════════════════════════════════════════════════
# Purged walk-forward CV with embargo
# ══════════════════════════════════════════════════════════════════════════
@dataclass
class Fold:
    tr_lo: int
    tr_hi: int
    te_lo: int
    te_hi: int


def purged_walk_forward(n: int, max_hold: int, embargo: float = 0.02, folds: int = 5) -> list[Fold]:
    """Expanding-window walk-forward.

    Purge: training samples whose label window [i, i+max_hold] reaches into the
    test window are dropped, otherwise the model sees the answer.

    Embargo: a further slice after the test window is dropped from training.
    """
    out: list[Fold] = []
    test_size = n // (folds + 2)
    emb = int(n * embargo)
    start = test_size
    for k in range(folds):
        te_lo = start + k * test_size
        te_hi = min(te_lo + test_size, n)
        if te_lo >= n:
            break
        tr_hi = te_lo - max_hold            # purge the overlap window
        if tr_hi <= max_hold:
            continue
        tr_lo = 0
        # embargo applies to the tail of a rolling variant; with an expanding
        # window the purge above already dominates, so it is a no-op here but
        # documented for completeness.
        out.append(Fold(tr_lo, tr_hi, te_lo, te_hi))
    return out


# ══════════════════════════════════════════════════════════════════════════
# Evaluation
# ══════════════════════════════════════════════════════════════════════════
@dataclass
class Stats:
    trades: int = 0
    wins: int = 0
    r_sum: float = 0.0
    gross_win: float = 0.0
    gross_loss: float = 0.0
    max_dd: float = 0.0
    peak: float = 0.0
    curve: list = field(default_factory=list)

    def add(self, r: float):
        self.trades += 1
        self.r_sum += r
        if r > 0:
            self.wins += 1
            self.gross_win += r
        else:
            self.gross_loss += -r
        if self.curve:
            self.peak = max(self.peak, self.curve[-1])
        self.curve.append(self.curve[-1] + r if self.curve else r)
        self.peak = max(self.peak, self.curve[-1])
        self.max_dd = max(self.max_dd, self.peak - self.curve[-1])

    @property
    def win_rate(self) -> float:
        return 100 * self.wins / self.trades if self.trades else 0.0

    @property
    def pf(self) -> float:
        return self.gross_win / self.gross_loss if self.gross_loss > 1e-12 else float("inf")

    @property
    def expectancy(self) -> float:
        return self.r_sum / self.trades if self.trades else 0.0


def deflated_sharpe(sharpe: float, trials: int, n_obs: int) -> float:
    """Approximate Bailey & Lopez de Prado deflated Sharpe.

    Expected max Sharpe under the null across `trials` independent trials,
    then the probability that an observed Sharpe exceeds it.
    """
    if trials < 2 or n_obs < 30 or sharpe <= 0:
        return 0.0
    euler = 0.5772156649
    s0 = (1 - euler) * _norm_ppf(1 - 1 / trials) + euler * _norm_ppf(1 - 1 / (trials * np.e))
    se = 1.0 / np.sqrt(max(n_obs - 1, 1))
    return float(_norm_cdf((sharpe - s0) / se))


def _norm_ppf(p: float) -> float:
    from math import sqrt
    from math import log
    # Acklam's inverse normal CDF approximation.
    a = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02,
         1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00]
    b = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02,
         6.680131188771972e+01, -1.328068155288572e+01]
    c = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00,
         -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00]
    d = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00,
         3.754408661907416e+00]
    plow, phigh = 0.02425, 1 - 0.02425
    if p < plow:
        q = sqrt(-2 * log(p))
        return (((((c[0]*q+c[1])*q+c[2])*q+c[3])*q+c[4])*q+c[5]) / ((((d[0]*q+d[1])*q+d[2])*q+d[3])*q+1)
    if p > phigh:
        q = sqrt(-2 * log(1 - p))
        return -(((((c[0]*q+c[1])*q+c[2])*q+c[3])*q+c[4])*q+c[5]) / ((((d[0]*q+d[1])*q+d[2])*q+d[3])*q+1)
    q = p - 0.5
    r = q * q
    return (((((a[0]*r+a[1])*r+a[2])*r+a[3])*r+a[4])*r+a[5])*q / (((((b[0]*r+b[1])*r+b[2])*r+b[3])*r+b[4])*r+1)


def _norm_cdf(x: float) -> float:
    from math import erf, sqrt
    return 0.5 * (1 + erf(x / sqrt(2)))


# ══════════════════════════════════════════════════════════════════════════
# Account simulation - real dollars, real constraints
# ══════════════════════════════════════════════════════════════════════════
# Deriv multiplier economics: a price move of `d` (fraction) on a stake of S
# returns d * MULTIPLIER * S. Everything below is in actual USD.
MULTIPLIER = 100.0
MIN_STAKE = 1.0          # Deriv will not accept less than $1
MAX_STAKE = 2000.0
# No real account commits an unbounded fraction of equity to one position.
# Without this cap a positive expectancy compounds into absurd paper returns
# that could never survive contact with a broker.
MAX_STAKE_FRAC = 0.10    # at most 10% of equity per position


@dataclass
class Account:
    start: float
    balance: float
    peak: float
    max_dd_abs: float = 0.0
    max_dd_pct: float = 0.0
    trades: int = 0
    skipped: int = 0
    bankrupt_at: int = -1     # trade count where trading stopped, -1 = survived
    curve: list = field(default_factory=list)

    def can_trade(self) -> bool:
        return self.balance >= MIN_STAKE

    def apply(self, pnl: float):
        self.balance += pnl
        self.trades += 1
        self.peak = max(self.peak, self.balance)
        dd = self.peak - self.balance
        self.max_dd_abs = max(self.max_dd_abs, dd)
        if self.peak > 0:
            self.max_dd_pct = max(self.max_dd_pct, 100.0 * dd / self.peak)
        self.curve.append(self.balance)


def size_stake(balance: float, stop_pct: float, risk_frac: float) -> float:
    """Convert a risk fraction into an executable whole-dollar Deriv stake.

    Returns 0 when the account is too small to express this risk at this stop
    width - which is a real answer, not a rounding artefact.
    """
    if stop_pct <= 0 or balance <= 0:
        return 0.0
    risk_amount = balance * risk_frac
    stake = risk_amount / (stop_pct * MULTIPLIER)
    stake = min(stake, MAX_STAKE, balance, balance * MAX_STAKE_FRAC)
    stake = max(stake, MIN_STAKE)
    stake = math.floor(stake)
    return stake if stake >= MIN_STAKE else 0.0


@dataclass
class AccountRun:
    """Out-of-sample trades recorded for replay against a real balance."""
    trades: list = field(default_factory=list)

    def add(self, r: float, side: int, stop_pct: float, day: int):
        self.trades.append((r, side, stop_pct, day))


def run_account_sim(run: AccountRun, start_balance: float,
                    risk_frac: float, label: str,
                    max_daily_loss_pct: float = 0.05,
                    max_consec_losses: int = 5,
                    max_dd_halt_pct: float = 0.30) -> Account:
    """Replay out-of-sample trades against a real balance WITH the risk rules a
    live account actually enforces.

    Without these gates a weak positive expectancy compounds into paper returns
    nobody could ever achieve, because nothing stops it after a 90% collapse.
    These are the same guards the desktop app already has: daily loss cap,
    consecutive-loss cooldown, and a drawdown kill switch.
    """
    acct = Account(start=start_balance, balance=start_balance, peak=start_balance)
    day_pnl = 0.0
    consec_losses = 0
    halted = False
    halt_reason = ""
    cur_day = None
    days_halted = 0

    for r, side, stop_pct, day in run.trades:
        if halted:
            acct.skipped += 1
            continue
        if not acct.can_trade():
            acct.bankrupt_at = acct.trades
            halt_reason = "bankrupt"
            break
        # New session: reset the intraday guards.
        if day != cur_day:
            cur_day = day
            day_pnl = 0.0
            consec_losses = 0
        # Drawdown kill switch (permanent).
        if acct.peak > 0:
            dd_pct = 100.0 * (acct.peak - acct.balance) / acct.peak
            if dd_pct >= max_dd_halt_pct * 100:
                halted = True
                halt_reason = f"drawdown {dd_pct:.1f}% >= {max_dd_halt_pct*100:.0f}%"
                break
        # Daily loss cap and consecutive-loss cooldown apply for the rest of
        # the session, then reset the next day.
        if day_pnl <= -acct.peak * max_daily_loss_pct or consec_losses >= max_consec_losses:
            acct.skipped += 1
            days_halted += 1
            continue

        stake = size_stake(acct.balance, stop_pct, risk_frac)
        if stake < MIN_STAKE:
            acct.skipped += 1
            continue

        move = r * stop_pct * side
        pnl = move * MULTIPLIER * stake - stake * COMMISSION_FRAC
        acct.apply(pnl)
        day_pnl += pnl
        if pnl < 0:
            consec_losses += 1
        else:
            consec_losses = 0

    print(f"\n  ── ACCOUNT SIM: {label} ──────────────────────────────────")
    print(f"  rules: daily loss {max_daily_loss_pct*100:.0f}%  ·  "
          f"consec-loss halt {max_consec_losses}  ·  DD kill {max_dd_halt_pct*100:.0f}%")
    print(f"  start balance    : ${acct.start:,.2f}")
    print(f"  end balance      : ${acct.balance:,.2f}")
    ret = 100.0 * (acct.balance - acct.start) / acct.start
    print(f"  return           : {ret:+.2f}%")
    print(f"  trades executed  : {acct.trades:,}   skipped/halted: {acct.skipped:,}")
    print(f"  max drawdown     : ${acct.max_dd_abs:,.2f}  ({acct.max_dd_pct:.1f}%)")
    if halted:
        print(f"  ** HALTED: {halt_reason}")
    elif acct.bankrupt_at >= 0:
        print(f"  ** BUSTED after {acct.bankrupt_at} trades")
    else:
        print(f"  survived the whole test window with all risk gates active")
    return acct


# ══════════════════════════════════════════════════════════════════════════
# Main experiment
# ══════════════════════════════════════════════════════════════════════════
def run(timeframe: str, pt: float, sl: float, vertical: int, prob_thresh: float,
        max_depth: int, folds: int) -> dict:
    from sklearn.ensemble import HistGradientBoostingClassifier
    from sklearn.metrics import roc_auc_score

    t0 = time.time()
    print(f"\n{'='*74}\n  ML EDGE SEARCH  ·  gold {timeframe}  ·  target={pt} stop={sl} "
          f"vert={vertical} thr={prob_thresh}\n{'='*74}")

    df = load_gold(timeframe)
    print(f"  loaded {len(df):,} bars  {df.ts.iloc[0]} .. {df.ts.iloc[-1]}")

    feats = build_features(df)
    tr = pd.concat([df["high"], df["low"], df["close"]], axis=1)
    tr_all = pd.concat([tr["high"] - tr["low"],
                        (tr["high"] - tr["close"].shift()).abs(),
                        (tr["low"] - tr["close"].shift()).abs()], axis=1).max(axis=1)
    atr = tr_all.rolling(14).mean() / df["close"]

    lab = triple_barrier(df, atr, pt, sl, vertical)
    data = feats.join(lab)
    data["ret_r"] = data["ret_r"].clip(-3, 3)
    data = data.replace([np.inf, -np.inf], np.nan)
    # Capture which raw rows survive BEFORE the reset, so timestamps and bar
    # indices stay aligned with the feature matrix.
    keep_idx = data.index[data.notna().all(axis=1)]
    data = data.dropna().reset_index(drop=True)
    data_ts = df.loc[keep_idx, "ts"].astype("datetime64[s]").astype("int64").to_numpy()  # epoch seconds
    print(f"  usable samples: {len(data):,}  (dropped {len(df)-len(data):,} for warmup/NaN)")

    pos = (data["label"] == 1).mean()
    print(f"  base rates: TP {pos*100:.2f}%  SL {(data['label']==-1).mean()*100:.2f}%  "
          f"time {(data['label']==0).mean()*100:.2f}%")

    X = data.drop(columns=["label", "t_hit", "ret_r"]).to_numpy(np.float32)
    y = data["label"].to_numpy()
    r_true = data["ret_r"].to_numpy(np.float32)
    n = len(y)
    # The effective stop width the labeler used (floored at 2x cost), so the
    # account sim sizes positions against the same stop the backtest assumed.
    eff_stop_pct = max(sl, 2 * (SPREAD_FRAC + COMMISSION_FRAC))

    max_hold = vertical
    splits = purged_walk_forward(n, max_hold, folds=folds)
    if not splits:
        return {"ok": False, "reason": "not enough data for folds"}

    oos = Stats()
    acct_run = AccountRun()
    aucs = []
    for fi, f in enumerate(splits):
        Xtr, ytr, rtr = X[f.tr_lo:f.tr_hi], y[f.tr_lo:f.tr_hi], r_true[f.tr_lo:f.tr_hi]
        Xte, yte, rte = X[f.te_lo:f.te_hi], y[f.te_lo:f.te_hi], r_true[f.te_lo:f.te_hi]

        # Train a DIRECTIONAL model: TP=1, SL=0, drop time-barrier samples.
        m = ytr != 0
        clf = HistGradientBoostingClassifier(
            max_depth=max_depth, max_iter=200, learning_rate=0.06,
            min_samples_leaf=200, l2_regularization=1.0, random_state=42,
        )
        clf.fit(Xtr[m], (ytr[m] > 0).astype(int))
        p = clf.predict_proba(Xte)[:, 1]

        # AUC must be measured on a binary target. The tradeable question is
        # "which direction wins", so score only samples that resolved to TP or
        # SL and map them to {0,1}. Time-barrier samples are excluded - they
        # carry no directional information.
        resolved = (yte != 0)
        if resolved.sum() > 100 and len(np.unique(yte[resolved])) == 2:
            auc = roc_auc_score((yte[resolved] > 0).astype(int), p[resolved])
        else:
            auc = 0.5
        aucs.append(auc)

        # Trade only on a confident directional call, taking the model's side.
        #
        # CRITICAL: trades must not overlap. Evaluating a signal on every bar
        # while each position lives up to `vertical` bars counts the same market
        # move dozens of times and inflates every statistic. So we walk the
        # test set sequentially and, once a position is taken, skip forward by
        # its actual holding time - exactly what the live engine does with its
        # max_open_positions gate.
        t_hit_te = data["t_hit"].to_numpy()[f.te_lo:f.te_hi]
        ts_te = data_ts[f.te_lo:f.te_hi]
        day_te = (ts_te // 86_400).astype(np.int64)  # ts in seconds
        idx = 0
        n_te = len(yte)
        taken = 0
        while idx < n_te:
            pi = p[idx]
            if pi >= prob_thresh:
                acct_run.add(float(rte[idx]), +1, eff_stop_pct, day_te[idx])
                oos.add(float(rte[idx]))
                taken += 1
                idx += max(int(t_hit_te[idx]), 1)
            elif pi <= 1.0 - prob_thresh:
                acct_run.add(float(rte[idx]), -1, eff_stop_pct, day_te[idx])
                oos.add(float(-rte[idx]))
                taken += 1
                idx += max(int(t_hit_te[idx]), 1)
            else:
                idx += 1
        sel = np.zeros(n_te, dtype=bool)
        sel[:] = taken > 0  # keep the print shape stable
        print(f"  fold {fi+1}: train {len(ytr):>7,}  test {len(yte):>7,}  "
              f"AUC {auc:.4f}  non-overlapping trades {taken:>6,}  "
              f"p[{p.min():.3f},{p.max():.3f}]")

    trials = folds  # one config per fold sweep in this run
    sharpe = 0.0
    if oos.trades > 5:
        # Per-trade Sharpe on the equity increments, then annualise using the
        # observed trades-per-year (roughly one trade every `vertical` bars).
        rets = np.diff(np.array([0.0] + oos.curve))
        s = rets.std(ddof=1)
        if s > 1e-12:
            per_trade = rets.mean() / s
            trades_per_year = 252 * 96 * 24 * 60 / max(vertical, 1) / 288
            sharpe = per_trade * np.sqrt(max(trades_per_year, 1.0))
    dsr = deflated_sharpe(sharpe, trials, oos.trades)

    print(f"\n{'─'*74}")
    print(f"  OUT-OF-SAMPLE ({oos.trades:,} trades, net of {SPREAD_FRAC*100:.2f}% spread "
          f"+ {COMMISSION_FRAC*100:.2f}% commission)")
    print(f"{'─'*74}")
    print(f"  win rate      : {oos.win_rate:.2f}%")
    print(f"  expectancy    : {oos.expectancy:+.5f} R/trade")
    print(f"  profit factor : {oos.pf:.4f}")
    print(f"  max drawdown  : {oos.max_dd:.3f} R")
    print(f"  mean AUC      : {np.mean(aucs):.4f}  (0.5 = no signal)")
    print(f"  Sharpe        : {sharpe:+.3f}   deflated p-value {dsr:.4f}")
    print(f"  elapsed       : {time.time()-t0:.1f}s")

    ok = oos.trades >= 100 and oos.expectancy > 0 and np.mean(aucs) > 0.52
    print(f"\n  VERDICT: {'✓ signal detected' if ok else '✗ no exploitable edge'}")
    if ok:
        print("  Still needs an independent period and a live demo run before capital.")
    else:
        print("  Either no edge, or this configuration does not capture it.")

    # ── The number that actually matters: a real $ balance ────────────────
    print(f"\n{'='*74}")
    print("  ACCOUNT SIMULATION — real dollars, Deriv constraints")
    print(f"  (min stake ${MIN_STAKE:.0f}, {MULTIPLIER:.0f}x multiplier, "
          f"commission {COMMISSION_FRAC*100:.2f}%)")
    print(f"{'='*74}")
    results = []
    for bal in (100.0, 200.0):
        for risk in (0.005, 0.01):
            a = run_account_sim(acct_run, bal, risk, f"${bal:.0f} @ {risk*100:.1f}% risk")
            results.append((bal, risk, a))

    # ── Realistic verdict ────────────────────────────────────────────────
    print(f"\n{'='*74}")
    print("  TRADABILITY VERDICT")
    print(f"{'='*74}")
    survived = [(b, r, a) for b, r, a in results
                if a.bankrupt_at < 0 and a.balance > a.start]
    all_survived = all(a.balance > a.start for _, _, a in results)
    worst_dd = max(a.max_dd_pct for _, _, a in results)
    any_halt = any(a.skipped > a.trades * 0.5 for _, _, a in results)

    if not survived:
        print("  ✗ NOT TRADEABLE - no configuration ended above its start balance")
        print("    with the risk gates active.")
    elif not all_survived:
        print("  ✗ FRAGILE - only some configurations survived.")
        for b, r, a in results:
            tag = "OK" if a.balance > a.start else "LOST"
            print(f"      ${b:.0f} @ {r*100:.1f}%  -> ${a.balance:,.2f}  [{tag}]")
    elif worst_dd > 50:
        print(f"  ✗ NOT TRADEABLE - survived, but {worst_dd:.0f}% max drawdown.")
        print("    A drawdown that deep stops a human long before the recovery.")
    elif any_halt:
        print("  ~ MARGINAL - positive, but the risk gates blocked most trades.")
        print("    The edge is too thin to run at production risk limits.")
    else:
        print("  ✓ TRADEABLE ON PAPER - positive with gates, drawdown acceptable.")
        print("    Still requires a live demo run before any real capital.")

    return {"ok": bool(ok), "timeframe": timeframe, "pt": pt, "sl": sl, "vertical": vertical,
            "thr": prob_thresh, "trades": oos.trades, "win_rate": oos.win_rate,
            "expectancy": oos.expectancy, "pf": oos.pf, "auc": float(np.mean(aucs)),
            "sharpe": sharpe, "deflated_p": dsr}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--timeframe", default="5m")
    ap.add_argument("--pt", type=float, default=0.004, help="take-profit, fraction of price")
    ap.add_argument("--sl", type=float, default=0.004, help="stop-loss, fraction of price")
    ap.add_argument("--vertical", type=int, default=48)
    ap.add_argument("--thr", type=float, default=0.55)
    ap.add_argument("--depth", type=int, default=5)
    ap.add_argument("--folds", type=int, default=5)
    ap.add_argument("--leak-test", action="store_true",
                    help="permutation test: does the model beat noise?")
    a = ap.parse_args()
    if a.leak_test:
        leak_test(a.timeframe, a.pt, a.sl, a.vertical, a.folds)
    else:
        run(a.timeframe, a.pt, a.sl, a.vertical, a.thr, a.depth, a.folds)


def leak_test(timeframe: str, pt: float, sl: float, vertical: int, folds: int):
    """Decisive check: does the model still 'predict' shuffled labels?

    If AUC stays meaningfully above 0.5 on randomly permuted targets, the
    pipeline is leaking information from the label into the features, and every
    number it produces is fiction. If it collapses to 0.50, the pipeline is
    sound and a real 0.54 means a real (if weak) signal.
    """
    from sklearn.ensemble import HistGradientBoostingClassifier
    from sklearn.metrics import roc_auc_score

    print("=" * 74)
    print("  LEAKAGE TEST - permutation of labels within the training window")
    print("=" * 74)

    df = load_gold(timeframe)
    feats = build_features(df)
    tr_all = pd.concat([
        df["high"] - df["low"],
        (df["high"] - df["close"].shift()).abs(),
        (df["low"] - df["close"].shift()).abs(),
    ], axis=1).max(axis=1)
    atr_pct = tr_all.rolling(14).mean() / df["close"]
    lab = triple_barrier(df, atr_pct, pt, sl, vertical)
    data = feats.join(lab).replace([np.inf, -np.inf], np.nan).dropna().reset_index(drop=True)

    y_raw = data["label"].to_numpy()
    n = len(y_raw)
    f = purged_walk_forward(n, vertical, folds=folds)[0]  # first fold only
    tr = slice(f.tr_lo, f.tr_hi)
    te = slice(f.te_lo, f.te_hi)

    X = data.drop(columns=["label", "t_hit", "ret_r"]).to_numpy(np.float32)
    Xtr_all = X[tr]
    Xte = X[te]

    def auc_for(labels: np.ndarray) -> float:
        clf = HistGradientBoostingClassifier(
            max_depth=5, max_iter=120, learning_rate=0.06,
            min_samples_leaf=200, l2_regularization=1.0, random_state=42)
        ytr_full = labels[tr]
        m = ytr_full != 0
        if m.sum() < 100 or len(np.unique(ytr_full[m])) < 2:
            return float("nan")
        clf.fit(Xtr_all[m], ytr_full[m])
        yte_full = labels[te]
        mt = yte_full != 0
        if mt.sum() < 100 or len(np.unique(yte_full[mt])) < 2:
            return float("nan")
        p = clf.predict_proba(Xte[mt])[:, 1]
        return roc_auc_score((yte_full[mt] > 0).astype(int), p)

    real = auc_for(y_raw)
    print(f"  real labels          AUC = {real:.4f}")

    perms = []
    rng = np.random.default_rng(7)
    for k in range(4):
        # Permute labels inside the training window only - this destroys any
        # genuine signal while leaving the test window untouched.
        shuffled = y_raw.copy()
        perm = rng.permutation(np.arange(f.tr_lo, f.tr_hi))
        shuffled[f.tr_lo:f.tr_hi] = y_raw[f.tr_lo:f.tr_hi][perm]
        a = auc_for(shuffled)
        perms.append(a)
        print(f"  permuted labels #{k+1}  AUC = {a:.4f}")

    pmean = float(np.nanmean(perms))
    pstd = float(np.nanstd(perms))
    print("-" * 74)
    print(f"  permuted mean        AUC = {pmean:.4f}  (sd {pstd:.4f})")
    lift = real - pmean
    print(f"  lift over noise      = {lift:+.4f}")
    print("-" * 74)
    if real - pmean < 2 * pstd:
        print("  ✗ FAILS the leakage test: observed lift is within the noise band.")
        print("    The apparent edge is an artefact. Do not trust any output above.")
    else:
        print("  ✓ PASSES: the lift exceeds the permutation noise band.")
        print("    There is a real, if weak, predictive signal in these features.")


if __name__ == "__main__":
    main()
