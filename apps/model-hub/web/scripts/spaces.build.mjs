// Spaces — turn three Hugging Face static Spaces into sealed, self-contained holospace artifacts under
// public/spaces/, each ONE κ: the sha-256 of its OCI manifest, which is also its address on the registry
// (`Docker-Content-Digest`) and the annotation-carried root of its IPFS tree.
//
//   node scripts/spaces.build.mjs <hf-space-sources> <holo-os usr/lib/holo>
//
// The transform is small, recorded here, and deterministic (same sources → same bytes → same κ):
//   1. root-absolute asset paths become relative; the runtime (verified fetch, store, style, the Space's
//      ONNX Runtime pin) is referenced at ./runtime/… INSIDE the artifact — the site's build copies the
//      shared public/spaces/runtime/ into each dist/spaces/<id>/runtime/, the artifact carries it as layers
//   2. every third-party load is removed (no CDN script, no HF favicon); a CSP written into the page allows
//      only its own files, the hub's brand kit and the model hosts it declares
//   3. a prelude installs the verified fetch before the bundle runs, with the model files' digests SEALED
//      here (read once from the hub's index — sha-256 for every file — or from huggingface.co: sha-256 for
//      LFS files, git blob sha-1 for small ones); a byte that does not re-derive is refused; a verified byte
//      is kept in the Space's own OPFS store; progress goes out on a BroadcastChannel
//   4. the brand kit's tokens and type are applied; the source Space's headline/branding is hidden
//   5. the artifact is sealed: manifest.json (canonical OCI manifest, config = holospace.json, one layer per
//      file incl. the runtime), holospace.lock.json (every file: sha-256 = OCI digest = raw CID digest for
//      files ≤ 1 MiB, the recipe CID for larger; root = κ = sha-256 of manifest.json), IPFS root CID of the
//      artifact tree (kubo, recipe unixfs-v1-2025: CIDv1, sha2-256, raw leaves, 1 MiB chunks)
// The catalog public/spaces/spaces.json is written from the same pass. Needs: node, kubo `ipfs` on PATH,
// network (the model indexes). Nothing here runs at site build time.
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, readdirSync, statSync, rmSync, existsSync, mkdtempSync, cpSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname, relative } from "node:path";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const [SRC, OSLIB] = process.argv.slice(2);
if (!SRC || !OSLIB) { console.error("usage: node scripts/spaces.build.mjs <hf-space-sources> <holo-os usr/lib/holo>"); process.exit(1); }
const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(WEB, "public", "spaces");
const RUNTIME = join(OUT, "runtime");
mkdirSync(RUNTIME, { recursive: true });
for (const f of ["holo-spaces-hf-fetch.mjs", "holo-opfs-kappastore.mjs"]) copyFileSync(join(OSLIB, f), join(RUNTIME, f));
// ONNX Runtime, pinned per Space (see `ort` below), vendored from <sources>/ort/<pin>/ — the exact files the
// bundles would otherwise fetch from jsDelivr, so a Space loads nothing from a CDN.
if (existsSync(join(SRC, "ort"))) for (const pin of readdirSync(join(SRC, "ort"))) {
  mkdirSync(join(RUNTIME, "ort", pin), { recursive: true });
  for (const f of readdirSync(join(SRC, "ort", pin))) copyFileSync(join(SRC, "ort", pin, f), join(RUNTIME, "ort", pin, f));
}

const HUB = "https://hub.uor.foundation", HF = "https://huggingface.co";
// connect-src: the model hosts and the places their `resolve` redirects to (the LFS/Xet CDNs). Nothing else.
const CONNECT = `'self' ${HUB} ${HF} https://*.huggingface.co https://*.hf.co`;
const csp = `default-src 'self'; script-src 'self' blob: 'wasm-unsafe-eval' 'unsafe-inline'; worker-src 'self' blob:; connect-src ${CONNECT}; img-src 'self' blob: data:; media-src 'self' blob: data:; style-src 'self' ${HUB} 'unsafe-inline'; font-src 'self' ${HUB}`;

