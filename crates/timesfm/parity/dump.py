"""Dump TimesFM 3.0 inputs and outputs for the Rust parity test.

Run with the Python finance service's virtualenv (timesfm 3.0.2, torch):

    cd personal-site/services/finance
    PYTHONPATH=src .venv/bin/python \
        <repo>/crates/timesfm/parity/dump.py OUT_DIR [GOLDEN_DB [NOW]]

Writes to OUT_DIR:

* ``synthetic.json``: ``predict_batch`` calls on made-up series (short, long,
  trending, lumpy, with and without covariates, horizons past one output
  patch), each with all nine quantiles.
* ``internals.json``: one small decode with the model's intermediate tensors.
* ``golden_calls.json`` (with GOLDEN_DB): every ``quantiles`` call the
  dashboard's daily forecast makes on that database with TimesFM enabled,
  with the service's (q0.1, mean, q0.9) rows and the full quantiles.
* ``golden_dashboard.json`` (with GOLDEN_DB): that dashboard's
  ``daily_forecasts``.

GOLDEN_DB is opened read-only; the dashboard's forecast caches are bypassed
so every call runs.
"""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime
from pathlib import Path

import numpy as np

CHECKPOINT = "google/timesfm-3.0-pytorch"


def _list(array: np.ndarray) -> list:
    return np.asarray(array, dtype=np.float64).tolist()


def _model():
    import timesfm
    import torch

    torch.set_num_threads(2)
    return timesfm.TimesFM3Forecaster.from_pretrained(CHECKPOINT, device="cpu")


def _predict(model, contexts, horizon, covariates):
    outputs = model.predict_batch(
        [np.asarray(c, dtype=np.float32) for c in contexts],
        horizon=horizon,
        past_future_covariates=(
            None
            if covariates is None
            else [None if c is None else np.asarray(c, dtype=np.float32) for c in covariates]
        ),
        return_quantiles=True,
        make_positive=True,
    )
    return [np.asarray(o.quantiles)[:horizon] for o in outputs]


def synthetic(model, out: Path) -> None:
    rng = np.random.default_rng(7)
    days = np.arange(800)
    lumpy = np.where(rng.random(800) < 0.4, rng.gamma(2.0, 15.0, 800), 0.0)
    weekly = 20 + 15 * np.sin(2 * np.pi * days / 7) + rng.normal(0, 3, 800)
    trend = 0.5 * days + rng.normal(0, 2, 800)
    signed = rng.normal(0, 5, 800).cumsum()
    planned = np.where(days % 30 == 3, 120.0, 0.0) + np.where(days % 7 == 2, 35.0, 0.0)
    cases = []

    def case(name, contexts, horizon, covariates=None):
        cases.append(
            {
                "name": name,
                "contexts": [_list(c) for c in contexts],
                "horizon": horizon,
                "covariates": None
                if covariates is None
                else [None if c is None else _list(c) for c in covariates],
                "quantiles": [_list(q) for q in _predict(model, contexts, horizon, covariates)],
            }
        )

    case("single_short", [lumpy[:20]], 7)
    case("single_weekly", [weekly[:200]], 36)
    case("trend_detrended", [trend[:300]], 64)
    case("signed_not_positive", [signed[:150]], 40)
    case("lumpy_long", [lumpy[:730]], 130)
    case("over_one_patch", [weekly[:100]], 100)
    case(
        "mixed_lengths_two_batches",
        [lumpy[:700], lumpy[:693], lumpy[:686], lumpy[:679], weekly[:90], trend[:33]],
        7,
    )
    case(
        "covariates_backtest",
        [lumpy[:400], lumpy[:393], lumpy[:386], lumpy[:379], lumpy[:372]],
        7,
        [planned[: 400 + 7], planned[: 393 + 7], planned[: 386 + 7], planned[: 379 + 7], planned[: 372 + 7]],
    )
    case("covariates_live", [lumpy[:500]], 66, [planned[: 500 + 66]])
    case("covariates_long_horizon", [weekly[:300]], 150, [planned[: 300 + 150]])
    case("covariates_some_missing", [weekly[:120], lumpy[:150]], 20, [planned[:140], None])
    case("all_zero", [np.zeros(60)], 7)
    case("constant", [np.full(64, 12.5)], 7)
    (out / "synthetic.json").write_text(json.dumps({"checkpoint": CHECKPOINT, "cases": cases}))


