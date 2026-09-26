# Tensor index: every open model as a tiny κ manifest over κ-addressed tensors

The hub stores tensors' addresses, not files. A file is a view computed from a small layout; a model is the
OCI address of its layouts. Every object is named by its sha256, so Hugging Face's own file digests, OCI digests
and IPFS raw CIDs stay one address, and stock clients (`oras`, `crane`, `huggingface_hub` via `HF_ENDPOINT`)
keep working.

## Objects

| Object | What it is | Where it lives |
|---|---|---|
| tensor κ | sha256 of a tensor's canonical bytes: contiguous, little-endian, unframed (strided pickle views are gathered) | nowhere new: its first source is the byte range inside the file Hugging Face already hosts; any other holder (another repo, another format, an IPFS object) serves the same κ |
| layout | one per file: the ordered segments that rebuild it byte for byte — literal (`l`, held), tensor payload (`t`), torch storage (`s`), or a view into a storage (`v`) | held by the hub, ~0.02% of the file |
| tensor table | the manifest's config: name, dtype, shape, κ of every tensor, plus the canonical model κ (sha256 over the sorted SET of `dtype|shape|κ`, names excluded) | held |
| manifest | OCI 1.1 image manifest, `artifactType application/vnd.hologram.model.v1`, one layer per file carrying the file's own digest and its layout | held, a few KB |
| index | OCI image index over the formats (`original`, `safetensors`, `safetensors-sharded`); tag `main` and the revision | held |
| day root | every gated model's index, the models that are the same weights, and the tensor-sharing edges between models | held; `latest.json` names it; `car.mjs` puts it on IPFS |

A format is just another layout over the same tensors, so a render costs no new data bytes, and its digest is
computed in the same single pass that hashes the original files.

## Run

```
export TENSOR_STATE=/path/to/state
node pipeline.mjs run --list pilot.txt      # index: plan every weight file, hash once, refuse any digest that differs from Hugging Face's
node pipeline.mjs gate                      # rebuild files from tensors (other holders first) and require the exact published sha256
node pipeline.mjs seal                      # the day's root over every gated model
node pipeline.mjs car                       # the root and every object it reaches as one verified CAR
node pipeline.mjs status
node pipeline.mjs nightly --budget-gb 200   # discover (the hub's /api/models) -> run -> gate -> seal -> car
IPFS_API=http://127.0.0.1:5001 node pipeline.mjs pin <repo>   # optional: payloads as IPFS objects, one per tensor
```

One runner at a time (a lock file); every stage is idempotent and resumes where it stopped.

## Provenance sample

Computed in the same single pass from the canonical bytes `hashpass.mjs` emits (`lib/sample.mjs`), and kept out of
the tensor table so the table stays small. One object per model, `application/vnd.hologram.provenance.v1+json`,
reachable from the day root (so it travels in the CAR) and served at `/v2/models/<owner>/<name>/provenance`:

    { v, repo, revision, method, rows: [[κ, shape, narrowDtype, narrow, signB64, blocks], ...] }   one row per distinct (κ, shape)

| Field | What it is |
|---|---|
| `narrowDtype`, `narrow` | the narrowest of BF16, F16, F32 that holds every value exactly, and sha256 of the payload in it. Equals κ when the tensor is already stored narrowest (the common case); an f32 file of bf16 values gets the bf16 tensor's digest. An equivalence key only, never a replacement for κ (and not `canonical`, the model κ) |
| `signB64` | the sign bits at element indices floor(k·n/4096), k < 4096, MSB-first: agreement between same-name, same-shape tensors measures lineage (independent training 49.9 %, fine-tunes, merges and edits 95.6-100 %, ten measured pairs) |
| `blocks` | [sha256, bytes] per 1024 rows of axis 0 of the narrow payload: a vocabulary resize keeps every earlier block. Rows cut along the shape, so a row is keyed by κ and shape (a reshape alias gets its own row) |

Float tensors only (BF16, F16, F32); GGUF quantised types and integers carry no sample. About 0.8 KB per distinct
tensor. `test/sample.test.mjs` holds the rules to vectors from the Python reference (`tensorhash.py`).

## Serve

`deploy/tensor-mirror.mjs` answers `/v2/models/<org>/<name>/…` (weight files 307 to Hugging Face while it serves)
and `/v2/tensors/<org>/<name>/…` (every file rebuilt from tensors). hub-resolve offers the same as the source
`tensors`, last in its order, so `HF_ENDPOINT` clients fail over to it and `/via/tensors/…` asks for it.
`deploy/tensor-assemble.mjs` is the one assembler (hub, CLI, service worker): it reads adjacent tensors in one
Range request, keeps four reads in flight, and releases each segment only after its κ checks; a source that
lies is skipped for the next holder.

One address, three uses. `gethologram.ai/qwen/qwen3-0.6b` is what the model page shows, what every client pulls,
and, opened in a browser, the model's page. Tags are formats: the bare name is safetensors, `:sharded` and
`:original` the others, `:index` (or the revision) the OCI index over all of them. Any other tag, such as an
Ollama quant, falls through to the hub's Ollama dialect. The manifest's IPFS address is the same sha256 as a raw CID.

```
oras pull gethologram.ai/qwen/qwen3-0.6b                 # safetensors; redirects to Hugging Face while it serves
oras pull gethologram.ai/tensors/qwen/qwen3-0.6b:sharded # every file rebuilt from its tensors
node pull.mjs Qwen/Qwen3-0.6B --format safetensors-sharded --out ./qwen   # assembled on this machine
```

## Ship (Ilya)

`deploy/tensor-sync.sh <state>` refuses unless `web/qa/tensor-index.mjs` passes, rsyncs to
`/root/hub/state/resolve/tensors`, swaps atomically (previous kept, `--rollback`), and proves one pull.
The CAR goes to Filebase with the existing `filebase-upload.sh`.