const SPACES = [
  {
    id: "kokoro-tts", name: "Kokoro TTS", task: "Speech", src: "kokoro-webgpu",
    tagline: "Type a sentence, hear it. 82M parameters, on your GPU.",
    source: "https://huggingface.co/spaces/webml-community/kokoro-webgpu",
    models: ["onnx-community/Kokoro-82M-v1.0-ONNX"], modelBytes: 325532232 + 522240,
    page: "assets/index-DQNH7Ttg.js", worker: "assets/worker-BJyeiA2_.js", workerRef: "/assets/worker-BJyeiA2_.js",
    ort: "transformers-3.3.3", ortPath: "wasmPaths=`https://cdn.jsdelivr.net/npm/@huggingface/transformers@${z.env.version}/dist/`",
    icon: `<path d="M32 20v24M24 26v12M40 26v12M16 30v4M48 30v4"/>`,
  },
  {
    id: "remove-background", name: "Remove Background", task: "Image", src: "remove-background-webgpu",
    tagline: "Drop a photo; the subject comes back cut out. RMBG 1.4.",
    source: "https://huggingface.co/spaces/Xenova/remove-background-webgpu",
    models: ["briaai/RMBG-1.4"], modelBytes: 176220000,
    page: "assets/index-17_Va4sS.js", prelude: "page",
    ort: "onnxruntime-web-1.17.1", ortIn: "page", ortPath: 'wasmPaths="https://cdn.jsdelivr.net/npm/onnxruntime-web@1.17.1/dist/"',
    example: "https://images.pexels.com/photos/5965592/pexels-photo-5965592.jpeg?auto=compress&cs=tinysrgb&w=1024",   // Pexels licence: free to use; vendored as example.jpg
    icon: `<rect x="16" y="18" width="32" height="28" rx="4"/><circle cx="27" cy="29" r="4"/><path d="M18 44l10-10 6 6 6-8 8 12"/>`,
  },
  {
    id: "smollm-chat", name: "SmolLM Chat", task: "Chat", src: "smollm-webgpu",
    tagline: "A 360M-parameter assistant. The whole conversation stays on your device.",
    source: "https://huggingface.co/spaces/webml-community/smollm-webgpu",
    models: ["HuggingFaceTB/SmolLM-360M-Instruct"], modelBytes: 386700000,
    page: "assets/index-DJnDi8CK.js", worker: "assets/worker-ClBgkh03.js", workerRef: "/assets/worker-ClBgkh03.js",
    ort: "transformers-3.0.0-alpha.7", ortPath: "wasmPaths=`https://cdn.jsdelivr.net/npm/@huggingface/transformers@${H.env.version}/dist/`",
    icon: `<path d="M18 22h28v18H30l-8 7v-7h-4z"/><path d="M26 31h12"/>`,
  },
];

const sha256 = (b) => createHash("sha256").update(b).digest("hex");
const walk = (dir, base = dir, out = []) => { for (const n of readdirSync(dir)) { const p = join(dir, n); if (statSync(p).isDirectory()) walk(p, base, out); else out.push(relative(base, p).split("\\").join("/")); } return out.sort(); };
// Canonical JSON: keys sorted at every level, no whitespace — one byte sequence per document, so the
// manifest digest (κ) is a function of its content and nothing else.
const canon = (v) => Array.isArray(v) ? "[" + v.map(canon).join(",") + "]" : v && typeof v === "object" ? "{" + Object.keys(v).sort().map((k) => JSON.stringify(k) + ":" + canon(v[k])).join(",") + "}" : JSON.stringify(v);
const mediaType = (p) => /\.html$/.test(p) ? "text/html" : /\.(m?js)$/.test(p) ? "text/javascript" : /\.css$/.test(p) ? "text/css" : /\.svg$/.test(p) ? "image/svg+xml" : /\.json$/.test(p) ? "application/json" : /\.wasm$/.test(p) ? "application/wasm" : /\.(png|jpg|jpeg)$/.test(p) ? `image/${p.split(".").pop().replace("jpg", "jpeg")}` : "application/octet-stream";

// ── the model index, read once and sealed: hub first (sha-256 for every file), huggingface.co otherwise
//    (sha-256 under `lfs` for large files, git blob sha-1 in `oid` for small ones). Same contract as the
//    runtime's indexFromTree, so what is sealed here is what the runtime would have read live. ──
async function modelIndex(id) {
  for (const host of [HUB, HF]) {
    try {
      const r = await fetch(`${host}/api/models/${id}/tree/main?recursive=true`);
      if (!r.ok) continue;
      const files = {};
      for (const e of await r.json()) {
        if (!e || e.type !== "file" || !e.path) continue;
        const lfs = e.lfs && /^[0-9a-f]{64}$/i.test(String(e.lfs.oid || "")) ? String(e.lfs.oid).toLowerCase() : null;
        const oid = String(e.oid || "").toLowerCase();
        if (lfs) files[e.path] = { axis: "sha256", hex: lfs, size: e.size ?? null };
        else if (/^[0-9a-f]{64}$/.test(oid)) files[e.path] = { axis: "sha256", hex: oid, size: e.size ?? null };
        else if (/^[0-9a-f]{40}$/.test(oid)) files[e.path] = { axis: "gitsha1", hex: oid, size: e.size ?? null };
      }
      if (Object.keys(files).length) return { host, files };
    } catch {}
  }
  throw new Error("no index for " + id);
}

