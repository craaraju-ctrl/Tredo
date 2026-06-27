from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from typing import List, Optional
import pandas as pd
from model import KronosPredictor
import uvicorn
import os

app = FastAPI(title="Kronos Forecasting Service for tredo", version="0.1.0")

# === Model Loading ===
# Set KRONOS_MODEL_PATH env var to use a local downloaded model
# Otherwise falls back to Hugging Face (first run will download)

print("[KronosService] Loading Kronos model...")
predictor = KronosPredictor()
print("[KronosService] Kronos model loaded successfully and ready.")

class OhlcvBar(BaseModel):
    timestamp: str
    open: float
    high: float
    low: float
    close: float
    volume: float

class ForecastRequest(BaseModel):
    symbol: str
    ohlcv: List[OhlcvBar]
    pred_len: int = 10
    temperature: float = 0.8
    top_p: float = 0.9
    sample_count: int = 1

class ForecastResponse(BaseModel):
    symbol: str
    forecasts: List[dict]
    message: str

@app.get("/health")
async def health():
    return {"status": "ok", "model": "chronos-bolt", "uptime_seconds": 0}

@app.post("/forecast", response_model=ForecastResponse)
async def get_forecast(request: ForecastRequest):
    try:
        # Convert input to DataFrame
        data = [bar.dict() for bar in request.ohlcv]
        df = pd.DataFrame(data)
        df['timestamp'] = pd.to_datetime(df['timestamp'], format='ISO8601')
        df = df.set_index('timestamp')

        # Generate future timestamps (simple example - adjust frequency as per your market)
        last_ts = df.index[-1]
        y_timestamp = pd.date_range(
            start=last_ts + pd.Timedelta(minutes=1),
            periods=request.pred_len,
            freq='1min'   # Change to '5min', '1H' etc. based on your data
        )

        # Call Kronos predictor
        pred_df = predictor.predict(
            df,
            x_timestamp=df.index,
            y_timestamp=y_timestamp,
            pred_len=request.pred_len,
            temperature=request.temperature,
            top_p=request.top_p,
            sample_count=request.sample_count
        )

        pred_df.index = pred_df.index.strftime('%Y-%m-%dT%H:%M:%SZ')
        pred_df.index.name = 'timestamp'
        forecasts = pred_df.reset_index().to_dict(orient='records')

        return ForecastResponse(
            symbol=request.symbol,
            forecasts=forecasts,
            message="Forecast generated successfully using Kronos"
        )
    except Exception as e:
        import traceback
        traceback.print_exc()
        raise HTTPException(status_code=500, detail=f"Kronos prediction failed: {str(e)}")

if __name__ == "__main__":
    port = int(os.getenv("KRONOS_PORT", "8000"))
    print(f"[KronosService] Starting on port {port} (from KRONOS_PORT env var)")
    uvicorn.run(app, host="0.0.0.0", port=port)
