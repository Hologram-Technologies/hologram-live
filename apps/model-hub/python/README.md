# hologram-verify

**What it does:** every file huggingface_hub hands your code is checked against Hologram's κ index before any library loads it.

**How to turn it on:**

```python
import hologram_verify; hologram_verify.enable()          # before transformers, diffusers, vLLM, mlx-lm …
import transformers
transformers.AutoModelForCausalLM.from_pretrained("HuggingFaceTB/SmolLM2-135M-Instruct")
```

Without code changes: `python -c "import hologram_verify.auto; …"`, or put `import hologram_verify.auto` in `sitecustomize`.

**What it checks:**
- Every path `hf_hub_download` returns. snapshot_download and every `from_pretrained` go through it.
- The path's sha256 is compared with the sha256 Hologram's index publishes for that exact commit.
- This holds whichever host sent the bytes: Hugging Face, a mirror, a proxy, or any `HF_ENDPOINT`.
- Cache hits are checked too, which huggingface_hub never does: once per file per process, keyed by size and mtime.

**What happens on a problem:**
- **A mismatch:** the file is deleted from the cache and `HologramVerificationError` is raised.
- **A commit Hologram hasn't indexed:** the file is reported as unverifiable. That's a warning by default, and an error with `strict=True` or `HOLOGRAM_VERIFY_STRICT=1`.
- **The expected hash never comes from the host that sent the bytes.** It comes from `hub` (`HOLOGRAM_HUB`, default `https://gethologram.ai`).

**Tests:**
- `python -m pytest tests/test_verify.py`: offline, 5 tests.
- `LIAR=http://127.0.0.1:18978 python tests/live_check.py`: real Hugging Face, plus a lying endpoint (`HOLOGRAM/spikes/kappa-piece-verify/hf-resolve-liar.mjs`).

Measured 2026-09-28 on SmolLM2-135M-Instruct, with huggingface_hub 1.33 plus transformers 5.17, and with huggingface_hub 2.0:
- **Honest downloads:** config, tokenizer and weights verified.
- **A cached file changed on disk:** refused and removed.
- **A lying endpoint:** its `config.json` and 269 MB `model.safetensors` refused and removed.
- **transformers:** `AutoConfig` and `AutoTokenizer.from_pretrained` go through the check.
