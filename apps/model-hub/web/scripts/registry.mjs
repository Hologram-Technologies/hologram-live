// The Registry index, end to end, from one command. Metadata only: no image bytes are ever downloaded.
//
//   node scripts/registry.mjs                 enrich + covers: every row of the index gets its profile and its logo
//   node scripts/registry.mjs --crawl         a fresh row set first (Docker Hub, Artifact Hub, Microsoft), then the same
//   node scripts/registry.mjs --force         refetch profiles that are still fresh (default max age: 1 day)
//   node scripts/registry.mjs --only docker|artifacthub|mcr --limit 40 --concurrency 4
//
// What it writes, all under public/registry/, all committed, so the site builds from a checkout with no network:
//   data/images.json            the page index: one row per artifact with the facets the rail counts
//   data/artifacts/<h>.json     one profile per row (hologram.registry.artifact/v1): README, tags, publisher, links,
//                               stats, security summary, install notes; <h> = sha256(id) first 16 hex
//   icons/<h>.<ext>             every logo a source publishes, fetched once and served from this origin
//
// Every field records the API it came from. Nothing is written by hand: a row with no logo upstream gets the mark
// drawn from its name (a data URL, no network), a row with no README says so, and a source that is down leaves
// yesterday's profile in place rather than a hole. Resumable: a profile fresher than --max-age is skipped, and each
// profile is written as it lands, so a run cut short keeps what it fetched.
//
// The row set is sealed separately (registry-kappa: every row's κ is committed to one index digest), so --crawl is
// explicit: a plain run keeps the rows and refreshes what the rows say.

