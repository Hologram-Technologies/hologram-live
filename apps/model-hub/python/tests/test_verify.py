"""Offline: the wrapper against a fake hf_hub_download and a fake index.  python -m pytest -q tests/test_verify.py"""
import hashlib
import os
import warnings

import pytest

import hologram_verify as hv

COMMIT = "a" * 40


@pytest.fixture()
def cache(tmp_path, monkeypatch):
    snap = tmp_path / "models--o--m" / "snapshots" / COMMIT
    snap.mkdir(parents=True)
    good = b'{"model_type": "llama"}'
    (snap / "config.json").write_bytes(good)
    index = {("o/m", COMMIT): {"config.json": hashlib.sha256(good).hexdigest()}}
    monkeypatch.setattr(hv, "_tree", lambda repo, commit: index.get((repo, commit)))
    hv._state.update(hub="https://example.invalid", strict=False, seen={}, trees={})
    fake = lambda repo_id, filename, *a, **k: str(snap / filename)
    return snap, hv._wrap(fake)


def test_a_right_file_passes_and_is_remembered(cache):
    snap, dl = cache
    assert dl("o/m", "config.json") == str(snap / "config.json")
    assert len(hv._state["seen"]) == 1


def test_a_changed_cached_file_is_refused_and_removed(cache):
    snap, dl = cache
    dl("o/m", "config.json")
    with open(snap / "config.json", "ab") as f:
        f.write(b" ")                                    # new size and mtime: the remembered check no longer applies
    with pytest.raises(hv.HologramVerificationError, match="removed"):
        dl("o/m", "config.json")
    assert not (snap / "config.json").exists()


def test_a_file_the_index_lacks_warns_or_refuses(cache):
    snap, dl = cache
    (snap / "extra.bin").write_bytes(b"x")
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        dl("o/m", "extra.bin")
    assert any(issubclass(x.category, hv.UnverifiableWarning) for x in w)
    hv._state["strict"] = True
    with pytest.raises(hv.HologramVerificationError, match="no digest"):
        dl("o/m", "extra.bin")


def test_the_commit_comes_from_the_path_or_local_dir_metadata(tmp_path):
    assert hv._commit_of(str(tmp_path / "snapshots" / COMMIT / "x"), None) == COMMIT
    meta = tmp_path / ".cache" / "huggingface" / "download"
    meta.mkdir(parents=True)
    (meta / "w.bin.metadata").write_text(f"{COMMIT}\netag\n0\n")
    assert hv._commit_of(str(tmp_path / "w.bin"), str(tmp_path)) == COMMIT
    assert hv._commit_of(str(tmp_path / "w.bin"), None) is None


def test_datasets_and_dry_runs_are_left_alone(cache):
    snap, _ = cache
    dl = hv._wrap(lambda *a, **k: object())
    assert not isinstance(dl("o/m", "config.json"), str)
    dl2 = hv._wrap(lambda *a, **k: str(snap / "nope"))
    assert dl2("o/d", "x", repo_type="dataset").endswith("nope")
