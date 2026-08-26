import json

import six


def main(request: bytes) -> bytes:
    name = six.ensure_text(request, encoding="utf-8").strip() or "Hologram"
    response = {
        "dependency": f"six-{six.__version__}",
        "message": f"Hello, {name}!",
        "name": name,
        "runtime": "python-component",
    }
    return json.dumps(response, separators=(",", ":"), sort_keys=True).encode()
