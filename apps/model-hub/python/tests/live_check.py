"""Live: real clients, real Hugging Face, a lying endpoint.

    LIAR=http://127.0.0.1:18978 python tests/live_check.py        (the liar: spikes/kappa-piece-verify/hf-resolve-liar.mjs)

1. honest Hugging Face: config, tokenizer and weights of SmolLM2-135M-Instruct pass, checked against gethologram.ai
2. a cached file changed on disk: the next call refuses it and removes it
3. a lying HF_ENDPOINT (API honest, every file body one byte off): refused, removed, before any library sees it
4. transformers: AutoConfig / AutoTokenizer.from_pretrained go through the same check
Downloads go to a temporary HF_HOME that is deleted at the end.
"""
import os
import shutil
import sys
import tempfile
import warnings

HOME = tempfile.mkdtemp(prefix="hv-")
os.environ["HF_HOME"] = HOME
os.environ["HF_HUB_DISABLE_PROGRESS_BARS"] = "1"
warnings.simplefilter("ignore")
M = "HuggingFaceTB/SmolLM2-135M-Instruct"
fails = 0


def endpoint(url):
    import huggingface_hub.constants as c
    os.environ["HF_ENDPOINT"] = url
    c.ENDPOINT = url
    c.HUGGINGFACE_CO_URL_TEMPLATE = url + "/{repo_id}/resolve/{revision}/{filename}"
    shutil.rmtree(os.path.join(HOME, "hub"), ignore_errors=True)


def check(name, ok, detail=""):
    global fails
    fails += not ok
    print(("ok   " if ok else "FAIL ") + name, detail)


try:
    import hologram_verify as hv
    print("wrapped references:", hv.enable())
    import huggingface_hub
    from huggingface_hub import hf_hub_download, snapshot_download
    print("huggingface_hub", huggingface_hub.__version__)

    endpoint("https://huggingface.co")
    p = snapshot_download(M, allow_patterns=["config.json", "tokenizer*.json", "model.safetensors"])
    check("1 honest Hugging Face: every file verified", len(hv._state["seen"]) >= 4, f"{len(hv._state['seen'])} files")

    real = os.path.realpath(os.path.join(p, "config.json"))
    with open(real, "ab") as f:
        f.write(b" ")
    try:
        hf_hub_download(M, "config.json")
        check("2 a changed cached file is refused", False)
    except hv.HologramVerificationError as e:
        check("2 a changed cached file is refused", not os.path.exists(real), str(e)[:90])

    liar = os.environ.get("LIAR")
    if liar:
        endpoint(liar)
        for f in ["config.json", "model.safetensors"]:
            try:
                hf_hub_download(M, f)
                check(f"3 a lying endpoint's {f} is refused", False)
            except hv.HologramVerificationError as e:
                check(f"3 a lying endpoint's {f} is refused", True, str(e)[:90])
    else:
        print("skip 3 (set LIAR)")

    try:
        import transformers
    except ImportError:
        print("skip 4 (no transformers)")
    else:
        endpoint("https://huggingface.co")
        hv.enable()
        before = len(hv._state["seen"])
        c = transformers.AutoConfig.from_pretrained(M)
        t = transformers.AutoTokenizer.from_pretrained(M)
        check("4 transformers from_pretrained goes through the check", len(hv._state["seen"]) > before, f"{c.model_type}, {len(hv._state['seen']) - before} more files verified")
finally:
    shutil.rmtree(HOME, ignore_errors=True)
print("failures:", fails)
sys.exit(1 if fails else 0)
