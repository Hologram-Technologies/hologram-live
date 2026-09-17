#!/usr/bin/env python3
"""Rebuild a pulled Model Hub model from a hologram object store and check every file against both addresses.

    verify-pull.py <store root (…/registry)> <owner/name> [out dir]

Finds the model manifest (layer 0, format hologram.model-hub.model/v1) among the pulled blobs, concatenates each
file's chunks in order, and requires the whole-file SHA-256 (Hugging Face) and BLAKE3 (hologram) to match.
"""
import hashlib, json, os, sys

try:
    from blake3 import blake3  # optional; SHA-256 alone is authoritative against Hugging Face
except ImportError:
    blake3 = None

root, model_id = sys.argv[1], sys.argv[2]
out = sys.argv[3] if len(sys.argv) > 3 else None
blobs = os.path.join(root, "blobs", "blake3")

manifest = None
for name in os.listdir(blobs):
    path = os.path.join(blobs, name)
    if os.path.getsize(path) > 8 << 20:
        continue
    try:
        doc = json.load(open(path, "rb"))
    except Exception:
        continue
    if isinstance(doc, dict) and doc.get("format") == "hologram.model-hub.model/v1" and doc.get("id") == model_id:
        manifest = doc
        break
if not manifest:
    sys.exit(f"no model manifest for {model_id} in {blobs}")

bad = 0
for f in manifest["files"]:
    s256 = hashlib.sha256()
    b3 = blake3() if blake3 else None
    sink = None
    if out:
        target = os.path.join(out, f["path"])
        os.makedirs(os.path.dirname(target), exist_ok=True)
        sink = open(target, "wb")
    for chunk in f["chunks"]:
        with open(os.path.join(blobs, chunk.split(":", 1)[1]), "rb") as fh:
            data = fh.read()
        s256.update(data)
        if b3:
            b3.update(data)
        if sink:
            sink.write(data)
    if sink:
        sink.close()
    ok = f"sha256:{s256.hexdigest()}" == f["sha256"] and (not b3 or f"blake3:{b3.hexdigest()}" == f["blake3"])
    if not ok:
        bad += 1
        print(f"MISMATCH {f['path']}")
print(json.dumps({"id": model_id, "revision": manifest["revision"], "files": len(manifest["files"]), "mismatched": bad}))
sys.exit(1 if bad else 0)
