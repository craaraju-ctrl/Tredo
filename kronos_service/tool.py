#!/usr/bin/env python3
"""
Kronos Forecasting Tool — Standalone CLI utility.
Run: python3 tool.py --closes "100.0,101.0,102.0,101.5,103.0" --pred-len 5
"""
import sys
import json
import argparse
import pandas as pd
import numpy as np

# Ensure model module is discoverable
import os
sys.path.append(os.path.dirname(os.path.abspath(__file__)))

try:
    from model import KronosPredictor
except ImportError as e:
    print(json.dumps({"error": f"Import error: {e}. Check python path."}))
    sys.exit(1)

def main():
    parser = argparse.ArgumentParser(description="Kronos Time-Series Forecasting Tool")
    parser.add_argument("--closes", type=str, required=True, help="Comma-separated close prices")
    parser.add_argument("--pred-len", type=int, default=5, help="Number of bars to forecast")

    args = parser.parse_args()

    try:
        closes = [float(x.strip()) for x in args.closes.split(",") if x.strip()]
    except Exception as e:
        print(json.dumps({"error": f"Invalid closes format: {e}"}))
        sys.exit(1)

    if not closes:
        print(json.dumps({"error": "Empty closes list"}))
        sys.exit(1)

    # Build DataFrame from input closes
    df = pd.DataFrame({
        "open": closes,
        "high": [c * 1.001 for c in closes],
        "low": [c * 0.999 for c in closes],
        "close": closes,
        "volume": [1000.0] * len(closes)
    })
    df.index = pd.date_range(start="2026-06-12T00:00:00Z", periods=len(closes), freq="1min")

    y_timestamp = pd.date_range(
        start=df.index[-1] + pd.Timedelta(minutes=1),
        periods=args.pred_len,
        freq="1min"
    )

    try:
        predictor = KronosPredictor()
        pred_df = predictor.predict(
            df,
            x_timestamp=df.index,
            y_timestamp=y_timestamp,
            pred_len=args.pred_len
        )

        pred_df.index = pred_df.index.strftime('%Y-%m-%dT%H:%M:%SZ')
        pred_df.index.name = 'timestamp'
        forecasts = pred_df.reset_index().to_dict(orient='records')

        result = {
            "symbol": "CLI",
            "forecasts": forecasts,
            "using_real_model": predictor.using_real_model,
            "message": "Forecast generated successfully using Kronos CLI"
        }
        print(json.dumps(result, indent=2))

    except Exception as e:
        print(json.dumps({"error": f"Prediction failed: {e}"}))
        sys.exit(1)

if __name__ == "__main__":
    main()
