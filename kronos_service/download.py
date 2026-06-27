#!/usr/bin/env python3
"""
Download Amazon Chronos-Bolt-Tiny to local HuggingFace cache.

Run once:  python3 download.py

The model is ~10MB and saved to ~/.cache/huggingface/hub/
After this, the service works fully offline.
"""
import sys
import os

import os
MODEL_ID = os.getenv("KRONOS_MODEL_PATH", "amazon/chronos-bolt-base")

def main():
    try:
        import torch
        from chronos import ChronosBoltPipeline
    except ImportError as e:
        print(f"❌ Missing dependency: {e}")
        print("Run: pip3 install chronos-forecasting")
        sys.exit(1)

    print(f"[Download] ⬇ Fetching {MODEL_ID} from HuggingFace Hub...")
    print(f"[Download]   This is ~10MB and needs internet once.")
    print()

    try:
        pipeline = ChronosBoltPipeline.from_pretrained(
            MODEL_ID,
            device_map="cpu",
            torch_dtype=torch.float32,
        )
    except Exception as e:
        print(f"❌ Download failed: {e}")
        sys.exit(1)

    # Quick smoke-test: forecast 3 bars from a fake series
    print("[Download] 🧪 Running smoke test...")
    context = torch.tensor([100.0, 101.0, 102.0, 101.5, 103.0], dtype=torch.float32)
    forecast = pipeline.predict(context=context.unsqueeze(0), prediction_length=3)
    closes = forecast[0][forecast[0].shape[0] // 2].numpy()  # median

    print(f"[Download] ✅ Smoke test passed!")
    print(f"[Download]   Input:  [100, 101, 102, 101.5, 103]")
    print(f"[Download]   Output: {[f'{c:.2f}' for c in closes]}")
    print()

    cache_dir = os.path.join(os.path.expanduser("~"), ".cache", "huggingface", "hub")
    print(f"[Download] 📁 Model cached at: {cache_dir}")
    print(f"[Download] ✅ Kronos is ready — start the service with: python3 main.py")

if __name__ == "__main__":
    main()
