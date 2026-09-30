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

## Pieces: every byte checked, from any holder

**The piece list.** For every tensor or storage over 1 MiB, the hash pass also records the sha256 of each 1 MiB of it, aligned to its start.
- It is computed from the same read that computes the κ, so there is no second pass.
- The lists live in one object, `application/vnd.hologram.pieces.v1+json` (`{v, piece, of: {κ: [hex…]}}`), named by the tensor table.
- So the model's one address reaches every piece hash.
- The CAR carries it, and the audit checks that every payload over 1 MiB has a list of the right length.

**`deploy/kappa-get.mjs`** is the one primitive, `get(κ, range)`:
- It yields any byte range of a κ object, fetching only the pieces the range covers and checking each before release.
- It reads neighbouring pieces in one range request (up to 8 MiB).
- A piece's sha256 is its IPFS raw CID, so any gateway is a holder with no second index. A tensor of at most 1 MiB is its own piece: its κ is its CID.
- A holder that lies is blamed for one piece of one κ, and its other pieces are kept.
- One lie, or a read slower than 1 MB/s plus 10 s, sends a holder to the back of the line for the process.

**`deploy/tensor-assemble.mjs`** routes every pieced payload through `get`:
- A `Range` inside a large tensor no longer fetches the whole tensor.
- Nothing is released before it checks. The old path, streaming tensors over 256 MB and aborting on a mismatch after release, is used only for indexes without pieces.
- `pull.mjs --ipfs <gateway>`, the mirror's `TENSOR_IPFS_GATEWAYS` and the gate's `TENSOR_IPFS_GATEWAYS` add IPFS as a holder.

**Measured on SmolLM2-135M-Instruct `model.safetensors`** (269 MB), 2026-09-28:
- 272 tensors; 91 over 1 MiB, with a 22.6 KB pieces object.
- The piece lists equal an independent implementation (`HOLOGRAM/spikes/kappa-piece-verify`) for 91 of 91 tensors.

Through the mirror, `/v2/tensors/…/blobs/<sha256>`, IPFS on the same machine:

| Holders | Time | Result |
|---|---|---|
| Honest Hugging Face | 27–38 s | exact |
| Hugging Face down, IPFS only | 1.6–2.1 s | exact |
| Hugging Face lying on every answer and slow (8 MiB in 84 s) | 21.6 s | exact; before the slow-holder deadline it took 695 s, and every byte released was still correct |
| A 1 MB range, Hugging Face lying | 1.4 s | exact |

A direct download from Hugging Face took 8.7–25 s on the same link. The honest run is slower than direct because tensors are fetched one range per tensor; see Open.

**Open:**
- Honest-run speed: coalesce neighbouring pieced tensors into one range, as groups already do.
- A read that misses its deadline keeps downloading in the background; add an abort.
- Pieces for literals and small files.
- IPFS here was on the same machine; a remote gateway is not yet measured.

## Registry records, and the edge on your own machine

**Registry records.** `deploy/kappa-records.mjs` walks a model from its index κ and records every object it reaches:
- index, manifests, tensor table and pieces;
- files, layouts, literals, tensors and storages;
- the model κ and provenance.

Each record gives the object's type, size, piece list, `refs` (what points to it) and `holders`. A holder is one of: the hub, a Hugging Face byte range at a pinned commit, a URL, the raw CID (objects of at most 1 MiB), each piece's raw CID, or a layout.

The canonical model κ is now a held object: its bytes, the sorted `dtype|shape|tensor κ` lines, name every tensor κ.

Where records appear:
- `/v2/models/<repo>/records[/<κ>]` and `/v2/kappa/<κ>` on the mirror.
- Sealed into the day root, carried by the CAR, and required by the audit.

`check-dialects.mjs` calls every answer the mirror gives for every model, and fails on any sha256 that has no record. It also re-hashes every held record. SmolLM2-135M plus -Instruct, 2026-09-28: 2,988 digests in 60 answers, 0 failures.

