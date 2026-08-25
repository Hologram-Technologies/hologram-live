import json

import numpy as np
import pandas as pd


def main(request: bytes) -> bytes:
    payload = json.loads(request)
    frame = pd.DataFrame(payload["rows"])
    values = frame["value"].to_numpy(dtype=np.float64)
    result = {
        "columns": frame.columns.tolist(),
        "mean": float(np.mean(values)),
        "rows": int(frame.shape[0]),
        "sum": float(np.sum(values)),
    }
    return json.dumps(result, separators=(",", ":"), sort_keys=True).encode()
