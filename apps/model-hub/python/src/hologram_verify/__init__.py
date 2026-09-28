"""hologram_verify: every file huggingface_hub hands your code is checked against Hologram's κ index first.

    import hologram_verify; hologram_verify.enable()      # before transformers, diffusers, vLLM, mlx-lm ...
    # or, with no code change:  HOLOGRAM_VERIFY=1 python -c "import hologram_verify.auto" ...

huggingface_hub checks only a file's length, and never re-checks its cache. With this enabled, every path
`hf_hub_download` returns (and `snapshot_download`, `from_pretrained` and friends go through it) is hashed and compared
with the sha256 Hologram's index publishes for that exact commit, whichever host served the bytes: Hugging Face, a
mirror, a proxy, an HF_ENDPOINT gateway. Cache hits are checked too (once per file per process, by size and mtime).

On a mismatch the file is deleted from the cache and HologramVerificationError is raised, so no library loads it.
A file Hologram has not indexed at that commit is reported as unverifiable: a warning, or with strict=True
(HOLOGRAM_VERIFY_STRICT=1) an error.

The expected hash never comes from the host that served the bytes: it comes from `hub` (default https://gethologram.ai),
whose digests are published, content-addressed, and (with the names log) signed.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import threading
import urllib.error
import urllib.request
import warnings
from pathlib import Path

__all__ = ["enable", "disable", "verify_file", "HologramVerificationError", "UnverifiableWarning"]
__version__ = "0.1.0"


class HologramVerificationError(OSError):
    """A downloaded or cached file does not hash to the sha256 Hologram's index names for it."""


class UnverifiableWarning(UserWarning):
    """Hologram's index has no digest for this file at this commit."""


_state = {"hub": None, "strict": False, "original": None, "seen": {}, "trees": {}}
_lock = threading.Lock()
_COMMIT = re.compile(r"^[0-9a-f]{40}$")


def _sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _tree(repo_id: str, commit: str) -> dict | None:
    """path -> sha256 for repo@commit from Hologram's index, or None when Hologram has not indexed that commit."""
    key = (repo_id, commit)
    with _lock:
        if key in _state["trees"]:
            return _state["trees"][key]
    url = f"{_state['hub']}/api/models/{repo_id}/tree/{commit}"
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": f"hologram-verify/{__version__}"}), timeout=30) as r:
            entries = json.load(r)
        tree = {e["path"]: (e.get("lfs") or {}).get("oid") or e["oid"] for e in entries if e.get("type") == "file"}
        tree = {p: d for p, d in tree.items() if re.fullmatch(r"[0-9a-f]{64}", d or "")}
    except urllib.error.HTTPError as e:
        if e.code != 404:
            raise
        tree = None
    with _lock:
        _state["trees"][key] = tree
    return tree


def _commit_of(path: str, local_dir) -> str | None:
    """The commit a returned file belongs to: from the cache layout, or from local_dir's download metadata."""
    parts = Path(path).parts
    if "snapshots" in parts:
        c = parts[parts.index("snapshots") + 1]
        if _COMMIT.match(c):
            return c
    if local_dir is not None:
        rel = os.path.relpath(path, local_dir)
        meta = Path(local_dir, ".cache", "huggingface", "download", rel + ".metadata")
        try:
            c = meta.read_text().splitlines()[0].strip()
            if _COMMIT.match(c):
                return c
        except OSError:
            pass
    return None


def verify_file(path: str, repo_id: str, filename: str, commit: str) -> bool:
    """Check one file against Hologram's index. True: verified. False: unverifiable. Raises on a mismatch."""
    st = os.stat(path)
    key = (os.path.realpath(path), st.st_size, st.st_mtime_ns)
    if _state["seen"].get(key):
        return True
    tree = _tree(repo_id, commit)
    want = tree.get(filename) if tree else None
    if not want:
        msg = f"{repo_id}@{commit[:12]}/{filename}: Hologram's index has no digest for it; not verified"
        if _state["strict"]:
            raise HologramVerificationError(msg)
        warnings.warn(msg, UnverifiableWarning, stacklevel=3)
        return False
    got = _sha256(path)
    if got != want:
        real = os.path.realpath(path)
        for p in {path, real}:
            try:
                os.remove(p)
            except OSError:
                pass
        raise HologramVerificationError(f"{repo_id}@{commit[:12]}/{filename}: sha256 {got[:16]}… is not the {want[:16]}… Hologram's index names; the file was removed")
    _state["seen"][key] = True
    return True


def _wrap(original):
    def hf_hub_download(repo_id, filename, *args, **kwargs):
        path = original(repo_id, filename, *args, **kwargs)
        if not isinstance(path, (str, os.PathLike)) or (kwargs.get("repo_type") or "model") != "model":
            return path                                                   # dry runs, datasets and spaces: untouched
        sub = kwargs.get("subfolder")
        name = f"{sub}/{filename}" if sub else filename
        commit = _commit_of(str(path), kwargs.get("local_dir"))
        if commit is None:
            msg = f"{repo_id}/{name}: the commit it came from is unknown; not verified"
            if _state["strict"]:
                raise HologramVerificationError(msg)
            warnings.warn(msg, UnverifiableWarning, stacklevel=2)
            return path
        verify_file(str(path), repo_id, name, commit)
        return path

    hf_hub_download.__wrapped__ = original
    hf_hub_download.__hologram_verify__ = True
    return hf_hub_download


def _swap(old, new) -> int:
    """Replace every module-level reference to `old` (huggingface_hub, transformers, diffusers … import it by name)."""
    n = 0
    for mod in list(sys.modules.values()):
        try:
            if getattr(mod, "hf_hub_download", None) is old:
                setattr(mod, "hf_hub_download", new)
                n += 1
        except Exception:
            pass
    return n


def enable(hub: str | None = None, strict: bool | None = None) -> int:
    """Check every file huggingface_hub returns. Returns how many module references were wrapped."""
    import huggingface_hub.file_download as fd
    import huggingface_hub._snapshot_download  # noqa: F401  (its imported name is swapped below)

    _state["hub"] = (hub or os.environ.get("HOLOGRAM_HUB") or "https://gethologram.ai").rstrip("/")
    _state["strict"] = strict if strict is not None else os.environ.get("HOLOGRAM_VERIFY_STRICT") == "1"
    current = fd.hf_hub_download
    if getattr(current, "__hologram_verify__", False):
        return _swap(current.__wrapped__, current)                       # already on; catch modules imported since
    _state["original"] = current
    return _swap(current, _wrap(current))


def disable() -> None:
    import huggingface_hub.file_download as fd
    current = fd.hf_hub_download
    if getattr(current, "__hologram_verify__", False):
        _swap(current, current.__wrapped__)