**The edge.** `edge.mjs` is a verifying gateway on the user's machine:

```
node edge.mjs --hub https://gethologram.ai --port 8095 [--ipfs http://127.0.0.1:8080]
HF_ENDPOINT=http://127.0.0.1:8095 hf download HuggingFaceTB/SmolLM2-135M-Instruct
crane pull --insecure 127.0.0.1:8095/huggingfacetb/smollm2-135m-instruct:safetensors out.tar
```

It speaks the Hugging Face dialect (model info, `tree`, `treesize`, `refs`, `resolve` with `Range`), OCI, and Ollama (a 307 to the other loopback name).

It trusts one thing: which index κ a name points to. Everything below is checked:
- manifests and layouts against their κ;
- weight files rebuilt through `get(κ)` piece by piece;
- other files hashed while streaming, with the last 64 KiB held back and a mismatch cutting the connection.

Measured, 2026-09-28:
- **huggingface_hub 2.0 through the edge:** `model.safetensors` exact, all 415 pieces checked, plus an ONNX file. `hf cache verify` checked 13 files.
- **With Hugging Face lying:** exact, all 269 MB from IPFS.
- **`crane`:** 16/16 layers hash to their names.
- **Ollama path:** the blob hop is a 307 to the other loopback name, and the bytes hash to the digest.

## Decentralised: Filebase

Tensors pass through a small Kubo node on their way to Filebase, a few GB at a time, so any number of models fit
a small disk. `pin` streams each payload from Hugging Face into the node and refuses bytes that do not hash to their
κ. `filebase.mjs ship` then puts each batch up as one CAR. The CAR is a directory whose entry names are the κ and
whose links are the tensors' own CIDs. A batch counts only when Filebase reports our root CID and tensors read back
through Filebase's gateway hash to their κ. The node then unpins it, except for payloads named by `--keep`.

    IPFS_API=http://127.0.0.1:5101 PIN_CONCURRENCY=4 IPFS_ADD_CMD="docker exec -i hub-ipfs ipfs add -Q --cid-version=1 --raw-leaves --chunker=size-1048576 --pin" node pipeline.mjs pin --list all.txt --ungated --budget-gb 4
    node filebase.mjs ship --keep top10.txt --batch-gb 4    # FB_ENV names the Filebase key file; it is never logged
    node filebase.mjs map                                   # the κ -> CID map for payloads over 1 MiB, as one object
    node filebase.mjs put state/car/<day>.car tensor-index/<day>.car <root>   # the day index: every manifest

A payload of at most 1 MiB needs no map: its CID is the raw CID of its κ. The same holds for every manifest, config
and layout in the day index.

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

## Names log

Which name pointed at which bytes, when, witnessed by whom (`lib/names.mjs`). Append-only JSON lines, each carrying
the sha256 of the entry before it; the head is signed with the witness's Ed25519 key (`names.key`, 0600, never
published) and its public half travels with it. Entry kinds: `witnessed` (name, revision, {path: sha256}),
`indexed` (name, revision, index, table), `replaced` (same name, revision and path, a different sha256: a silent
replacement, also written to `replacements.json`), `moved` (a new revision). `run` records what it indexed;
`witness` (in `nightly`, after `discover`) records what Hugging Face serves under every indexed name now, from
the revision and LFS sha256s, no weight bytes. `seal` signs the head and names log and head in the day root, so
`car` carries them and `audit` replays the chain and checks the signature. `node pipeline.mjs names <repo>
[--at ISO]` answers what a name pointed to. It records per-file history, so it can replace `models.json`
`history` (kept for now).

## Ship (Ilya)

`deploy/tensor-sync.sh <state>` refuses unless `web/qa/tensor-index.mjs` passes, rsyncs to
`/root/hub/state/resolve/tensors`, swaps atomically (previous kept, `--rollback`), and proves one pull.
The CAR goes to Filebase with the existing `filebase-upload.sh`.
