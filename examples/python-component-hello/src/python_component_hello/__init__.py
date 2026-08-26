import json
import sys


def main(request: bytes) -> bytes:
    name = request.decode("utf-8").strip() or "Hologram"
    response = {
        "message": f"Hello, {name}!",
        "name": name,
        "python": sys.version.split()[0],
        "runtime": "python-component",
    }
    return json.dumps(response, separators=(",", ":"), sort_keys=True).encode()