import { readFile, writeFile, mkdir, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = join(dirname(fileURLToPath(import.meta.url)), "..");
const REG = join(SITE, "public", "registry");
const DATA = join(REG, "data", "images.json");
const ARTIFACTS = join(REG, "data", "artifacts");
const ICONS = join(REG, "icons");
const OURS = process.env.HOLOGRAM_REGISTRY || ORIGIN;   // the hub's one name, from src/origin.mjs
const UA = "hologram-registry-index/1.0 (+https://gethologram.ai/registry)";

const args = process.argv.slice(2);
const flag = (f) => args.includes(f);
const opt = (f, d) => { const i = args.indexOf(f); return i > -1 && args[i + 1] != null ? args[i + 1] : d; };
const CRAWL = flag("--crawl");
const FORCE = flag("--force");
const ONLY = opt("--only", null);
const LIMIT = Number(opt("--limit", 0)) || 0;
const CONCURRENCY = Number(opt("--concurrency", 4)) || 4;
const MAX_AGE_MS = Number(opt("--max-age", 1)) * 86400000;
const COVERS = !flag("--no-covers");
const README_CAP = 160_000;
const TAGS_CAP = 50;

import { artifactFile, mark } from "../src/registry-page.mjs";
import { ORIGIN } from "../src/origin.mjs";

// ---------------------------------------------------------------- http, with the manners each source expects

const calls = {};
let limited = 0;
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

async function http(url, { accept = "application/json", tries = 4 } = {}) {
  const host = new URL(url).host;
  calls[host] = (calls[host] || 0) + 1;
  for (let t = 0; ; t++) {
    let res;
    try { res = await fetch(url, { headers: { accept, "user-agent": UA }, redirect: "follow" }); }
    catch (e) { if (t < tries) { await sleep(1500 * 2 ** t); continue; } return { status: 0, error: String(e) }; }
    if ((res.status === 429 || res.status >= 500) && t < tries) {
      limited += res.status === 429 ? 1 : 0;
      const wait = Number(res.headers.get("retry-after")) * 1000 || 4000 * 2 ** t;
      await sleep(wait);
      continue;
    }
    return res;
  }
}

async function json(url) {
  const res = await http(url);
  if (!res.ok) return { error: res.status || res.error || "no answer" };
  try { return { body: await res.json() }; } catch { return { error: "not json" }; }
}

// ---------------------------------------------------------------- shared shape helpers (same rules as the page)


const freshness = (iso) => {
  if (!iso) return null;
  const days = (Date.now() - Date.parse(iso)) / 86400000;
  if (!isFinite(days)) return null;
  return days < 1 ? "Today" : days < 7 ? "This week" : days < 31 ? "This month" : days < 366 ? "This year" : "Over a year ago";
};
const repoBucket = (n) => n == null ? null : n < 104857600 ? "Under 100 MB" : n < 1073741824 ? "100 MB to 1 GB" : n < 10737418240 ? "1 to 10 GB" : "Over 10 GB";
const sizeBucket = (n) => n == null ? null : n < 10 * 1048576 ? "Under 10 MB" : n < 100 * 1048576 ? "10 to 100 MB" : n < 1073741824 ? "100 MB to 1 GB" : "Over 1 GB";

const homeOf = (r) =>
  r.registry === "Docker Hub" ? (r.org === "library" ? `https://hub.docker.com/_/${r.name}` : `https://hub.docker.com/r/${r.org}/${r.name}`)
  : r.registry === "Microsoft Artifact Registry" ? `https://mcr.microsoft.com/en-us/product/${r.org}/${r.name}/about`
  : r.registry === "Artifact Hub" ? `https://artifacthub.io/packages/${AH_PATH[r.kind] || "helm"}/${r.id.split("/")[1]}/${r.name}`
  : null;

// Artifact Hub's kinds by their own numbering and URL path. The numbering is theirs (search?kind=N); the path is
// what a package page is addressed by (/packages/<path>/<repo>/<name>) and what the package API takes.
const AH_KINDS = {
  0: ["Helm chart", "helm"], 1: ["Falco rule", "falco"], 2: ["OPA policy", "opa"], 3: ["OLM operator", "olm"],
  4: ["Tinkerbell action", "tbaction"], 5: ["Kubectl plugin", "krew"], 6: ["Helm plugin", "helm-plugin"],
  7: ["Tekton task", "tekton-task"], 8: ["KEDA scaler", "keda-scaler"], 9: ["CoreDNS plugin", "coredns"],
  10: ["Keptn integration", "keptn"], 11: ["Tekton pipeline", "tekton-pipeline"], 12: ["Container image", "container"],
  13: ["Kubewarden policy", "kubewarden"], 14: ["Gatekeeper policy", "gatekeeper"], 15: ["Kyverno policy", "kyverno"],
  16: ["Knative client plugin", "knative-client-plugin"], 17: ["Backstage plugin", "backstage"],
  18: ["Argo template", "argo-template"], 19: ["KubeArmor policy", "kubearmor"], 20: ["KCL module", "kcl"],
  21: ["Headlamp plugin", "headlamp"], 22: ["Inspektor gadget", "inspektor-gadget"], 23: ["Tekton stepaction", "tekton-stepaction"],
  24: ["Meshery design", "meshery"], 25: ["OpenCost plugin", "opencost"], 26: ["Radius recipe", "radius"],
  27: ["Bootable container", "bootc"], 28: ["Kagent agent", "kagent"],
};
const AH_PATH = Object.fromEntries(Object.values(AH_KINDS));
const AH_CATEGORY = { 1: "AI and machine learning", 2: "Database", 3: "Integration and delivery", 4: "Monitoring and logging",
  5: "Networking", 6: "Security", 7: "Storage", 8: "Streaming and messaging" };

const sourceOf = (r) =>
  r.registry === "Docker Hub" ? "docker" : r.registry === "Artifact Hub" ? "artifacthub"
  : r.registry === "Microsoft Artifact Registry" ? "mcr" : r.registry === "Hologram" ? "hologram" : null;

// ---------------------------------------------------------------- crawl: a fresh row set

const KINDS = [
  [/helm\.config/, "Helm chart"], [/cyclonedx|spdx|sbom/, "SBOM"], [/cosign|signature|notary/, "Signature"],
  [/in-toto|attestation/, "Attestation"], [/wasm/, "Wasm module"], [/ai\.model|model/, "Model"], [/mcp|skill/, "Skill"],
  [/sandbox\.kit/, "Sandbox kit"], [/tekton/, "Tekton package"], [/opa|rego/, "Policy"], [/hologram/, "Artifact"],
];
const kindOf = (declared, mediaTypes = [], contentTypes = []) => {
  if (declared && declared !== "image") return declared;
  const t = `${mediaTypes.join(" ")} ${contentTypes.join(" ")}`.toLowerCase();
  for (const [re, name] of KINDS) if (re.test(t)) return name;
  return "Image";
};

async function dockerNamespace(ns, kind, official) {
  const out = [];
  const page = await json(`https://hub.docker.com/v2/namespaces/${ns}/repositories?page_size=100`);
  for (const r of page.body?.results || []) {
    out.push({ id: `docker.io/${ns}/${r.name}`, registry: "Docker Hub", org: ns, name: r.name,
      kind: kindOf(kind, r.media_types, r.content_types), description: r.description || "",
      pulls: r.pull_count ?? null, stars: r.star_count ?? null, updated: r.last_updated || null, storage: r.storage_size ?? null,
      official, verified: false, signed: null, license: null, category: null, architectures: [], size: null, provenance: "Docker Hub API" });
  }
  return out;
}
const SEEDS = ["nginx", "postgres", "redis", "node", "python", "ubuntu", "alpine", "mysql", "mongo", "java", "golang", "rust", "php",
  "ruby", "grafana", "prometheus", "elasticsearch", "kafka", "rabbitmq", "traefik", "caddy", "wordpress", "jenkins", "gitlab", "nextcloud"];
const PUBLISHERS = ["bitnami", "grafana", "hashicorp", "confluentinc", "elastic", "jenkins", "nginxinc", "datadog", "minio", "portainer",
  "rancher", "sonarqube", "timescale", "victoriametrics", "envoyproxy", "fluent", "jaegertracing", "kong", "linuxserver", "curlimages", "otel", "istio"];

async function crawlDocker() {
  const rows = [...await dockerNamespace("library", "image", true)];
  for (const [ns, kind] of [["ai", "Model"], ["mcp", "Skill"], ["sbx", "Sandbox kit"]]) rows.push(...await dockerNamespace(ns, kind, false));
  for (const ns of PUBLISHERS) rows.push(...await dockerNamespace(ns, "image", false));
  for (const q of SEEDS) for (const page of [1, 2]) {
    const res = await json(`https://hub.docker.com/v2/search/repositories/?query=${q}&page_size=100&page=${page}`);
    if (!res.body?.results?.length) break;
    for (const r of res.body.results) {
      if (r.is_official) continue;
      const [org, ...rest] = r.repo_name.split("/");
      rows.push({ id: `docker.io/${r.repo_name}`, registry: "Docker Hub", org, name: rest.join("/") || r.repo_name, kind: "Image",
        description: r.short_description || "", pulls: r.pull_count ?? null, stars: r.star_count ?? null, updated: null, storage: null,
        official: false, verified: false, signed: null, license: null, category: null, architectures: [], size: null, provenance: "Docker Hub API" });
    }
  }
  return rows;
}

async function crawlArtifactHub(perKind = 60) {
  const rows = [];
  for (const [num, [label]] of Object.entries(AH_KINDS)) {
    const page = await json(`https://artifacthub.io/api/v1/packages/search?kind=${num}&limit=${perKind}&offset=0&sort=stars`);
    for (const p of page.body?.packages || []) {
      const repo = p.repository || {};
      rows.push({ id: `artifacthub/${repo.name}/${p.normalized_name}`, registry: "Artifact Hub", org: repo.organization_name || repo.name || "",
        name: p.normalized_name || p.name, kind: label, description: p.description || "", pulls: null, stars: p.stars ?? null,
        updated: p.ts ? new Date(p.ts * 1000).toISOString() : null, storage: null, official: !!repo.official, verified: !!repo.verified_publisher,
        signed: p.signed ?? null, license: null, category: AH_CATEGORY[p.category] || null, architectures: [], size: null,
        logoUrl: p.logo_image_id ? `https://artifacthub.io/image/${p.logo_image_id}` : null, provenance: "Artifact Hub API" });
    }
  }
  return rows;
}

async function crawlMicrosoft() {
  const products = await json("https://mcr.microsoft.com/api/v1/catalog/products");
  return (Array.isArray(products.body) ? products.body : []).map((p) => ({
    id: `mcr.microsoft.com/${p.repository}`, registry: "Microsoft Artifact Registry", org: (p.repository || "").split("/")[0],
    name: (p.repository || "").split("/").slice(1).join("/") || p.repository, kind: "Image", description: p.shortDescription || "",
    pulls: null, pullsText: p.totalPullCount || null, stars: null, updated: p.lastModifiedDate || null, storage: null, official: true,
    verified: false, signed: null, license: p.licenseType || null, category: (p.categories && p.categories[0]) || null,
    architectures: p.architectures || [], size: p.sizeInBytes || null,
    logoUrl: p.imagePath ? (p.imagePath.startsWith("http") ? p.imagePath : `https://mcr.microsoft.com${p.imagePath}`) : null,
    provenance: "Microsoft catalogue API" }));
}

async function crawl(previous) {
  const byId = new Map((previous?.images || []).map((r) => [r.id, r]));
  const [docker, ah, ms] = await Promise.all([crawlDocker(), crawlArtifactHub(), crawlMicrosoft()]);
  const byPulls = (a, b) => (b.pulls || 0) - (a.pulls || 0) || (b.stars || 0) - (a.stars || 0);
  const seen = new Set();
  const rows = [...docker.filter((r) => r.kind !== "Image"), ...docker.filter((r) => r.kind === "Image").sort(byPulls).slice(0, 500),
    ...ah, ...ms.sort(byPulls).slice(0, 600)].filter((r) => !seen.has(r.id) && seen.add(r.id));
  // Everything the sealed κ index said about a row carries over; the κ is bound to the digest it named, and
  // re-resolving is registry-kappa's job, not this file's.
  for (const r of rows) {
    const old = byId.get(r.id);
    if (old) for (const k of ["kappa", "trust", "held", "kappaStatus", "digest", "tag", "cover", "logoUrl"]) if (old[k] != null && r[k] == null) r[k] = old[k];
  }
  return rows;
}

// ---------------------------------------------------------------- enrich: one profile per row

const plat = (os, arch, variant) => (os || arch ? `${os || "unknown"}/${arch || "unknown"}${variant ? "/" + variant : ""}` : null);
const uniq = (a) => [...new Set(a.filter(Boolean))];

async function profileDocker(r) {
  const repo = `${r.org}/${r.name}`;
  const [meta, tags] = await Promise.all([
    json(`https://hub.docker.com/v2/repositories/${repo}/`),
    json(`https://hub.docker.com/v2/repositories/${repo}/tags?page_size=${TAGS_CAP}&ordering=last_updated`),
  ]);
  if (meta.error && tags.error) return { error: `Docker Hub ${meta.error}` };
  const m = meta.body || {};
  const shape = (t) => ({
    name: t.name, digest: t.digest || null, pushed: t.tag_last_pushed || t.last_updated || null, size: t.full_size ?? null,
    platforms: uniq((t.images || []).map((i) => plat(i.os, i.architecture, i.variant))), mediaType: t.media_type || null,
    status: t.tag_status || null,
  });
  const list = (tags.body?.results || []).map(shape);
  // The row's own tag (its κ names it) leads the list even when it is not among the newest pushes: a repository
  // with a thousand tags pushes `latest` less often than its nightlies.
  const want = r.tag && !/^sha256-/.test(r.tag) ? r.tag : "latest";
  if (!list.some((t) => t.name === want)) {
    const one = await json(`https://hub.docker.com/v2/repositories/${repo}/tags/${encodeURIComponent(want)}`);
    if (one.body?.name) list.unshift(shape(one.body));
  }
  const logo = { url: `https://hub.docker.com/api/media/repos_logo/v1/${encodeURIComponent(repo)}`, via: "Docker Hub media" };
  const publisher = r.org === "library"
    ? { name: "Docker Official Images", url: "https://hub.docker.com/u/library", official: true, verified: true }
    : { name: m.hub_user || r.org, url: `https://hub.docker.com/u/${r.org}`, official: false, verified: m.affiliation === "verified publisher" || false };
  return {
    body: {
      source: { name: "Docker Hub", api: `https://hub.docker.com/v2/repositories/${repo}/`, page: homeOf(r) },
      readme: cap(m.full_description), readmeBase: null,
      publisher, registered: m.date_registered || null, updated: m.last_updated || null,
      categories: (m.categories || []).map((c) => c.name).filter(Boolean), keywords: [], license: null,
      links: [], maintainers: [],
      stats: { pulls: m.pull_count ?? r.pulls ?? null, stars: m.star_count ?? r.stars ?? null },
      mediaTypes: m.media_types || [], contentTypes: m.content_types || [], storage: m.storage_size ?? r.storage ?? null,
      tags: list, tagsTotal: tags.body?.count ?? list.length, security: null, install: null, version: null, appVersion: null,
      containersImages: [], logo,
    },
  };
}

async function profileArtifactHub(r) {
  const path = AH_PATH[r.kind] || "helm";
  const repoName = r.id.split("/")[1];
  const res = await json(`https://artifacthub.io/api/v1/packages/${path}/${repoName}/${r.name}`);
  if (res.error) return { error: `Artifact Hub ${res.error}` };
  const p = res.body;
  const repo = p.repository || {};
  const versions = (p.available_versions || []).slice(0, TAGS_CAP).map((v) => ({
    name: v.version, digest: v.version === p.version && p.digest ? (p.digest.startsWith("sha256:") ? p.digest : `sha256:${p.digest}`) : null,
    pushed: v.ts ? new Date(v.ts * 1000).toISOString() : null, size: null, platforms: [], mediaType: null,
    status: v.prerelease ? "prerelease" : v.contains_security_updates ? "security update" : null, appVersion: v.app_version || null,
  }));
  const links = [...(p.links || []).map((l) => ({ name: l.name || "link", url: l.url }))];
  if (p.home_url && !links.some((l) => l.url === p.home_url)) links.unshift({ name: "home", url: p.home_url });
  if (repo.url) links.push({ name: `${repo.display_name || repo.name} repository`, url: repo.url });
  return {
    body: {
      source: { name: "Artifact Hub", api: `https://artifacthub.io/api/v1/packages/${path}/${repoName}/${r.name}`, page: homeOf(r) },
      readme: cap(p.readme), readmeBase: p.home_url || repo.url || null,
      publisher: { name: repo.organization_display_name || repo.organization_name || repo.user_alias || repo.name || r.org,
        url: repo.organization_name ? `https://artifacthub.io/packages/search?org=${repo.organization_name}` : repo.user_alias ? `https://artifacthub.io/packages/search?user=${repo.user_alias}` : null,
        official: !!repo.official, verified: !!repo.verified_publisher, cncf: !!repo.cncf },
      registered: null, updated: p.ts ? new Date(p.ts * 1000).toISOString() : null,
      categories: [AH_CATEGORY[p.category]].filter(Boolean), keywords: p.keywords || [], license: p.license || null,
      links, maintainers: (p.maintainers || []).map((m) => ({ name: m.name, email: m.email || null })),
      stats: { pulls: null, stars: p.stars ?? r.stars ?? null, subscriptions: p.stats?.subscriptions ?? null },
      mediaTypes: [], contentTypes: [], storage: null,
      tags: versions, tagsTotal: (p.available_versions || []).length,
      security: p.security_report_summary || null, install: cap(p.install, 20_000), version: p.version || null, appVersion: p.app_version || null,
      containersImages: (p.containers_images || []).map((c) => c.image).filter(Boolean),
      contentUrl: p.content_url || null, signed: p.signed ?? null, deprecated: !!p.deprecated,
      repo: { name: repo.name || repoName, url: repo.url || null, kind: AH_KINDS[repo.kind]?.[0] || null },
      logo: p.logo_image_id ? { url: `https://artifacthub.io/image/${p.logo_image_id}`, via: "Artifact Hub image" } : null,
    },
  };
}

async function profileMicrosoft(r) {
  const repo = r.id.replace(/^mcr\.microsoft\.com\//, "");   // a one-segment repository has org === name
  const [meta, tags] = await Promise.all([
    json(`https://mcr.microsoft.com/api/v1/catalog/${repo}/details?reg=mar`),
    json(`https://mcr.microsoft.com/api/v1/catalog/${repo}/tags?reg=mar`),
  ]);
  if (meta.error && tags.error) return { error: `Microsoft ${meta.error}` };
  const m = meta.body || {};
  const all = (Array.isArray(tags.body) ? tags.body : []).sort((a, b) => Date.parse(b.lastModifiedDate || 0) - Date.parse(a.lastModifiedDate || 0));
  const list = all.slice(0, TAGS_CAP).map((t) => ({
    name: t.name, digest: t.digest || null, pushed: t.lastModifiedDate || null, size: t.size ?? null,
    platforms: uniq([plat(t.operatingSystem, t.architecture)]), mediaType: t.manifestType || null, status: null,
  }));
  return {
    body: {
      source: { name: "Microsoft Artifact Registry", api: `https://mcr.microsoft.com/api/v1/catalog/${repo}/details?reg=mar`, page: homeOf(r) },
      readme: cap(m.readme), readmeBase: null,
      publisher: { name: m.publisher || "Microsoft", url: "https://mcr.microsoft.com/", official: true, verified: true },
      registered: m.createdDate || null, updated: m.lastModifiedDate || r.updated || null,
      categories: m.categories || (r.category ? [r.category] : []), keywords: m.keywords || [], license: m.licenseType || r.license || null,
      links: [m.projectUrl && { name: "project", url: m.projectUrl }, m.documentationUrl && { name: "documentation", url: m.documentationUrl },
        m.supportUrl && { name: "support", url: m.supportUrl }, m.licenseUrl && { name: "licence", url: m.licenseUrl }].filter(Boolean),
      maintainers: [],
      stats: { pulls: null, pullsText: m.totalPullCount || r.pullsText || null, stars: null },
      mediaTypes: [], contentTypes: [], storage: null,
      tags: list, tagsTotal: all.length, security: null, install: null, version: null, appVersion: null, containersImages: [],
      logo: m.imagePath || r.logoUrl ? { url: (m.imagePath || r.logoUrl).startsWith("http") ? (m.imagePath || r.logoUrl) : `https://mcr.microsoft.com${m.imagePath || r.logoUrl}`, via: "Microsoft catalogue" } : null,
    },
  };
}

function cap(text, n = README_CAP) {
  if (!text || typeof text !== "string") return null;
  const t = text.replace(/\r\n/g, "\n").trim();
  return t ? (t.length > n ? t.slice(0, n) : t) : null;
}

const PROFILERS = { docker: profileDocker, artifacthub: profileArtifactHub, mcr: profileMicrosoft };

async function enrich(rows) {
  await mkdir(ARTIFACTS, { recursive: true });
  const todo = [];
  let fresh = 0;
  for (const r of rows) {
    const src = sourceOf(r);
    if (!PROFILERS[src] || (ONLY && src !== ONLY)) continue;
    const file = join(ARTIFACTS, artifactFile(r.id));
    if (!FORCE && existsSync(file)) {
      const age = Date.now() - (await stat(file)).mtimeMs;
      // A stub written for a failed fetch is never fresh: it is retried on every run until the source answers.
      const stub = JSON.parse(await readFile(file, "utf8")).error;
      if (age < MAX_AGE_MS && !stub) { fresh++; continue; }
    }
    todo.push(r);
  }
  const queue = LIMIT ? todo.slice(0, LIMIT) : todo.slice();
  const planned = queue.length;
  let done = 0, failed = 0;
  const started = Date.now();
  const worker = async () => {
    for (;;) {
      const r = queue.shift();
      if (!r) return;
      const res = await PROFILERS[sourceOf(r)](r).catch((e) => ({ error: String(e) }));
      const file = join(ARTIFACTS, artifactFile(r.id));
      if (res.error) {
        failed++;
        // Yesterday's profile beats a hole. A row that never had one gets an honest stub, so the page still builds.
        if (!existsSync(file)) await writeFile(file, JSON.stringify({ format: "hologram.registry.artifact/v1", id: r.id, fetched: new Date().toISOString(), error: res.error, source: { name: r.registry, api: null, page: homeOf(r) } }));
        else { const old = JSON.parse(await readFile(file, "utf8")); old.lastError = { at: new Date().toISOString(), error: res.error }; await writeFile(file, JSON.stringify(old)); }
      } else {
        await writeFile(file, JSON.stringify({ format: "hologram.registry.artifact/v1", id: r.id, fetched: new Date().toISOString(), ...res.body }));
      }
      done++;
      if (done % 50 === 0) console.log(`  profiles ${done}/${queue.length + done} (${failed} failed, ${((Date.now() - started) / 1000).toFixed(0)}s)`);
    }
  };
  await Promise.all(Array.from({ length: CONCURRENCY }, worker));
  return { fetched: done, failed, fresh, skipped: todo.length - planned };
}

// ---------------------------------------------------------------- covers: every logo, once, from this origin

const ALIAS = { postgres: "postgresql", mongo: "mongodb", node: "nodedotjs", golang: "go", "fluent-bit": "fluentbit", "docker-compose": "docker" };
const slugs = (org, name) => {
  const clean = (x) => String(x || "").toLowerCase().replace(/[^a-z0-9.-]/g, "");
  const out = [];
  const n = clean(name), o = clean(org);
  for (const v of [ALIAS[n], n, n.replace(/-/g, ""), ALIAS[o], o, o.replace(/-/g, "")]) if (v && v.length > 1 && !out.includes(v)) out.push(v);
  return out.slice(0, 4);
};

const sniff = (buf, type) => {
  if (buf.length > 4 && buf[0] === 0x89 && buf[1] === 0x50) return "png";
  if (buf[0] === 0xff && buf[1] === 0xd8) return "jpg";
  if (buf.slice(0, 4).toString() === "RIFF" && buf.slice(8, 12).toString() === "WEBP") return "webp";
  const head = buf.slice(0, 512).toString("utf8").trim().toLowerCase();
  if (head.startsWith("<svg") || (head.startsWith("<?xml") && head.includes("<svg"))) return "svg";
  return type.includes("svg") ? "svg" : type.includes("png") ? "png" : type.includes("jpeg") ? "jpg" : type.includes("webp") ? "webp" : null;
};

const seenLogo = new Map();
async function vendor(url) {
  if (seenLogo.has(url)) return seenLogo.get(url);
  let out = null;
  const res = await http(url, { accept: "image/*,*/*", tries: 2 });
  if (res.ok) {
    const buf = Buffer.from(await res.arrayBuffer());
    const ext = sniff(buf, (res.headers.get("content-type") || "").split(";")[0]);
    if (ext && buf.length && buf.length < 262144) {
      const name = createHash("sha256").update(url).digest("hex").slice(0, 16) + "." + ext;
      await writeFile(join(ICONS, name), buf);
      out = "icons/" + name;
    }
  }
  seenLogo.set(url, out);
  return out;
}

async function covers(rows, profiles) {
  await mkdir(ICONS, { recursive: true });
  let vendored = 0, kept = 0, marks = 0;
  const queue = rows.filter((r) => !r.here);
  const worker = async () => {
    for (;;) {
      const r = queue.shift();
      if (!r) return;
      const p = profiles.get(r.id);
      // The source's own logo first; then the open Simple Icons set by brand slug; then the mark. A cover already
      // on disk from an earlier run is kept unless the source now names a different one.
      const tries = [];
      if (p?.logo?.url) tries.push(p.logo.url);
      if (r.logoUrl) tries.push(r.logoUrl);
      for (const s of slugs(r.org, r.name)) tries.push(`https://cdn.simpleicons.org/${s}/a6a4a2`);
      if (r.cover && existsSync(join(REG, r.cover)) && !FORCE) { kept++; r.logo = mark(r.id); continue; }
      let hit = null;
      for (const url of tries) { hit = await vendor(url); if (hit) { r.coverVia = url === p?.logo?.url ? p.logo.via : url === r.logoUrl ? r.registry : "Simple Icons"; break; } }
      if (hit) { r.cover = hit; vendored++; } else { delete r.cover; marks++; }
      r.logo = mark(r.id);
    }
  };
  await Promise.all(Array.from({ length: CONCURRENCY }, worker));
  for (const r of rows) { delete r.logoUrl; if (!r.logo) r.logo = mark(r.id); }
  return { vendored, kept, marks };
}

// ---------------------------------------------------------------- the index file

const count = (rows, pick) => {
  const out = {};
  for (const r of rows) for (const v of [].concat(pick(r) ?? [])) if (v != null && v !== "") out[v] = (out[v] || 0) + 1;
  return out;
};

function fold(rows, profiles) {
  for (const r of rows) {
    const p = profiles.get(r.id);
    r.home = r.home || homeOf(r);
    r.publisher = r.publisher || r.org || r.registry;
    if (p && !p.error) {
      if (p.stats?.pulls != null) r.pulls = p.stats.pulls;
      if (p.stats?.stars != null) r.stars = p.stats.stars;
      if (p.updated) r.updated = p.updated;
      if (p.storage != null) r.storage = p.storage;
      if (p.license && !r.license) r.license = p.license;
      if (p.categories?.[0] && !r.category) r.category = p.categories[0];
      if (p.publisher?.verified) r.verified = true;
      if (!r.tag && p.tags?.length) r.tag = (p.tags.find((t) => t.name === "latest") || p.tags[0]).name;
      const latest = p.tags?.find((t) => t.name === r.tag) || p.tags?.[0];
      if (latest?.platforms?.length && !(r.architectures || []).length) r.architectures = latest.platforms;
      if (latest?.size != null && r.size == null) r.size = latest.size;
      if (!r.description && p.readme) r.description = p.readme.replace(/^#.*$/m, "").replace(/[#*`>_\[\]]/g, "").trim().split(/\n+/)[0]?.slice(0, 200) || "";
      r.profile = true;
      r.tags = p.tagsTotal ?? p.tags?.length ?? null;
    } else if (p?.error) { r.profile = false; }
    r.updatedBucket = freshness(r.updated);
    r.repoBucket = repoBucket(r.storage);
    r.sizeBucket = sizeBucket(r.size);
  }
  return {
    facets: {
      registry: count(rows, (r) => r.registry), kind: count(rows, (r) => r.kind), category: count(rows, (r) => r.category),
      publisher: count(rows, (r) => r.publisher), license: count(rows, (r) => r.license), repoSize: count(rows, (r) => r.repoBucket),
      updated: count(rows, (r) => r.updatedBucket), architecture: count(rows, (r) => r.architectures), size: count(rows, (r) => r.sizeBucket),
      marks: count(rows, (r) => [(r.here || r.kappa) ? "Addressed here" : null, r.official ? "Official" : null, r.signed ? "Signed" : null,
        r.verified ? "Verified publisher" : null].filter(Boolean)),
    },
  };
}

// ---------------------------------------------------------------- run

const started = Date.now();
const previous = existsSync(DATA) ? JSON.parse(await readFile(DATA, "utf8")) : null;
let rows = previous?.images || [];
if (CRAWL) { rows = await crawl(previous); console.log(`crawled ${rows.length} rows`); }
if (!rows.length) throw new Error("no rows: run with --crawl to build a row set");

const enriched = await enrich(rows);
console.log("profiles", JSON.stringify(enriched));

const profiles = new Map();
for (const r of rows) {
  const file = join(ARTIFACTS, artifactFile(r.id));
  if (existsSync(file)) profiles.set(r.id, JSON.parse(await readFile(file, "utf8")));
}
const covered = COVERS ? await covers(rows, profiles) : { skipped: true };
console.log("covers", JSON.stringify(covered));

const { facets } = fold(rows, profiles);
const out = {
  built: new Date().toISOString(),
  totals: { images: rows.length, here: rows.filter((r) => r.here).length, elsewhere: rows.filter((r) => !r.here).length,
    profiles: rows.filter((r) => r.profile).length, covers: rows.filter((r) => r.cover).length },
  sources: { "Docker Hub": "https://hub.docker.com/v2/repositories/", "Artifact Hub": "https://artifacthub.io/api/v1/packages/",
    "Microsoft Artifact Registry": "https://mcr.microsoft.com/api/v1/catalog/", Hologram: OURS },
  facets,
  images: rows,
  covers: { vendored: rows.filter((r) => r.cover).length, missed: rows.filter((r) => !r.cover && !r.here).length },
  ...(previous?.kappaIndex ? { kappaIndex: previous.kappaIndex } : {}),
};
await writeFile(DATA, JSON.stringify(out));
console.log(JSON.stringify({ rows: rows.length, profiles: out.totals.profiles, covers: out.totals.covers, calls, rateLimited: limited,
  seconds: +((Date.now() - started) / 1000).toFixed(1) }, null, 1));
