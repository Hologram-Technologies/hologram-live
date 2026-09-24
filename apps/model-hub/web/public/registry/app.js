import { art } from "../card-art.mjs";
// The Images page. Same shape as the models page: facets on the left, search and sort on top, a card grid.
// It reads one static file. There is no query service, because the index is a file and filtering is a filter.

import { createRail, ICONS as I } from "../lib/rail.mjs";

const $ = (id) => document.getElementById(id);
const PAGE = 60;
let data = null;
let shown = PAGE;
const picked = { registry: new Set(), category: new Set(), marks: new Set(), kind: new Set(),
  architecture: new Set(), publisher: new Set(), license: new Set(), repoSize: new Set(), updated: new Set() };

const GROUPS = [
  ["registry", "Registry"],
  ["category", "Category"],
  ["here", "Addressed here"],
  ["official", "Official"],
  ["kind", "Kind"],
  ["architecture", "Architecture"],
  ["size", "Size"],
];

const num = (n) =>
  n == null ? null :
  n >= 1e9 ? (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B" :
  n >= 1e6 ? (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M" :
  n >= 1e3 ? (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K" : String(n);
const bytes = (n) =>
  n == null ? null : n < 1048576 ? (n / 1024).toFixed(0) + " KB" :
  n < 1073741824 ? (n / 1048576).toFixed(1) + " MB" : (n / 1073741824).toFixed(2) + " GB";

// The search box takes whatever is on your clipboard. A word filters. A reference, with a registry, a
// tag or a digest, is resolved to the row it names, because that is what you meant by pasting it.
function reference(text) {
  const t = text.trim();
  if (!t) return null;
  const digest = /^(sha256|sha512|blake3):[0-9a-f]{16,}$/i.exec(t);
  if (digest) return { kind: "digest", digest: t };
  if (/[/:@]/.test(t) && !/\s/.test(t)) {
    const at = t.split("@");
    const path = at[0];
    const parts = path.split("/");
    const hasHost = parts.length > 1 && /[.:]/.test(parts[0]);
    const last = parts[parts.length - 1].split(":");
    return {
      kind: "reference",
      host: hasHost ? parts[0] : null,
      repo: (hasHost ? parts.slice(1) : parts).slice(0, -1).concat(last[0]).join("/"),
      tag: last[1] || null,
      digest: at[1] || null,
    };
  }
  return null;
}

function resolve() {
  const ref = reference($("q").value);
  const box = $("resolved");
  if (!ref) { box.hidden = true; return null; }
  if (ref.kind === "digest") {
    const hit = data.images.find((r) => r.digest === ref.digest);
    box.hidden = false;
    box.textContent = hit ? `That digest is ${hit.id}, here in this registry.` : "That is a digest. No row in this index carries it.";
    return hit ? [hit] : [];
  }
  const rows = data.images.filter((r) => r.id.endsWith("/" + ref.repo) || r.id === ref.repo || r.id.includes(ref.repo));
  box.hidden = false;
  box.textContent = rows.length
    ? `Reading that as a reference to ${ref.repo}${ref.tag ? ":" + ref.tag : ""}${ref.host ? " on " + ref.host : ""}.`
    : `Nothing indexed matches ${ref.repo}. It may still exist: this index is a snapshot, not the world.`;
  return rows;
}

function matches(r) {
  const q = $("q").value.trim().toLowerCase();
  if (q && !(`${r.id} ${r.description}`.toLowerCase().includes(q))) return false;
  if (picked.registry.size && !picked.registry.has(r.registry)) return false;
  if (picked.kind.size && !picked.kind.has(r.kind)) return false;
  if (picked.category.size && !picked.category.has(r.category)) return false;
  if (picked.repoSize.size && !picked.repoSize.has(r.repoBucket)) return false;
  if (picked.publisher.size && !picked.publisher.has(r.publisher)) return false;
  if (picked.license.size && !picked.license.has(r.license)) return false;
  if (picked.updated.size && !picked.updated.has(r.updatedBucket)) return false;
  if (picked.marks.size) {
    const has = { "Addressed here": r.here, Official: r.official, Signed: r.signed, "Verified publisher": r.verified };
    for (const m of picked.marks) if (!has[m]) return false;
  }
  if (picked.architecture.size && !(r.architectures || []).some((a) => picked.architecture.has(a))) return false;
  return true;
}

function sorted(rows) {
  const by = $("sort").value;
  return rows.slice().sort((a, b) =>
    by === "name" ? a.id.localeCompare(b.id) :
    by === "stars" ? (b.stars || 0) - (a.stars || 0) :
    (b.pulls || 0) - (a.pulls || 0) || (b.stars || 0) - (a.stars || 0));
}


// A cover for every row, tried in order and allowed to fail: the logo its source published, then the
// open Simple Icons set by brand slug, then the mark drawn from the name. Lazy, so only what is on
// screen is ever fetched, and a miss costs one 404.

// ---------------------------------------------------------------- our own registry, read live

const ACCEPT = [
  "application/vnd.oci.image.manifest.v1+json",
  "application/vnd.oci.image.index.v1+json",
  "application/vnd.docker.distribution.manifest.v2+json",
  "application/vnd.docker.distribution.manifest.list.v2+json",
].join(", ");

// hash-wasm's blake3 build, vendored beside this page and pinned by checksum, because the layers here
// are addressed by blake3 and crypto.subtle cannot do it. Nothing is fetched from another origin: a
// verification page that loaded its own hasher from a third party would be verifying nothing.
//   vendor/blake3.umd.min.js  hash-wasm 4.12.0
//   sha256:21a2bf9f37dd86cf38fdd4bcb7ac8a4f8fa8956ea1b92a4f2eb440345ab20727
let hashwasm = null;
function loadBlake3() {
  if (window.hashwasm) return Promise.resolve(window.hashwasm);
  return new Promise((ok, bad) => {
    const el = document.createElement("script");
    el.src = "vendor/blake3.umd.min.js";
    el.onload = () => ok(window.hashwasm);
    el.onerror = () => bad(new Error("blake3 unavailable"));
    document.head.appendChild(el);
  });
}

async function digestOf(algo, stream) {
  if (algo === "blake3") {
    if (!hashwasm) hashwasm = await loadBlake3();
    const h = await hashwasm.createBLAKE3();
    h.init();
    const reader = stream.getReader();
    for (;;) { const { done, value } = await reader.read(); if (done) break; h.update(value); }
    return h.digest("hex");
  }
  const buf = await new Response(stream).arrayBuffer();
  const out = await crypto.subtle.digest(algo === "sha512" ? "SHA-512" : "SHA-256", buf);
  return [...new Uint8Array(out)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

const reg = {
  async base() { try { return (await fetch("/v2/")).ok; } catch { return false; } },
  async catalogue() {
    const r = await fetch("/v2/_catalog?n=100");
    return r.ok ? (await r.json()).repositories || [] : [];
  },
  async tags(repo) {
    const r = await fetch(`/v2/${repo}/tags/list?n=100`);
    return r.ok ? (await r.json()).tags || [] : [];
  },
  async manifest(repo, ref) {
    const r = await fetch(`/v2/${repo}/manifests/${ref}`, { headers: { Accept: ACCEPT } });
    if (!r.ok) throw new Error("manifest " + r.status);
    const text = await r.text();
    return { digest: r.headers.get("Docker-Content-Digest"), mediaType: r.headers.get("Content-Type") || "", body: JSON.parse(text) };
  },
  blob(repo, digest) { return fetch(`/v2/${repo}/blobs/${digest}`); },
  async allows(path) {
    try {
      const r = await fetch(path, { method: "OPTIONS" });
      return (r.headers.get("Allow") || "").split(",").map((m) => m.trim().toUpperCase()).filter(Boolean);
    } catch { return []; }
  },
  del(repo, ref, token) {
    const headers = token ? { Authorization: token.includes(":") ? "Basic " + btoa(token) : "Bearer " + token } : {};
    return fetch(`/v2/${repo}/manifests/${ref}`, { method: "DELETE", headers });
  },
};

// Deleting is the only thing on this page that changes the registry, so the page asks whether it is
// allowed before it offers the button. OPTIONS only: it never writes to find out.
let canDelete = false;
async function probe(sample) {
  if (!sample) return;
  canDelete = (await reg.allows(`/v2/${sample}/manifests/latest`)).includes("DELETE");
}

// Our repositories become rows like any other, read at load rather than baked into yesterday's file.
// An empty repository is still a repository, so it appears with an honest count.
async function liveRows() {
  const repos = await reg.catalogue().catch(() => []);
  const rows = await Promise.all(repos.map(async (repo) => {
    const tags = await reg.tags(repo).catch(() => []);
    return {
      id: `${location.host}/${repo}`,
      repo,
      registry: "Hologram",
      org: repo.split("/")[0],
      name: repo.split("/").slice(1).join("/") || repo,
      publisher: repo.split("/")[0],
      kind: "Artifact",
      description: tags.length ? `${tags.length} tag${tags.length > 1 ? "s" : ""} in this registry.` : "No tags yet.",
      pulls: null, stars: null, license: null, category: null,
      updated: null, updatedBucket: null, repoBucket: null, sizeBucket: null,
      official: false, verified: false, signed: false,
      architectures: [], size: null, digest: null,
      tags,
      here: true,
      live: true,
      logo: null,
      slugs: [],
      home: null,
      provenance: "read live from this registry",
    };
  }));
  return rows;
}

// ---------------------------------------------------------------- one of ours, opened

function ourCommands(repo, tag, digest) {
  const ref = `${location.host}/${repo}:${tag || "<tag>"}`;
  return [
    ["Pull it", `docker pull ${ref}`],
    ["By digest, the exact bytes", `docker pull ${location.host}/${repo}@${digest || "sha256:<digest>"}`],
    ["Push to it", `docker push ${ref}`],
    ["Remove a tag", `crane delete ${ref}`],
  ];
}

async function openOurs(r) {
  $("s-logo").src = r.logo || "";
  $("s-logo").style.visibility = "hidden";
  $("s-name").textContent = r.id;
  $("s-desc").textContent = "Read live from this registry. Every layer here can be checked in your browser.";
  $("s-tags").innerHTML = '<span class="tag here">here</span><span class="tag">live</span>';
  $("s-cmdlabel").textContent = "Pull it";
  $("s-cmd").textContent = `docker pull ${location.host}/${r.repo}:${r.tags[0] || "<tag>"}`;
  $("s-home").hidden = true;
  $("sheet").hidden = false;
  $("scrim").hidden = false;

  const facts = $("s-facts");
  facts.innerHTML = "";
  if (!r.tags.length) {
    facts.innerHTML = '<div style="grid-column:1/-1"><dd style="color:var(--faint)">' +
      "Nothing is tagged here yet. Untagged manifests can still exist: a registry keeps them until a " +
      "garbage collection, exactly as Docker Registry does.</dd></div>";
    return;
  }

  const list = document.createElement("div");
  list.style.cssText = "grid-column:1/-1;display:flex;flex-direction:column;gap:8px";
  facts.appendChild(list);

  for (const tag of [...r.tags].reverse()) {
    const row = document.createElement("div");
    row.style.cssText = "border:1px solid var(--line);border-radius:9px;padding:10px 12px;display:flex;flex-direction:column;gap:8px";
    row.innerHTML =
      '<div style="display:flex;align-items:center;gap:10px">' +
      '<span style="font-family:var(--mono);font-size:13px">' + tag + "</span>" +
      '<span class="dg" style="flex:1">…</span>' +
      '<button class="copy check">Check</button>' +
      (canDelete ? '<button class="copy del">Delete</button>' : "") +
      "</div><div class=\"detail\" hidden style=\"padding:0\"></div>";
    list.appendChild(row);

    const detail = row.querySelector(".detail");
    reg.manifest(r.repo, tag).then((m) => {
      row._m = m;
      const layers = m.body.layers || [];
      const total = layers.reduce((a, l) => a + (l.size || 0), 0);
      row.querySelector(".dg").textContent =
        (m.digest ? m.digest.slice(0, 17) + "…" : "") + (layers.length ? ` · ${layers.length} layers · ${bytes(total)}` : "");
    }).catch(() => { row.querySelector(".dg").textContent = "unreadable"; });

    row.querySelector(".check").addEventListener("click", () => checkTag(r.repo, tag, row, detail));
    const del = row.querySelector(".del");
    if (del) del.addEventListener("click", () => askToken(r.repo, tag, row, detail));
  }
}

// The one thing this page can do that a command line cannot: fetch the bytes and check them here.
let running = null;
async function checkTag(repo, tag, row, detail) {
  const button = row.querySelector(".check");
  if (running === tag) { running = null; button.textContent = "Check"; return; }
  running = tag;
  button.textContent = "Stop";
  detail.hidden = false;
  detail.innerHTML = '<span class="barp"><i></i></span><p class="result">Reading the manifest…</p><div class="layers"></div>';
  const bar = detail.querySelector("i"), result = detail.querySelector(".result"), list = detail.querySelector(".layers");

  let m = row._m;
  try { if (!m) { m = await reg.manifest(repo, tag); row._m = m; } }
  catch (e) { result.innerHTML = '<b class="bad">Could not read the manifest.</b>'; running = null; button.textContent = "Check"; return; }

  const take = (m.body.layers || []).slice(0, 12);
  let moved = 0, bad = 0;
  const started = performance.now();
  for (let i = 0; i < take.length; i++) {
    if (running !== tag) { result.textContent = `Stopped after ${i} layers.`; break; }
    const [algo, want] = take[i].digest.split(":");
    const line = document.createElement("div");
    line.innerHTML = '<span class="d"></span><span class="s">checking</span>';
    line.querySelector(".d").textContent = take[i].digest.slice(0, 22) + "…";
    list.appendChild(line);
    list.scrollTop = list.scrollHeight;
    try {
      const res = await reg.blob(repo, take[i].digest);
      const ok = (await digestOf(algo, res.body)) === want;
      if (!ok) bad++;
      moved += take[i].size || 0;
      line.querySelector(".s").className = "s " + (ok ? "ok" : "bad");
      line.querySelector(".s").textContent = ok ? "matches" : "does not match";
    } catch {
      bad++;
      line.querySelector(".s").className = "s bad";
      line.querySelector(".s").textContent = "failed";
    }
    bar.style.width = ((i + 1) / take.length) * 100 + "%";
  }
  if (running === tag) {
    const secs = ((performance.now() - started) / 1000).toFixed(1);
    result.innerHTML = bad
      ? `<b class="bad">${bad} of ${take.length} did not match</b> and would not be saved · ${bytes(moved)} in ${secs}s`
      : `<b>All ${take.length} checked layers match their addresses</b> · ${bytes(moved)} in ${secs}s`;
  }
  running = null;
  button.textContent = "Check";
}

// Deleting is the only thing here that changes the registry: it asks plainly, and the token stays in memory.
function askToken(repo, tag, row, detail) {
  detail.hidden = false;
  detail.innerHTML = '<p class="result">Remove the tag ' + tag + '. The bytes stay until a garbage collection runs.</p>' +
    '<div class="token"><input type="password" placeholder="Operator token, or user:password" autocomplete="off">' +
    '<button>Delete</button><button class="cancel">Cancel</button></div>';
  const say = detail.querySelector(".result");
  const input = detail.querySelector("input");
  input.focus();
  detail.querySelector(".cancel").addEventListener("click", () => { detail.hidden = true; detail.innerHTML = ""; });
  detail.querySelector(".token button").addEventListener("click", async () => {
    const res = await reg.del(repo, tag, input.value.trim()).catch(() => null);
    if (res && res.status === 202) {
      say.innerHTML = `<b>${tag} deleted.</b> The bytes remain until a garbage collection.`;
      detail.querySelector(".token").remove();
      row.remove();
      return;
    }
    const code = res ? res.status : "no answer";
    say.innerHTML = code === 401
      ? '<b class="bad">That token was not accepted.</b> Pushing and deleting are for the operator.'
      : `<b class="bad">Refused with ${code}.</b>`;
  });
}

function cover(img, r) {
  // Covers are vendored at build time and served from this origin. Nothing here reaches a third party,
  // so scrolling the list tells nobody outside what you looked at.
  if (r.cover) {
    img.src = r.cover;
    img.addEventListener("error", () => { if (r.logo) img.src = r.logo; else img.style.visibility = "hidden"; });
    return;
  }
  if (r.logo) { img.src = r.logo; return; }        // the mark drawn from the name, a data url
  img.style.visibility = "hidden";
}

// The command that actually uses this thing, which is not the same command for every kind.
function useCommand(r) {
  // docker.io is implied, and library/ is how Docker writes "official" in a path, not in a command.
  const ref = r.id.replace(/^docker\.io\//, "").replace(/^library\//, "");
  if (r.kind === "Model") return ["Pull the model", "docker model pull " + ref];
  if (r.kind === "Skill") return ["Run the server", "docker mcp gateway run " + r.name];
  if (r.kind === "Helm chart") return ["Install the chart", "helm install " + r.name + " oci://<repository>/" + r.name];
  if (r.registry === "Artifact Hub") return ["Find it", "open " + (r.home || "https://artifacthub.io")];
  if (r.kind === "Artifact") return ["Pull the artifact", "oras pull " + r.id + (r.tag ? ":" + r.tag : "")];
  return ["Pull it", "docker pull " + ref];
}

function openSheet(r) {
  if (r.live) return openOurs(r);
  const [label, command] = useCommand(r);
  cover($("s-logo"), r);
  $("s-name").textContent = r.id;
  $("s-desc").textContent = r.description || "No description from this source.";
  $("s-cmdlabel").textContent = label;
  $("s-cmd").textContent = command;
  const tags = [];
  if (r.here) tags.push('<span class="tag here">here</span>');
  if (r.official) tags.push('<span class="tag official">official</span>');
  if (r.verified) tags.push('<span class="tag">verified</span>');
  if (r.signed) tags.push('<span class="tag">signed</span>');
  tags.push('<span class="tag">' + r.registry + "</span>");
  $("s-tags").innerHTML = tags.join("");

  const facts = [
    ["Kind", r.kind],
    ["Publisher", r.publisher],
    ["Category", r.category],
    ["Licence", r.license],
    ["Pulls", r.pulls == null ? null : num(r.pulls)],
    ["Stars", r.stars == null ? null : num(r.stars)],
    ["Last updated", r.updatedBucket],
    ["Repository size", r.repoBucket],
    ["Platforms", (r.architectures || []).join(", ") || null],
    ["Digest", r.digest ? r.digest.slice(0, 20) + "…" : null],
  ].filter(([, v]) => v != null && v !== "");
  $("s-facts").innerHTML = facts.map(([k]) => '<div><dt>' + k + "</dt><dd></dd></div>").join("");
  [...$("s-facts").querySelectorAll("dd")].forEach((dd, i) => { dd.textContent = facts[i][1]; });

  const home = $("s-home");
  if (r.home) { home.hidden = false; home.href = r.home; home.textContent = "Open on " + r.registry; }
  else home.hidden = true;
  const here = $("s-here");
  here.hidden = !r.here;
  if (r.here) here.href = "/#/" + r.id.split("/").slice(1).join("/");

  $("sheet").hidden = false;
  $("scrim").hidden = false;
  $("sheet-close").focus();
}

function closeSheet() {
  $("sheet").hidden = true;
  $("scrim").hidden = true;
}

function card(r) {
  const el = document.createElement("button");
  el.type = "button";
  el.className = "card" + (r.here ? " ours" : "");
  const tags = [];
  if (r.here) tags.push(`<span class="tag here">here</span>`);
  if (r.official) tags.push(`<span class="tag official">official</span>`);
  tags.push(`<span class="tag"></span>`);
  if (r.signed) tags.push(`<span class="tag">signed</span>`);
  el.innerHTML = `<img class="logo" alt="" loading="lazy"><div class="tags">${tags.join("")}</div><h3></h3><p></p><div class="foot"></div>${art(r.id)}`;
  cover(el.querySelector("img"), r);
  el.addEventListener("click", () => openSheet(r));
  el.querySelectorAll(".tag")[r.here || r.official ? (r.here && r.official ? 2 : 1) : 0].textContent = r.registry;
  el.querySelector("h3").textContent = r.id.replace(/^docker\.io\//, "");
  el.querySelector("p").textContent = r.description || "No description from this source.";
  const foot = [];
  if (r.pulls != null) foot.push(`${num(r.pulls)} pulls`);
  if (r.stars != null && r.stars > 0) foot.push(`${num(r.stars)} stars`);
  if (r.size != null) foot.push(bytes(r.size));
  if ((r.architectures || []).length) foot.push(`${r.architectures.length} platforms`);
  if (r.here) foot.push("checkable");
  el.querySelector(".foot").textContent = foot.join(" · ") || r.provenance;
  el.title = `${r.id}\n${r.provenance}`;
  return el;
}

function render() {
  const referenced = resolve();
  const rows = referenced ? sorted(referenced.filter((r) => matches(r) || true)) : sorted(data.images.filter(matches));
  $("grid").innerHTML = "";
  rows.slice(0, shown).forEach((r) => $("grid").appendChild(card(r)));
  $("count").textContent = rows.length;
  $("empty").hidden = rows.length > 0;
  $("more").hidden = rows.length <= shown;
}

// The rail is the one every catalogue page has (lib/rail.mjs), with the Models rail's grouping: what you
// reach for first on Main, the rest one click away. Counts come from the index file; the registry's own rows
// are added to them when they are read live (start, below).
const rail = createRail({
  el: $("rail"),
  tabs: [
    ["Main", I.grid, ["registry", "category", "marks"]],
    ["Kind", I.layers, ["kind", "architecture"]],
    ["Publisher", I.seal, ["publisher"]],
    ["Licence", I.tag, ["license"]],
    ["Size", I.box, ["repoSize"]],
    ["Updated", I.clock, ["updated"]],
  ],
  labels: { registry: "Registry", category: "Category", marks: "Marks", kind: "Kind",
    architecture: "Architecture", publisher: "Publisher", license: "Licence",
    repoSize: "Repository size", updated: "Last updated" },
  icons: { registry: I.grid, category: I.tag, marks: I.seal, kind: I.layers, architecture: I.chip,
    publisher: I.seal, license: I.tag, repoSize: I.box, updated: I.clock },
  counts: (key) => data.facets[key],
  picked,
  onChange: () => { shown = PAGE; render(); },
  order: {
    updated: ["Today", "This week", "This month", "This year", "Over a year ago"],
    repoSize: ["Under 100 MB", "100 MB to 1 GB", "1 to 10 GB", "Over 10 GB"],
  },
  // Ours leads the registry list however small it is: it is the one whose bytes you can check here.
  lead: { registry: { value: "Hologram", className: "ours", title: "Derived from our own registry: these rows carry a digest you can check here" } },
});

(async function start() {
  data = await (await fetch("data/images.json")).json();
  $("host").textContent = location.host + "/v2/";

  // Ours are read now, not from yesterday's file, and they lead the list.
  const live = (await reg.base()) ? await liveRows() : [];
  $("dot").className = "dot " + (live.length ? "ok" : "bad");
  if (live.length) {
    data.images = [...live, ...data.images.filter((r) => !r.here)];
    data.facets.registry = { ...data.facets.registry, Hologram: live.length };
    data.facets.kind = { ...data.facets.kind, Artifact: (data.facets.kind.Artifact || 0) + live.length };
    data.facets.marks = { ...data.facets.marks, "Addressed here": live.length };
    data.facets.publisher = { ...data.facets.publisher };
    for (const r of live) data.facets.publisher[r.publisher] = (data.facets.publisher[r.publisher] || 0) + 1;
    data.totals.here = live.length;
    probe(live[0] && live[0].repo);
  }
  $("prov").textContent = `${data.totals.here} here, read live · ${data.totals.elsewhere} indexed elsewhere`;
  $("sources").textContent = "Sources: " + Object.keys(data.sources).join(", ") + ".";
  rail.render();
  render();
  $("q").addEventListener("input", () => { shown = PAGE; render(); });
  $("sheet-close").addEventListener("click", closeSheet);
  $("scrim").addEventListener("click", closeSheet);
  $("s-copy").addEventListener("click", () => {
    navigator.clipboard.writeText($("s-cmd").textContent);
    $("s-copy").textContent = "Copied";
    setTimeout(() => ($("s-copy").textContent = "Copy"), 1200);
  });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("sheet").hidden) closeSheet(); });
  $("sort").addEventListener("change", () => { shown = PAGE; render(); });
  $("more").addEventListener("click", () => { shown += PAGE; render(); });
})();
