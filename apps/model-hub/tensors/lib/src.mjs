// Reads from Hugging Face: model info, file tree, signed CDN URLs, Range reads and resumable streams.
// Paced for a shared IP and obedient to 429 + RateLimit headers. Nothing here stores bytes.
const HF = process.env.HF_ORIGIN || "https://huggingface.co";
const UA = { "user-agent": "hologram-tensor-index/0.1" };
const auth = process.env.HF_TOKEN ? { authorization: `Bearer ${process.env.HF_TOKEN}` } : {};
const PACE = { api: 750, resolve: 170 };
const next = { api: 0, resolve: 0 };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function paced(bucket) {
  const now = Date.now(), t = Math.max(now, next[bucket]);
  next[bucket] = t + PACE[bucket];
  if (t > now) await sleep(t - now);
}

async function hf(url, { bucket = "api", redirect = "follow", tries = 6 } = {}) {
  for (let k = 0; k < tries; k++) {
    await paced(bucket);
    let r;
    try { r = await fetch(url, { redirect, headers: { ...UA, ...auth }, signal: AbortSignal.timeout(60_000) }); }
    catch { await sleep(2 ** k * 1000); continue; }
    if (r.status === 429) {
      const m = /t=(\d+)/.exec(r.headers.get("ratelimit") || "");
      await sleep(((m ? +m[1] : 30 * (k + 1)) + 2) * 1000); continue;
    }
    if (r.status >= 500) { await sleep(2 ** k * 1000); continue; }
    const m = /r=(\d+);t=(\d+)/.exec(r.headers.get("ratelimit") || "");
    if (m && +m[1] < 40) await sleep((+m[2] + 1) * 1000);
    return r;
  }
  throw new Error(`gave up on ${url}`);
}

export async function info(repo) {
  const r = await hf(`${HF}/api/models/${repo}`);
  if (!r.ok) throw new Error(`info ${repo}: ${r.status}`);
  return r.json();
}

// Every file at a revision: { path, size, oid } where oid is the LFS sha256 (null for small git files).
export async function tree(repo, rev) {
  const out = [];
  let url = `${HF}/api/models/${repo}/tree/${rev}?recursive=true&expand=false`;
  while (url) {
    const r = await hf(url);
    if (!r.ok) throw new Error(`tree ${repo}@${rev}: ${r.status}`);
    for (const x of await r.json()) if (x.type === "file") out.push({ path: x.path, size: x.size, oid: x.lfs?.oid || null });
    const link = /<([^>]+)>;\s*rel="next"/.exec(r.headers.get("link") || "");
    url = link ? new URL(link[1], HF).href : null;
  }
  return out;
}

export const resolveUrl = (repo, rev, path) => `${HF}/${repo}/resolve/${rev}/${path.split("/").map(encodeURIComponent).join("/")}`;

// One resolver hit -> the signed CDN URL, reusable for many Range reads for ~40 minutes.
const cdn = new Map();
export async function cdnUrl(repo, rev, path, fresh = false) {
  const key = `${repo}@${rev}/${path}`, c = cdn.get(key);
  if (c && !fresh && Date.now() - c.at < 40 * 60_000) return c.url;
  let url = resolveUrl(repo, rev, path);
  for (let hop = 0; hop < 4; hop++) {
    const r = await hf(url, { bucket: "resolve", redirect: "manual" });
    const loc = r.headers.get("location");
    if (r.status >= 300 && r.status < 400 && loc) {
      r.body?.cancel();
      const abs = new URL(loc, url).href;
      if (new URL(abs).origin === new URL(HF).origin) { url = abs; continue; } // renamed repo: follow once more
      cdn.set(key, { url: abs, at: Date.now() }); return abs;
    }
    if (r.ok) { r.body?.cancel(); cdn.set(key, { url, at: Date.now() }); return url; } // small files are served inline
    throw new Error(`resolve ${key}: ${r.status}`);
  }
  throw new Error(`resolve ${key}: too many redirects`);
}

// A small file in full (configs, tokenizers).
export async function whole(repo, rev, path) {
  const r = await hf(resolveUrl(repo, rev, path), { bucket: "resolve" });
  if (!r.ok) throw new Error(`fetch ${repo}/${path}: ${r.status}`);
  return Buffer.from(await r.arrayBuffer());
}

export async function range(url, a, b, tries = 6) {
  for (let k = 0; k < tries; k++) {
    try {
      const r = await fetch(url, { headers: { ...UA, range: `bytes=${a}-${b}` }, signal: AbortSignal.timeout(120_000) });
      if (r.status === 206) return Buffer.from(await r.arrayBuffer());
      if (r.status === 200 && a === 0) { const buf = Buffer.from(await r.arrayBuffer()); return buf.subarray(0, b + 1); }
      r.body?.cancel();
      if (r.status === 403 || r.status === 410) throw Object.assign(new Error("signed url expired"), { expired: true });
    } catch (e) { if (e.expired) throw e; }
    await sleep(2 ** k * 1000);
  }
  throw new Error(`range failed ${a}-${b}`);
}

// Stream bytes [a, b] as chunks, resuming after drops. `getUrl(fresh)` supplies (and renews) the URL.
export async function* stream(getUrl, a, b, tries = 8) {
  let pos = a, url = await getUrl(false);
  for (let k = 0; k < tries && pos <= b; k++) {
    try {
      const r = await fetch(url, { headers: { ...UA, range: `bytes=${pos}-${b}` }, signal: AbortSignal.timeout(600_000) });
      if (r.status === 403 || r.status === 410) { r.body?.cancel(); url = await getUrl(true); continue; }
      if (r.status !== 206 && !(r.status === 200 && pos === 0)) { r.body?.cancel(); throw new Error(`status ${r.status}`); }
      for await (const chunk of r.body) {
        const c = Buffer.from(chunk);
        const take = Math.min(c.length, b - pos + 1);
        if (take <= 0) break;
        yield take === c.length ? c : c.subarray(0, take);
        pos += take;
        if (pos > b) break;
      }
      if (pos > b) return;
    } catch { await sleep(2 ** k * 1000); }
  }
  if (pos <= b) throw new Error(`stream failed at ${pos} of ${b}`);
}
