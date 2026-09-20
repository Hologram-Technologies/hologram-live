// Builds web/data/models.json: trending Hugging Face models joined with the Hologram address index.
// Metadata only. No weight bytes are downloaded.
//
//   node apps/model-hub/web/scripts/data.mjs [--limit 500]
//
// HOLOGRAM_API may point at a local hologram-api checkout or a URL (default: its GitHub Pages site).

import { mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = join(dirname(fileURLToPath(import.meta.url)), "..");
const DATA = join(SITE, "data");
const AVATARS = join(SITE, "public", "avatars");
const HF = "https://huggingface.co";
const API = process.env.HOLOGRAM_API || "https://humuhumu33.github.io/hologram-api";
// Models whose bytes are stored on a Hologram registry, published as models/<owner>/<name>:<revision>.
const HUB = process.env.MODEL_HUB_REGISTRY || "hub.uor.foundation";
const LIMIT = Number(process.argv[process.argv.indexOf("--limit") + 1]) || 500;
// The Hugging Face token goes to huggingface.co and nowhere else: never to the hub, the address index or a gateway.
const headersFor = (url) => (process.env.HF_TOKEN && /^https:\/\/huggingface\.co\//.test(url) ? { authorization: `Bearer ${process.env.HF_TOKEN}` } : {});

async function get(url, as = "json", tries = 3) {
  if (!/^https?:/.test(url)) {
    const text = await readFile(url, "utf8").catch(() => null);
    return text == null ? null : as === "json" ? JSON.parse(text) : text;
  }
  for (let i = 0; i < tries; i++) {
    const res = await fetch(url, { headers: headersFor(url) }).catch(() => null);
    if (res?.ok) {
      try { return as === "json" ? await res.json() : as === "bytes" ? new Uint8Array(await res.arrayBuffer()) : await res.text(); }
      catch { return null; }
    }
    if (res && res.status !== 429 && res.status < 500) return null;
    await new Promise((r) => setTimeout(r, 1000 * 2 ** i));
  }
  return null;
}

async function pool(items, size, fn) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: size }, async () => {
    while (next < items.length) { const i = next++; out[i] = await fn(items[i], i); }
  }));
  return out;
}

const MODALITY = {
  "text-generation": "Text", "text2text-generation": "Text", "fill-mask": "Text", "translation": "Text",
  "summarization": "Text", "question-answering": "Text", "text-classification": "Text", "token-classification": "Text",
  "image-text-to-text": "Vision", "image-to-text": "Vision", "visual-question-answering": "Vision",
  "video-text-to-text": "Vision", "any-to-any": "Vision", "image-classification": "Vision", "object-detection": "Vision",
  "image-segmentation": "Vision", "zero-shot-image-classification": "Vision", "depth-estimation": "Vision",
  "text-to-image": "Image", "image-to-image": "Image", "unconditional-image-generation": "Image",
  "text-to-video": "Video", "image-to-video": "Video",
  "automatic-speech-recognition": "Audio", "text-to-speech": "Audio", "text-to-audio": "Audio",
  "audio-to-audio": "Audio", "audio-classification": "Audio", "audio-text-to-text": "Audio",
  "feature-extraction": "Embedding", "sentence-similarity": "Embedding",
};

const BUCKETS = [
  [1e9, "Under 1B"], [3.5e9, "1 to 3B"], [9.5e9, "4 to 9B"], [20.5e9, "10 to 20B"],
  [40.5e9, "21 to 40B"], [100.5e9, "41 to 100B"], [300.5e9, "101 to 300B"], [Infinity, "Over 300B"],
];

const LANGUAGE = new Intl.DisplayNames(["en"], { type: "language" });

function languages(tags) {
  const out = [];
  for (const t of tags || []) {
    if (!/^[a-z]{2}$/.test(t)) continue;
    const name = LANGUAGE.of(t);
    if (name && name !== t) out.push(name);
  }
  return out;
}

// "Qwen/Qwen3.6-35B-A3B" → "Qwen 3.6", "meta-llama/Llama-3.1-8B" → "Llama 3.1", "google/gemma-4-31B" → "Gemma 4".
function family(id, tags) {
  const base = (tags || []).map((t) => t.match(/^base_model:(?:[a-z_]+:)?([^/]+\/[^/]+)$/)?.[1]).find(Boolean);
  const name = (base || id).split("/")[1];
  const parts = name.split(/[-_ ]+/);
  let head = parts[0].match(/^([A-Za-z]{3,}?)[vV]?(\d+(?:\.\d+)?)$/);
  let label;
  if (head && !/^\d+(\.\d+)?[bBmMkK]$/.test(parts[0])) label = `${head[1]} ${head[2]}`;
  else if (/^[A-Za-z]+$/.test(parts[0]) && /^[vV]?\d+(\.\d+)?$/.test(parts[1] || "")) label = `${parts[0]} ${parts[1].replace(/^v/, "V")}`;
  else if (/^[A-Za-z][A-Za-z0-9.]+$/.test(parts[0])) label = parts[0];
  else return null;
  return label[0].toUpperCase() + label.slice(1);
}