def internals(model, out: Path) -> None:
    import torch

    rng = np.random.default_rng(11)
    target = np.where(rng.random(70) < 0.5, rng.gamma(2.0, 10.0, 70), 0.0).astype(np.float32)
    cov = np.where(np.arange(70 + 10) % 9 == 4, 50.0, 0.0).astype(np.float32)
    # Left-pad to 96 as predict_batch would.
    context = 96
    pad = context - 70
    tgt = np.pad(target, (pad, 0))[None, None, :]
    pf = np.pad(cov, (pad, 0))[None, None, :]
    mask = np.zeros((1, context), dtype=bool)
    mask[:, :pad] = True
    with torch.inference_mode():
        logits, aux = model.model.decode(
            target=torch.from_numpy(tgt),
            past_future_covariates=torch.from_numpy(pf),
            mask=torch.from_numpy(mask),
            return_aux_outputs=True,
        )
    payload = {
        "target": _list(target),
        "covariate": _list(cov),
        "context": context,
        "horizon": 10,
        "resblock_input": _list(aux["__call__:resblock_input"]),
        "transformer_input": _list(aux["__call__:transformer_input"]),
        "transformer_output": _list(aux["__call__:transformer_output"]),
        "logits": _list(aux["logits"]),
        "horizon_logits": _list(logits),
    }
    (out / "internals.json").write_text(json.dumps(payload))


def golden(model, out: Path, db: Path, now: datetime) -> None:
    os.environ["PFA_TIMESFM_ENABLED"] = "true"
    os.environ["PFA_TIMESFM_CHECKPOINT"] = CHECKPOINT
    from finance_assistant import behavior_model, daily_forecast, dashboard
    from finance_assistant.dashboard import DashboardService
    from finance_assistant.persistence.sqlite import SQLiteDerivedState

    # The service loads the model lazily; hand it the one already loaded.
    daily_forecast.TimesFMForecaster._loaded[CHECKPOINT] = model
    calls = []
    original = daily_forecast.TimesFMForecaster.quantiles

    def recording(self, contexts, horizon, covariates=None):
        rows = original(self, contexts, horizon, covariates)
        full = _predict(model, contexts, horizon, covariates)
        calls.append(
            {
                "contexts": [_list(c) for c in contexts],
                "horizon": horizon,
                "covariates": None
                if covariates is None
                else [None if c is None else _list(c) for c in covariates],
                "rows": [_list(r) for r in rows],
                "quantiles": [_list(q) for q in full],
            }
        )
        return rows

    daily_forecast.TimesFMForecaster.quantiles = recording
    # As the Rust parity goldens: no caches (they fail on a read-only
    # database) and no CatBoost outlook model.
    behavior_model._fit = lambda values, start, payday: None
    dashboard.cached_outlook = lambda path, key, build: build()
    dashboard.cached_daily_forecast = lambda path, key, build: build()
    state = SQLiteDerivedState(str(db), read_only=True)
    view = DashboardService(state, upstream_writes="disabled").view(now=now)
    (out / "golden_calls.json").write_text(json.dumps({"calls": calls}))
    daily = json.loads(view.model_dump_json())["daily_forecasts"]
    (out / "golden_dashboard.json").write_text(json.dumps({"daily_forecasts": daily}, indent=1))


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    model = _model()
    synthetic(model, out)
    internals(model, out)
    if len(sys.argv) > 2:
        now = datetime.fromisoformat(sys.argv[3] if len(sys.argv) > 3 else "2026-10-26T21:00:00+01:00")
        golden(model, out, Path(sys.argv[2]), now)


if __name__ == "__main__":
    main()