// ── IPFS addresses by the recipe, with kubo (no network: --only-hash). Per file and for the tree root. ──
function ipfsAddresses(dir) {
  const r = spawnSync("ipfs", ["add", "-r", "--only-hash", "--cid-version", "1", "--raw-leaves", "--chunker", "size-1048576", "--quieter=false", dir], { encoding: "utf8", maxBuffer: 64 << 20 });
  if (r.status !== 0) throw new Error("ipfs add failed: " + (r.stderr || r.stdout));
  const base = dir.split(/[\\/]/).pop();
  const files = {}; let root = null;
  for (const line of r.stdout.split(/\r?\n/)) {
    const m = line.match(/^added (\S+) (.*)$/); if (!m) continue;
    const rel = m[2].replace(/\\/g, "/");
    if (rel === base) root = m[1]; else if (rel.startsWith(base + "/")) files[rel.slice(base.length + 1)] = m[1];
  }
  if (!root) throw new Error("ipfs add: no root for " + dir);
  return { root, files };
}
// A raw-leaf CIDv1 (sha2-256) carries the file's sha-256 verbatim: bafkrei… = base32(0x01 0x55 0x12 0x20 <32 bytes>).
const b32 = (buf) => { const A = "abcdefghijklmnopqrstuvwxyz234567"; let bits = 0, v = 0, out = ""; for (const byte of buf) { v = (v << 8) | byte; bits += 8; while (bits >= 5) { out += A[(v >>> (bits - 5)) & 31]; bits -= 5; } } if (bits) out += A[(v << (5 - bits)) & 31]; return out; };
const rawCid = (hex) => "b" + b32(Buffer.concat([Buffer.from([0x01, 0x55, 0x12, 0x20]), Buffer.from(hex, "hex")]));

// The prelude, in the two shapes it takes: a module script in the page, or the head of a module worker.
// `rel` is the path from the file that carries it to the Space's runtime/.
const prelude = (s, rel, pinned) => `// ── Hologram Spaces prelude: every model byte is verified against a digest sealed with this Space before
// the runtime sees it, then kept in this Space's own store by digest. Hosts are locations, never identity.
import { installVerifiedFetch } from "${rel}runtime/holo-spaces-hf-fetch.mjs";
import { OpfsKappaStore } from "${rel}runtime/holo-opfs-kappastore.mjs";
const __holoStoreP = OpfsKappaStore.open(${JSON.stringify("holo-space:" + s.id)}).catch(() => null);
const __holoChan = (() => { try { return new BroadcastChannel(${JSON.stringify("holo-spaces:" + s.id)}); } catch { return null; } })();
const __holoSay = (e) => { try { __holoChan && __holoChan.postMessage(e); } catch {} };
// a failure inside this context is a fact the page should show, not a silent spinner
self.addEventListener("error", (ev) => __holoSay({ kind: "error", error: String((ev && ev.message) || ev) }));
self.addEventListener("unhandledrejection", (ev) => __holoSay({ kind: "error", error: String((ev && ev.reason && (ev.reason.message || ev.reason)) || ev) }));
installVerifiedFetch({
  hosts: ${JSON.stringify([HUB, HF])}, altHosts: ${JSON.stringify([HUB, HF])}, models: ${JSON.stringify(s.models)},
  pinned: ${JSON.stringify(pinned)},
  store: { getByKey: async (a, h) => { const st = await __holoStoreP; return st ? st.getByKey(a, h) : null; },
           putVerified: async (a, h, b) => { const st = await __holoStoreP; if (st) await st.putVerified(a, h, b); } },
  onEvent: __holoSay,
}).then(() => __holoSay({ kind: "armed" }), (e) => __holoSay({ kind: "index-failed", error: String(e && e.message || e) }));
`;