function format(m) {
  const t = m.tags || [];
  if (t.includes("gguf") || m.gguf) return "GGUF";
  if (m.library_name === "mlx" || t.includes("mlx")) return "MLX";
  if (t.includes("safetensors")) return "Safetensors";
  if (t.includes("onnx")) return "ONNX";
  return "Other";
}

async function main() {
  const fields = ["author", "likes", "downloads", "trendingScore", "pipeline_tag", "library_name", "tags",
    "createdAt", "gated", "sha", "safetensors", "gguf"].map((f) => `expand[]=${f}`).join("&");
  const trending = await get(`${HF}/api/models?sort=trendingScore&direction=-1&limit=${LIMIT}&${fields}`);
  if (!Array.isArray(trending) || !trending.length) throw new Error("Hugging Face trending list is empty");

  const index = await get(`${API}/v1/index.json`);
  if (!index?.models) throw new Error(`no index at ${API}`);
  const indexed = new Map(index.models.map((m) => [m.name, m]));
  const sourceIndex = (await get(`${API}/v1/sources/index.json`))?.models || {};
  // Models pinned on IPFS by the hub (pin-model.sh): root CID per model revision, served by a CORS-open gateway.
  const pins = (await get(`https://${HUB}/pins.json`)) || { gateway: "https://ipfs.filebase.io/ipfs/", models: {} };
  // The archive ledger (archive.sh): one entry per captured day, hash-chained, each day a CAR on IPFS.
  const archive = await get(`https://${HUB}/archive.json`);
  await mkdir(DATA, { recursive: true }); // a fresh checkout (CI) has no data directory yet
  if (archive?.days?.length) await writeFile(join(DATA, "archive.json"), JSON.stringify(archive));
  else await rm(join(DATA, "archive.json"), { force: true });
  // A source counts for a model only when every weight file there is byte identical (every file, if none are weights).
  const complete = (s) => (s.weights ? s.weights_identical === s.weights : s.identical === s.files);

  await rm(join(DATA, "files"), { recursive: true, force: true });
  await rm(join(DATA, "overview"), { recursive: true, force: true });
  await mkdir(AVATARS, { recursive: true });

  const models = await pool(trending, 8, async (m, i) => {
    const [org, name] = m.id.split("/");
    const hit = indexed.get(m.id);
    const config = m.gguf ? null : await get(`${HF}/${m.id}/resolve/main/config.json`);
    const cfg = config?.text_config || config?.llm_config || config || {};
    const context = m.gguf?.context_length || cfg.max_position_embeddings || cfg.max_sequence_length || cfg.seq_length || null;
    const params = m.safetensors?.total || m.gguf?.total || null;
    const license = (m.tags || []).find((t) => t.startsWith("license:"))?.slice(8) || null;
    const row = {
      id: m.id, org, name, rank: i + 1,
      likes: m.likes || 0, downloads: m.downloads || 0,
      created: m.createdAt?.slice(0, 10) || null,
      modality: MODALITY[m.pipeline_tag] || "Other",
      task: m.pipeline_tag || null,
      params, bucket: params ? BUCKETS.find(([max]) => params < max)[1] : null,
      context: typeof context === "number" && context > 0 && context < 1e8 ? context : null,
      arch: m.gguf?.architecture || config?.model_type || null,
      family: family(m.id, m.tags),
      languages: languages(m.tags),
      library: m.library_name || null,
      format: format(m), license,
      state: hit ? "addressed" : index.skipped?.[m.id] ? "skipped" : "pending",
    };
    if (m.gated) row.state = "skipped";
    row.sources = hit ? null : [];
    if (hit) {
      const doc = await get(`${API}/v1/huggingface.co/${m.id}/latest.json`);
      Object.assign(row, { revision: hit.revision, manifest: hit.manifest, files: hit.files, weightBytes: hit.weight_bytes });
      const summary = sourceIndex[m.id] || [];
      const detail = summary.length > 1 ? await get(`${API}/v1/sources/huggingface.co/${m.id}.json`) : null;
      const sources = detail && detail.revision === hit.revision
        ? detail.sources.filter(complete).map((s) => ({ kind: s.kind, name: s.kind === "bittorrent" ? "P2P" : s.name, page: s.page, resolve: s.resolve || s.gateway || null, missing: s.missing || [], p2p: !!s.p2p }))
        : [{ kind: "huggingface.co", name: "Hugging Face", page: `https://huggingface.co/${m.id}`, resolve: null, missing: [] }];
      // Listed only when this exact revision is published there; every file is present by construction.
      const repository = `models/${m.id.toLowerCase()}`;
      const tags = await get(`https://${HUB}/v2/${repository}/tags/list`);
      if (tags?.tags?.includes(hit.revision)) {
        const reference = `${HUB}/${repository}:${hit.revision}`;
        sources.unshift({ kind: "hologram", name: "Hologram", page: `https://${HUB}/v2/${repository}/tags/list`, resolve: null, missing: [], pull: reference });
      }
      const pin = pins.models?.[m.id];
      if (pin && pin.revision === hit.revision && !sources.some((s) => s.kind === "ipfs")) {
        const root = `${pins.gateway}${pin.root}/`;
        // Pins made before 2026-09-18 were packed without dotfiles (ipfs-car skips hidden paths unless told otherwise;
        // measured: .gitattributes answered 404 on the gateway). pin-model.sh now packs them and marks the pin.
        const absent = pin.hidden ? [] : (doc?.files || []).map((f) => f.path).filter((p) => p.split("/").some((part) => part.startsWith(".")));
        sources.push({ kind: "ipfs", name: "IPFS", page: root, resolve: root, missing: absent });
      }
      row.sources = sources.map((s) => s.name);
      if (doc) {
        const file = join(DATA, "files", org, `${name}.json`);
        await mkdir(dirname(file), { recursive: true });
        await writeFile(file, JSON.stringify({ revision: doc.revision, manifest: doc.manifest, sources,
          files: doc.files.map((f) => [f.path, f.size, f.address, f.weights ? 1 : 0, f.url]) }));
      }
    }
    // Overview: the model passport from hologram-api, and the README at the same revision, checked by address.
    const overview = await get(`${API}/v1/overview/huggingface.co/${m.id}.json`);
    if (overview) {
      const dir = join(DATA, "overview", org);
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, `${name}.json`), JSON.stringify(overview));
      const pinned = overview.provenance?.readme?.match(/@sha256:([0-9a-f]{64})/)?.[1];
      if (pinned && overview.revision) {
        const readme = await get(`${HF}/${m.id}/resolve/${overview.revision}/README.md`, "bytes");
        if (readme && createHash("sha256").update(readme).digest("hex") === pinned) {
          await writeFile(join(dir, `${name}.md`), readme);
        }
      }
    }
    return row;
  });

  const orgs = [...new Set(models.map((m) => m.org))];
  const avatars = await pool(orgs, 8, async (org) => {
    for (const kind of ["organizations", "users"]) {
      const meta = await get(`${HF}/api/${kind}/${org}/avatar`, "json", 1);
      if (!meta?.avatarUrl) continue;
      const url = meta.avatarUrl.startsWith("http") ? meta.avatarUrl : `${HF}${meta.avatarUrl}`;
      const ext = (url.match(/\.(png|jpe?g|webp|gif|svg)(\?|$)/i)?.[1] || "png").toLowerCase();
      const bytes = await get(/cdn-avatars/.test(url) && ext !== "svg" ? `${url}?w=96` : url, "bytes", 2);
      if (!bytes || bytes.length > 200_000) return null;
      const file = `${org}.${ext}`;
      await writeFile(join(AVATARS, file), bytes);
      return file;
    }
    return null;
  });
  const avatarOf = new Map(orgs.map((o, i) => [o, avatars[i]]));
  for (const m of models) m.avatar = avatarOf.get(m.org) || null;

  const addressed = models.filter((m) => m.state === "addressed");
  const out = {
    snapshot: new Date().toISOString().slice(0, 10),
    source: { trending: `${HF}/api/models?sort=trendingScore`, index: "https://humuhumu33.github.io/hologram-api/v1/index.json" },
    totals: { models: models.length, addressed: addressed.length, weightBytes: addressed.reduce((s, m) => s + (m.weightBytes || 0), 0) },
    models,
  };
  await mkdir(DATA, { recursive: true });
  await writeFile(join(DATA, "models.json"), JSON.stringify(out));
  console.log(`${models.length} trending · ${addressed.length} addressed · ${avatars.filter(Boolean).length}/${orgs.length} avatars`);
  const unindexed = models.filter((m) => m.state === "pending").map((m) => m.id);
  await writeFile(join(DATA, "pending.txt"), unindexed.join("\n") + "\n");
}

main().catch((e) => { console.error(e); process.exit(1); });
