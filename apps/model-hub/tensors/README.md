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

## Ship (Ilya)

`deploy/tensor-sync.sh <state>` refuses unless `web/qa/tensor-index.mjs` passes, rsyncs to
`/root/hub/state/resolve/tensors`, swaps atomically (previous kept, `--rollback`), and proves one pull.
The CAR goes to Filebase with the existing `filebase-upload.sh`.