const catalog = [];
for (const s of SPACES) {
  const from = join(SRC, s.src), to = join(OUT, s.id);
  rmSync(to, { recursive: true, force: true }); mkdirSync(join(to, "assets"), { recursive: true });
  const pinned = {}; const modelHosts = {};
  for (const id of s.models) { const ix = await modelIndex(id); pinned[id] = ix.files; modelHosts[id] = ix.host; }

  // the page
  let html = readFileSync(join(from, "index.html"), "utf8");
  html = html.replace(/\s+crossorigin(?=[\s>])/g, "");
  html = html.replace(/(src|href)="\/assets\//g, '$1="./assets/');
  html = html.replace(/<link rel="icon"[^>]*>\s*/g, "");
  html = html.replace(/<script[^>]*src="https?:\/\/[^"]*"[^>]*>\s*<\/script>\s*/g, "");
  // MathJax came from a CDN; the chat calls MathJax.typeset() after each reply. A no-op stub keeps the app whole.
  html = html.replace(/<script>\s*window\.MathJax[\s\S]*?<\/script>\s*/g, `<script>window.MathJax={typeset(){},typesetPromise(){return Promise.resolve()},startup:{promise:Promise.resolve()}};</script>\n`);
  html = html.replace(/<h1>[\s\S]*?<\/h1>\s*/g, "").replace(/<h4>[\s\S]*?<\/h4>\s*/g, "");
  html = html.replace(/<title>[^<]*<\/title>/, `<title>${s.name} · Spaces · Hologram</title>`);
  html = html.replace(/<html lang="en">/i, `<html lang="en" class="dark" data-theme="dark" data-space="${s.id}">`);
  const head = [
    `<meta http-equiv="Content-Security-Policy" content="${csp}">`,
    `<link rel="icon" href="./icon.svg" type="image/svg+xml">`,
    `<link rel="stylesheet" href="${HUB}/kit/hologram-warm.css">`,
    `<link rel="stylesheet" href="${HUB}/kit/hologram-gap-tokens.css">`,
    `<link rel="stylesheet" href="${HUB}/tokens.css">`,
    ...(s.prelude === "page" ? [`<script type="module">${prelude(s, "./", pinned)}</script>`] : []),
  ].join("\n");
  html = html.replace(/(<meta name="viewport"[^>]*>)/i, `$1\n${head}`);
  html = html.replace(/<\/head>/i, `<link rel="stylesheet" href="./runtime/space.css">\n</head>`);
  if (/(\ssrc|<link[^>]*\shref)=["']https?:\/\/(?!hub\.uor\.foundation\/)/i.test(html)) throw new Error(s.id + ": index.html still loads a foreign origin");
  writeFileSync(join(to, "index.html"), html);

  // the bundles
  const ortFrom = (rel) => `new URL(${JSON.stringify(`${rel}runtime/ort/${s.ort}/`)},self.location.href).href`;   // origin-qualified from the file's own URL
  for (const f of walk(from)) {
    if (f === "index.html" || f === "README.md" || f.startsWith(".")) continue;
    const dest = join(to, f); mkdirSync(dirname(dest), { recursive: true });
    if (f === s.page) {
      let js = readFileSync(join(from, f), "utf8");
      if (s.workerRef) { const b = js.length; js = js.split(`new URL("${s.workerRef}",import.meta.url)`).join(`new URL("./${s.worker.split("/").pop()}",import.meta.url)`); if (js.length === b) throw new Error(s.id + ": worker URL rewrite did not apply"); }
      if (s.example) { const b = js.length; js = js.split(s.example).join("./example.jpg"); if (js.length === b) throw new Error(s.id + ": example URL rewrite did not apply"); }
      if (s.ortIn === "page") { const b = js.length; js = js.split(s.ortPath).join(`wasmPaths=${ortFrom("./")}`); if (js.length === b) throw new Error(s.id + ": ort path rewrite did not apply (page)"); }
      writeFileSync(dest, js);
    } else if (f === s.worker) {
      let js = readFileSync(join(from, f), "utf8");
      const b = js.length; js = js.split(s.ortPath).join(`wasmPaths=${ortFrom("../")}`); if (js.length === b) throw new Error(s.id + ": ort path rewrite did not apply (worker)");
      writeFileSync(dest, prelude(s, "../", pinned) + js);
    } else copyFileSync(join(from, f), dest);
  }
  writeFileSync(join(to, "icon.svg"), `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">${s.icon}</svg>\n`);

  // the manifest config: what this Space is, what it needs, what it runs on
  const runtimeFiles = ["runtime/holo-spaces-hf-fetch.mjs", "runtime/holo-opfs-kappastore.mjs", "runtime/space.css", ...readdirSync(join(RUNTIME, "ort", s.ort)).map((f) => `runtime/ort/${s.ort}/${f}`)].sort();
  const config = {
    format: "hologram.space/v1", id: s.id, name: s.name, task: s.task, tagline: s.tagline, entry: "index.html",
    space: { source: s.source, sdk: "static", models: s.models, modelHosts, modelBytes: s.modelBytes, modelFiles: pinned, ort: s.ort },
    requires: ["webgpu", "opfs"], capabilities: { storage: ["holo-space:" + s.id] },
  };
  const configBytes = Buffer.from(canon(config) + "\n");
  writeFileSync(join(to, "holospace.json"), configBytes);

  // the artifact tree as it will exist everywhere (site dist, registry pull, IPFS, the OS): Space files + runtime/
  const tree = mkdtempSync(join(tmpdir(), "space-")); const art = join(tree, s.id); mkdirSync(art);
  for (const f of walk(to)) { if (f === "holospace.lock.json" || f === "manifest.json") continue; mkdirSync(dirname(join(art, f)), { recursive: true }); copyFileSync(join(to, f), join(art, f)); }
  for (const f of runtimeFiles) { mkdirSync(dirname(join(art, f)), { recursive: true }); copyFileSync(join(OUT, f), join(art, f)); }
  const ipfs = ipfsAddresses(art);
  rmSync(tree, { recursive: true, force: true });

  // every file by digest; the layers in one fixed order (path-sorted)
  const files = {};
  for (const f of [...walk(to).filter((f) => f !== "holospace.lock.json" && f !== "manifest.json"), ...runtimeFiles].sort()) {
    const b = readFileSync(f.startsWith("runtime/") ? join(OUT, f) : join(to, f));
    const hex = sha256(b); const cid = ipfs.files[f];
    if (!cid) throw new Error(s.id + ": no CID for " + f);
    if (b.length <= (1 << 20) && cid !== rawCid(hex)) throw new Error(`${s.id}: ${f} raw CID ${cid} != sha-256 ${hex}`);   // the recipe's promise, checked
    files[f] = { sha256: hex, bytes: b.length, cid, mediaType: mediaType(f) };
  }
  const layers = Object.entries(files).filter(([p]) => p !== "holospace.json").map(([p, f]) => ({ mediaType: f.mediaType, digest: "sha256:" + f.sha256, size: f.bytes, annotations: { "org.opencontainers.image.title": p } }));
  const manifest = {
    schemaVersion: 2, mediaType: "application/vnd.oci.image.manifest.v1+json", artifactType: "application/vnd.hologram.space.v1+json",
    config: { mediaType: "application/vnd.hologram.space.config.v1+json", digest: "sha256:" + sha256(configBytes), size: configBytes.length },
    layers,
    annotations: {
      "org.opencontainers.image.title": s.name, "org.opencontainers.image.description": s.tagline, "org.opencontainers.image.source": s.source,
      "foundation.uor.space.entry": "index.html", "foundation.uor.space.models": s.models.join(","), "foundation.uor.space.ipfs": ipfs.root,
    },
  };
  const manifestBytes = Buffer.from(canon(manifest));
  const kappa = "sha256:" + sha256(manifestBytes);
  writeFileSync(join(to, "manifest.json"), manifestBytes);
  const bytes = Object.values(files).reduce((a, f) => a + f.bytes, 0);
  writeFileSync(join(to, "holospace.lock.json"), JSON.stringify({ format: "hologram.space.lock/v1", id: s.id, root: kappa, manifest: "manifest.json", ipfs: { recipe: "unixfs-v1-2025", root: ipfs.root }, files }, null, 2) + "\n");
  catalog.push({ id: s.id, name: s.name, task: s.task, tagline: s.tagline, root: kappa, ipfs: ipfs.root, bytes, files: Object.keys(files).length,
    models: s.models, modelHosts, modelBytes: s.modelBytes, ort: s.ort, source: s.source, requires: config.requires, entry: `${s.id}/index.html` });
  console.log(`sealed ${s.id}  κ ${kappa.slice(7, 19)}…  ipfs ${ipfs.root.slice(0, 16)}…  ${Object.keys(files).length} files  ${(bytes / 1e6).toFixed(1)} MB  models via ${Object.values(modelHosts).map((h) => new URL(h).host).join(",")}`);
}
writeFileSync(join(OUT, "spaces.json"), JSON.stringify({ format: "hologram.spaces.catalog/v1", spaces: catalog }, null, 2) + "\n");
console.log("catalog", join(OUT, "spaces.json"));
