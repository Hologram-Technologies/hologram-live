import json


def main(request: bytes) -> bytes:
    name = request.decode("utf-8").strip() or "Hologram"
    response = {
        "message": f"Hello, {name}!",
        "name": name,
        "runtime": "python",
    }
    return json.dumps(response, separators=(",", ":"), sort_keys=True).encode()
