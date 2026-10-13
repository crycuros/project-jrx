"""
Train the gold model on ALL available history and freeze it for forward testing.

After this runs, the model is never retrained during the forward test. That is
the entire point: every subsequent prediction is made by a model that has never
seen the bar it is predicting, which is what makes the forward numbers honest.

Usage:
    python scripts/ml_train.py
"""

from __future__ import annotations

import json
import pickle
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ml_research import (  # noqa: E402
    ROOT, SPREAD_FRAC, COMMISSION_FRAC, build_features, load_gold, triple_barrier,
)

MODEL_DIR = ROOT / "data" / "models"
MODEL_PATH = MODEL_DIR / "gold_5m_v1.pkl"


def main():
    from sklearn.ensemble import HistGradientBoostingClassifier

    t0 = time.time()
    tf = "5m"
    pt = 0.004
    sl = 0.004
    vertical = 48

    print("=" * 70)
    print("  TRAIN + FREEZE  ·  gold 5m")
    print("=" * 70)

    df = load_gold(tf)
    print(f"  history: {len(df):,} bars  {df.ts.iloc[0]} .. {df.ts.iloc[-1]}")

    feats = build_features(df)
    tr_all = pd.concat([
        df["high"] - df["low"],
        (df["high"] - df["close"].shift()).abs(),
        (df["low"] - df["close"].shift()).abs(),
    ], axis=1).max(axis=1)
    atr_pct = (tr_all.rolling(14).mean() / df["close"])

    lab = triple_barrier(df, atr_pct, pt, sl, vertical)
    data = feats.join(lab)
    data["ret_r"] = data["ret_r"].clip(-3, 3)
    data = data.replace([np.inf, -np.inf], np.nan).dropna().reset_index(drop=True)

    resolved = data["label"] != 0
    y = (data.loc[resolved, "label"] > 0).astype(int).to_numpy()
    X = data.drop(columns=["label", "t_hit", "ret_r"]).to_numpy(np.float32)[resolved.to_numpy()]

    print(f"  training samples: {len(X):,}  (resolved TP/SL only)")
    print(f"  base win rate   : {100*y.mean():.2f}%")

    clf = HistGradientBoostingClassifier(
        max_depth=5, max_iter=200, learning_rate=0.06,
        min_samples_leaf=200, l2_regularization=1.0, random_state=42,
    )
    clf.fit(X, y)
    print(f"  trained in {time.time()-t0:.1f}s")

    # In-sample AUC is reported only as a sanity check that the model learned
    # something at all. It is NOT a performance claim - the forward test is.
    from sklearn.metrics import roc_auc_score
    insample_auc = roc_auc_score(y, clf.predict_proba(X)[:, 1])
    print(f"  in-sample AUC   : {insample_auc:.4f}  (sanity check only)")

    MODEL_DIR.mkdir(parents=True, exist_ok=True)
    payload = {
        "model": clf,
        "features": list(data.drop(columns=["label", "t_hit", "ret_r"]).columns),
        "timeframe": tf,
        "pt_pct": pt,
        "sl_pct": sl,
        "vertical": vertical,
        "spread_frac": SPREAD_FRAC,
        "commission_frac": COMMISSION_FRAC,
        "trained_through": str(df.ts.iloc[-1]),
        "n_train": int(len(X)),
        "insample_auc": float(insample_auc),
        "created": time.strftime("%Y-%m-%d %H:%M:%S"),
    }
    with open(MODEL_PATH, "wb") as f:
        pickle.dump(payload, f)

    meta_path = MODEL_DIR / "gold_5m_v1.json"
    meta_path.write_text(json.dumps(
        {k: v for k, v in payload.items() if k != "model"}, indent=2, default=str))

    print(f"\n  FROZEN -> {MODEL_PATH}")
    print(f"  trained through : {payload['trained_through']}")
    print("  This model will NOT be retrained during the forward test.")
    print("  Every prediction from here on is genuinely out-of-sample.")


if __name__ == "__main__":
    main()