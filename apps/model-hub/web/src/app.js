import * as R from "./render.mjs";
import * as B from "./braille.mjs";

const base = document.documentElement.dataset.base;
const $ = (s, el = document) => el.querySelector(s);

themeSwitch();
B.play();
let view = null; // set by browse(): lets the archive swap the catalog under the same interface
if ($("#browse")) browse();
if ($("[data-verify]")) model();
copyButtons();
if ($("#archive")) archive();

async function browse() {
  const data = await fetch(`${base}data/models.json`).then((r) => r.json());
  let models = R.prepare(data.models, data.snapshot);
  let state = R.parseState(location.search);
  const filters = $("#filters-body"), grid = $("#grid"), pager = $("#pager"), total = $("#total"), q = $("#q");
  const facetSearch = {}, openMore = new Set(), order = {};
  q.value = state.q;

  function render(push) {
    const r = R.query(models, state, order);
    state.page = r.page;
    const focus = document.activeElement?.dataset?.facetSearch;
    filters.innerHTML = R.filters(r, state, order);
    grid.innerHTML = R.grid(r, { base });
    const at = document.documentElement.dataset.at;
    if (at) for (const a of grid.querySelectorAll("a.card")) a.href = `${a.getAttribute("href")}?at=${at}`;
    B.play(grid);
    pager.innerHTML = R.pager(r, state);
    total.textContent = r.results.length.toLocaleString("en-US");
    $("#sheet-count").textContent = total.textContent;
    clampAll();
    for (const [key, text] of Object.entries(facetSearch)) {
      const input = filters.querySelector(`[data-facet-search="${key}"]`);
      if (input) { input.value = text; narrow(input); if (focus === key) { input.focus(); input.setSelectionRange(text.length, text.length); } }
    }
    syncSort();
    document.title = R.title(state);
    const search = R.stateToSearch(state);
    const url = `${location.pathname}${at ? `${search ? `${search}&` : "?"}at=${at}` : search}`;
    if (push === "push") history.pushState(null, "", url);
    else if (push === "replace") history.replaceState(null, "", url);
  }

  function clampAll() {
    for (const section of filters.querySelectorAll(".facet")) clamp(section, openMore.has(section.dataset.key));
  }

  function clamp(section, open) {
    const chips = section.querySelector(".chips"), more = section.querySelector(".more");
    chips.classList.add("clamped");
    const hidden = [...chips.children].filter((c) => c.offsetTop - chips.offsetTop >= chips.clientHeight).length;
    if (!hidden) { chips.classList.remove("clamped"); more.hidden = true; return; }
    more.hidden = false;
    more.textContent = open ? "Show less" : `+${hidden} more`;
    if (open) chips.classList.remove("clamped");
    more.onclick = () => {
      const next = chips.classList.contains("clamped");
      next ? openMore.add(section.dataset.key) : openMore.delete(section.dataset.key);
      clamp(section, next);
    };
  }

  function narrow(input) {
    const text = input.value.trim().toLowerCase();
    facetSearch[input.dataset.facetSearch] = input.value;
    const section = input.closest(".facet"), chips = section.querySelector(".chips");
    for (const chip of chips.children) if (chip.dataset.value) chip.hidden = !!text && !chip.dataset.value.toLowerCase().includes(text);
    if (text) { chips.classList.remove("clamped"); section.querySelector(".more").hidden = true; }
    else clamp(section, false);
  }

  const change = (fn) => { fn(); state.page = 1; render("push"); };

  // Verified only: the same filter as Status → Verified, one tap away.
  const verifiedOnly = $("#verified-only");
  const isVerifiedOnly = () => state.f.stateLabel?.length === 1 && state.f.stateLabel[0] === "Verified";
  verifiedOnly.addEventListener("click", () => change(() => {
    if (isVerifiedOnly()) delete state.f.stateLabel; else state.f.stateLabel = ["Verified"];
  }));

  filters.addEventListener("click", (e) => {
    const chip = e.target.closest(".chip"), tab = e.target.closest(".tab"), reset = e.target.closest("[data-reset]"), sorter = e.target.closest("[data-order]");
    if (sorter) { const k = sorter.dataset.order; order[k] = order[k] === "az" ? "count" : "az"; render(); }
    if (chip) change(() => {
      const list = new Set(state.f[chip.dataset.facet] || []);
      list.has(chip.dataset.value) ? list.delete(chip.dataset.value) : list.add(chip.dataset.value);
      state.f[chip.dataset.facet] = [...list];
    });
    if (tab) { state.tab = tab.dataset.tab; render("replace"); }
    if (reset) change(() => { delete state.f[reset.dataset.reset]; });
  });
  filters.addEventListener("input", (e) => { if (e.target.dataset.facetSearch) narrow(e.target); });

  let typing;
  q.addEventListener("input", () => {
    clearTimeout(typing);
    typing = setTimeout(() => { state.q = q.value.trim(); state.page = 1; render("replace"); }, 144);
  });

  document.addEventListener("click", (e) => {
    if (e.target.closest("[data-clear]")) change(() => { state.f = {}; state.q = ""; q.value = ""; });
    const page = e.target.closest("[data-page]");
    if (page && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      state.page = Number(page.dataset.page);
      render("push");
      $("#results").scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    }
  });

  // sort menu
  const sortButton = $("#sort"), list = $("#sort-list");
  function syncSort() {
    verifiedOnly.setAttribute("aria-checked", String(isVerifiedOnly()));
    $("#sort-label").textContent = R.SORTS.find(([k]) => k === state.sort)[1];
    for (const o of list.children) o.setAttribute("aria-selected", o.dataset.sort === state.sort);
  }
  const openMenu = (open) => {
    list.hidden = !open;
    sortButton.setAttribute("aria-expanded", open);
    if (open) [...list.children].forEach((o) => o.classList.toggle("hot", o.dataset.sort === state.sort));
  };
  sortButton.addEventListener("click", () => openMenu(list.hidden));
  list.addEventListener("click", (e) => {
    const o = e.target.closest("[data-sort]");
    if (o) { openMenu(false); change(() => { state.sort = o.dataset.sort; }); sortButton.focus(); }
  });
  sortButton.addEventListener("keydown", (e) => {
    if (list.hidden && (e.key === "ArrowDown" || e.key === "ArrowUp")) { e.preventDefault(); openMenu(true); return; }
    if (list.hidden) return;
    const items = [...list.children], i = items.findIndex((o) => o.classList.contains("hot"));
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = (i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
      items.forEach((o, k) => o.classList.toggle("hot", k === n));
    }
    if (e.key === "Enter") { e.preventDefault(); items[i]?.click(); }
    if (e.key === "Escape") openMenu(false);
  });
  document.addEventListener("click", (e) => { if (!e.target.closest(".sort")) openMenu(false); });

  // filters as a sheet on narrow screens
  $("#open-filters").addEventListener("click", () => { document.body.classList.add("sheet"); clampAll(); });
  $("#sheet-done").addEventListener("click", () => document.body.classList.remove("sheet"));
  $("#close-filters").addEventListener("click", () => document.body.classList.remove("sheet"));
  document.addEventListener("keydown", (e) => {
    if (e.key === "/" && document.activeElement.tagName !== "INPUT") { e.preventDefault(); q.focus(); }
    if (e.key === "Escape") document.body.classList.remove("sheet");
  });

  window.addEventListener("popstate", () => { state = R.parseState(location.search); q.value = state.q; render(); });
  view = { setCatalog(list, snapshot) { models = R.prepare(list, snapshot); state.page = 1; render("replace"); } };
  render("replace");
}

// The Archive: open any captured day. Bytes come from the IPFS gateway by CID; the browser verifies the day's index
// against the BLAKE3 the ledger records, then every file against the index, before anything is shown.
function archive() {
  const box = $("#archive"), button = $("#archive-button"), menu = $("#archive-menu"), label = $("#archive-label");
  const banner = $("#archive-banner"), ledger = JSON.parse($("#archive-days").textContent);
  const days = ledger.days; // newest first
  const params = new URLSearchParams(location.search);
  const wanted = params.get("at");
  const resolve = (date) => days.find((d) => d.date <= date) || null; // nearest earlier captured day
  let hasher = null;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));

  // ---- menu
  const items = () => [...menu.querySelectorAll('[role="menuitemradio"]')];
  const open = (show, focusFirst) => {
    menu.hidden = !show;
    button.setAttribute("aria-expanded", String(show));
    if (show && focusFirst) (menu.querySelector('[aria-checked="true"]') || items()[0]).focus();
  };
  button.addEventListener("click", (e) => { e.stopPropagation(); open(menu.hidden, e.detail === 0); });
  document.addEventListener("click", (e) => { if (!menu.hidden && !e.target.closest("#archive")) open(false); });
  let typed = "", typedAt = 0;
  menu.addEventListener("keydown", (e) => {
    const list = items(), i = list.indexOf(document.activeElement);
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (step) { e.preventDefault(); list[(i + step + list.length) % list.length].focus(); return; }
    if (e.key === "Escape") { open(false); button.focus(); return; }
    if (e.key.length === 1 && /[a-z0-9 ]/i.test(e.key)) {
      typed = (Date.now() - typedAt < 900 ? typed : "") + e.key.toLowerCase();
      typedAt = Date.now();
      const hit = list.find((b) => b.textContent.trim().toLowerCase().startsWith(typed));
      if (hit) hit.focus();
    }
  });
  document.addEventListener("click", (e) => {
    const choice = e.target.closest("[data-at]");
    if (!choice) return;
    open(false);
    go(choice.dataset.at);
  });

  function mark(date) {
    for (const b of items()) b.setAttribute("aria-checked", String(b.dataset.at === (date || "latest")));
    const entry = date ? days.find((d) => d.date === date) : null;
    $("#archive-cid").dataset.copy = entry?.cid || "";
    $("#archive-cid").hidden = !entry;
    $("#archive-pull").dataset.copy = entry ? `hologram pull ${entry.reference}` : "";
    $("#archive-pull").hidden = !entry;
  }

  function go(date) {
    const url = new URL(location.href);
    if (date === "latest") { url.searchParams.delete("at"); location.href = url.toString(); return; }
    url.searchParams.set("at", date);
    history.replaceState(null, "", url.toString());
    show(resolve(date));
  }

  // ---- verified reads
  // Every file is checked against the BLAKE3 the day's index records (and the index against the ledger), so where
  // the bytes come from is only a matter of speed: the hub's mirror answers in milliseconds, the IPFS gateway can
  // take tens of seconds on a cold day. Verified bytes are kept in the Cache API under the content address.
  const timed = async (url, ms) => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), ms);
    try { return await fetch(url, { signal: controller.signal }); } finally { clearTimeout(timer); }
  };
  async function verified(entry, path, expect) {
    const key = `${ledger.gateway}${entry.cid}/${path}`;
    const store = await caches.open("model-hub-archive").catch(() => null);
    const cached = store ? await store.match(key) : null;
    if (cached) return new Uint8Array(await cached.arrayBuffer());
    hasher ||= (await import("https://humuhumu33.github.io/hologram-api/vendor/hash-wasm/index.esm.min.js")).createBLAKE3;
    const sources = [];
    if (ledger.mirror) sources.push([`${ledger.mirror}${entry.date}/${path}`, 8000]);
    sources.push([key, 30000]);
    let failure = "";
    for (const [url, ms] of sources) {
      const host = new URL(url).host;
      let bytes;
      try {
        const r = await timed(url, ms);
        if (!r.ok) { failure = `${host} answered ${r.status}`; continue; }
        bytes = new Uint8Array(await r.arrayBuffer());
      } catch { failure = `${host} did not answer`; continue; }
      const h = await hasher();
      h.update(bytes);
      if (`blake3:${h.digest("hex")}` !== expect) { failure = `${host} served bytes that do not match the address`; continue; }
      if (store) await store.put(key, new Response(bytes, { headers: { "content-type": "application/json" } })).catch(() => {});
      return bytes;
    }
    throw new Error(`${path}: ${failure}`);
  }

  async function show(entry) {
    if (!entry) { go("latest"); return; }
    label.textContent = `Loading ${R.day(entry.date)}`;
    box.classList.add("busy");
    try {
      const index = JSON.parse(new TextDecoder().decode(await verified(entry, "index.json", entry.index)));
      const addressOf = new Map(index.files.map(([path, address]) => [path, address]));
      const read = async (path) => (addressOf.has(path) ? JSON.parse(new TextDecoder().decode(await verified(entry, path, addressOf.get(path)))) : null);
      const catalog = await read("models.json");
      document.documentElement.dataset.at = entry.date;
      label.textContent = `Index ${R.day(entry.date)}`;
      box.classList.add("past");
      $("#archive-banner-date").textContent = R.day(entry.date);
      banner.hidden = false;
      mark(entry.date);
      if (view) view.setCatalog(catalog.models, catalog.snapshot);
      const id = document.documentElement.dataset.model;
      if (id) await showModel(entry, catalog, read, id);
      document.title = `${document.title.replace(/ · Index .*$/, "")} · Index ${R.day(entry.date)}`;
    } catch (error) {
      label.textContent = `Index ${R.day(ledger.latest)}`;
      banner.hidden = false;
      $("#archive-banner-date").textContent = R.day(entry.date);
      banner.querySelector("span").innerHTML = `This day could not be loaded (${R.esc(error.message)}). <button type="button" class="link" data-at="${entry.date}">Try again</button>`;
    } finally {
      box.classList.remove("busy");
    }
  }

  // A model page on a past day: that day's rank, downloads and files. Verify and downloads stay with the latest
  // index, because they check live mirrors.
  async function showModel(entry, catalog, read, id) {
    const m = catalog.models.find((x) => x.id === id);
    const note = $("#verdict");
    for (const b of document.querySelectorAll("[data-verify], #dl-all, [data-zip]")) { b.disabled = true; b.title = "Verify and downloads use the latest index. Switch to Latest."; }
    if (!m) {
      const nearest = days.find((d) => d.date > entry.date) || days[0];
      note.hidden = false; note.className = "verdict";
      note.innerHTML = `Not in the index on ${R.day(entry.date)}. <button type="button" class="link" data-at="${nearest.date}">Open ${R.day(nearest.date)}</button>`;
      return;
    }
    for (const dt of document.querySelectorAll(".facts dt")) {
      const dd = dt.nextElementSibling;
      if (dt.textContent === "Trending") dd.textContent = `#${m.rank}`;
      if (dt.textContent.startsWith("Downloads")) dd.textContent = R.count(m.downloads);
      if (dt.textContent === "Status") dd.firstElementChild.textContent = R.STATE_LABEL[m.state];
    }
    const files = await read(`files/${m.org}/${m.name}.json`);
    const table = $("#files");
    if (!files || !table) return;
    const copy = (text, shown) => `<button type="button" class="copy" data-copy="${R.esc(text)}" aria-label="Copy ${R.esc(text)}">${R.esc(shown)}${R.icon.copy}</button>`;
    table.querySelector("thead tr").innerHTML = `<th>Path</th><th class="size">Size</th><th>Address</th>`;
    table.tBodies[0].innerHTML = files.files.map(([path, size, address]) => `<tr data-path="${R.esc(path)}" data-size="${size ?? 0}" data-address="${R.esc(address)}"><td class="path" title="${R.esc(path)}">${R.esc(path)}</td><td class="size">${R.bytes(size)}</td><td class="addr">${copy(address, R.shortAddress(address))}</td></tr>`).join("");
    const pill = $("#tab-files .pill");
    if (pill) pill.textContent = files.files.length;
    const head = $(".files-head .note");
    if (head) head.textContent = `${files.files.length} files on ${R.day(entry.date)}, revision ${files.revision.slice(0, 12)}.`;
  }

  mark(null);
  if (wanted) show(resolve(wanted));
}

function model() {
  const button = $("[data-verify]"), out = $("#verdict");
  const id = button.dataset.verify, pinned = button.dataset.manifest;
  const glyph = [...document.querySelectorAll("#glyph .cell")];
  const sources = JSON.parse($("#sources")?.textContent || "[]");
  const mark = (kind, state) => {
    const li = document.querySelector(`.sources li[data-source="${kind}"]`);
    if (!li) return;
    li.dataset.state = state;
    if (state === "busy") B.play(li);
  };
  const pinnedBytes = B.hexToBytes(pinned.split(":")[1]);
  const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const wait = (ms) => new Promise((r) => setTimeout(r, calm ? 0 : ms));

  // While the browser fetches and hashes, the cells search. When the digest is known, each byte locks
  // left to right and is compared with the pinned address: every lit dot is a real bit of the result.
  let searching = false;
  function search() {
    if (!searching || calm) return;
    for (const svg of glyph) if (!svg.classList.contains("ok") && !svg.classList.contains("bad")) B.setCell(svg, (Math.random() * 256) | 0);
    setTimeout(search, 70);
  }
  async function lock(received) {
    for (let i = 0; i < glyph.length; i++) {
      const byte = received ? received[i] : pinnedBytes[i];
      B.setCell(glyph[i], byte, received && byte === pinnedBytes[i] ? "ok lock" : "bad");
      const svg = glyph[i];
      setTimeout(() => svg.classList.remove("lock"), 260);
      await wait(34);
    }
    searching = false;
  }

  button.addEventListener("click", async () => {
    button.disabled = true;
    button.classList.add("busy");
    B.play(button);
    out.hidden = false;
    out.className = "verdict";
    out.textContent = "Checking the bytes in your browser.";
    for (const svg of glyph) svg.setAttribute("class", "cell");
    for (const s of sources) mark(s.kind, "");
    searching = true;
    search();
    const started = performance.now();
    let received = null;
    try {
      const api = await import("https://humuhumu33.github.io/hologram-api/hologram.js");
      const doc = await api.resolve(id, { manifest: pinned });
      received = B.hexToBytes(doc.manifest.split(":")[1]);
      // The expected address comes from the index; each source only supplies bytes.
      const file = doc.files.find((f) => f.path === button.dataset.probe);
      // Peer to peer sources are checked piece by piece by the torrent client; the browser verifies HTTP sources.
      const results = await Promise.all(sources.filter((s) => !s.p2p && !s.pull).map(async (s) => {
        mark(s.kind, "busy");
        if (!file) { mark(s.kind, "ok"); return { s, ok: true }; }
        const url = s.resolve ? s.resolve + file.path.split("/").map(encodeURIComponent).join("/") : file.url;
        try { await api.fetchVerified(url, file.address); mark(s.kind, "ok"); return { s, ok: true }; }
        catch (e) { mark(s.kind, "bad"); return { s, ok: false, mismatch: e.code === "ADDRESS_MISMATCH" }; }
      }));
      const ms = Math.round(performance.now() - started);
      await lock(received);
      const good = results.filter((r) => r.ok).map((r) => r.s.name), bad = results.filter((r) => !r.ok);
      const list = (names) => (names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}` : names[0]);
      if (bad.some((r) => r.mismatch)) {
        out.className = "verdict bad";
        out.textContent = `${list(bad.filter((r) => r.mismatch).map((r) => r.s.name))} served different bytes. Do not use that copy.`;
      } else if (bad.length) {
        out.className = good.length ? "verdict ok" : "verdict bad";
        out.textContent = good.length ? `Verified in ${ms} ms from ${list(good)}. ${list(bad.map((r) => r.s.name))} could not be reached.` : "No source could be reached.";
      } else {
        out.className = "verdict ok";
        out.textContent = good.length > 1 ? `Verified in ${ms} ms. Identical bytes from ${list(good)}.` : `Verified in ${ms} ms from ${good[0]}.`;
      }
    } catch (error) {
      await lock(received);
      out.className = "verdict bad";
      out.textContent = error.code === "ADDRESS_MISMATCH" ? "These bytes do not match their address." : `Could not verify: ${error.message}`;
    } finally {
      searching = false;
      button.disabled = false;
      button.classList.remove("busy");
    }
  });

  downloads();
  panelTabs();

  const table = $("#files");
  if (table) {
    const body = table.tBodies[0];
    table.addEventListener("click", (e) => {
      const th = e.target.closest("[data-col]");
      if (!th) return;
      const col = th.dataset.col, rows = [...body.rows];
      const dir = th.getAttribute("aria-sort") === "ascending" ? -1 : 1;
      rows.sort((a, b) => {
        const x = a.dataset[col], y = b.dataset[col];
        return (col === "size" ? Number(x) - Number(y) : x.localeCompare(y)) * dir;
      });
      for (const other of table.querySelectorAll("[data-col]")) other.removeAttribute("aria-sort");
      th.setAttribute("aria-sort", dir === 1 ? "ascending" : "descending");
      body.append(...rows);
    });
  }
}

// Overview and Files: tabs that keep their place in the URL hash and move with arrow keys.
function panelTabs() {
  const tabs = [...document.querySelectorAll(".panel-tabs [role=tab]")];
  if (!tabs.length) return;
  const select = (tab, focus) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    }
    if (focus) tab.focus();
    const hash = tab.id === "tab-files" ? "#files" : "";
    if (location.hash !== hash) history.replaceState(null, "", `${location.pathname}${location.search}${hash}`);
  };
  for (const t of tabs) {
    t.addEventListener("click", () => select(t));
    t.addEventListener("keydown", (e) => {
      const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (step) { e.preventDefault(); select(tabs[(tabs.indexOf(t) + step + tabs.length) % tabs.length], true); }
    });
  }
  if (location.hash === "#files") select(tabs[1]);
}

// Every download is checked against the index address before it is kept.
function downloads() {
  const table = $("#files");
  if (!table) return;
  const progress = $("#dl-progress");
  const MEMORY_LIMIT = 256 * 1024 * 1024;
  const hex = (buf) => [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
  const rowOf = (el) => el.closest("tr");
  const name = (path) => path.split("/").pop();
  const hero = $("#dl-status"), toggle = $("#dl-all"), menu = $("#dl-menu");
  const say = (text, tone = "") => {
    for (const el of [progress, hero]) { if (!el) continue; el.className = el === hero ? `verdict ${tone}` : `progress ${tone}`; el.textContent = text; }
    progress.hidden = !text;
    if (hero) hero.hidden = !text || !$("#pane-files")?.hidden;
  };

  function save(blob, filename) {
    const a = Object.assign(document.createElement("a"), { href: URL.createObjectURL(blob), download: filename });
    document.body.append(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 30000);
  }

  // Single file: small files are fetched, hashed and saved in the browser; large files and sources that do not
  // allow browser fetches download directly (the source's own download, unchecked by this page).
  table.addEventListener("click", async (e) => {
    const link = e.target.closest("a[data-download]");
    if (!link) return;
    const row = rowOf(link), size = Number(row.dataset.size), path = row.dataset.path;
    if (size > MEMORY_LIMIT) return;
    e.preventDefault();
    if (link.classList.contains("busy")) return;
    link.classList.add("busy");
    try {
      const response = await fetch(link.href);
      if (!response.ok) throw new Error(String(response.status));
      const bytes = await response.arrayBuffer();
      const got = `sha256:${hex(await crypto.subtle.digest("SHA-256", bytes))}`;
      if (got !== row.dataset.address) {
        link.classList.add("bad");
        say(`${link.dataset.source} served different bytes for ${path}. The file was not saved.`, "bad");
        return;
      }
      save(new Blob([bytes]), name(path));
      link.classList.add("done");
      say(`Saved ${name(path)} from ${link.dataset.source}. It matches its address.`, "ok");
    } catch {
      window.location.href = link.href;
    } finally {
      link.classList.remove("busy");
    }
  });

  const head = $(".files-head");
  const files = () => [...table.tBodies[0].rows].map((row) => ({
    path: row.dataset.path, size: Number(row.dataset.size), address: row.dataset.address,
    links: [...row.querySelectorAll("a[data-download]")].map((a) => ({ source: a.dataset.source, href: a.href })),
  }));

  // Terminal script: every file, every source, SHA-256 checked at the end.
  // The hero Download menu
  const open = (show) => { if (!menu) return; menu.hidden = !show; toggle.setAttribute("aria-expanded", String(show)); };
  toggle?.addEventListener("click", (e) => { e.stopPropagation(); open(menu.hidden); });
  document.addEventListener("click", (e) => { if (menu && !menu.hidden && !e.target.closest(".download-all")) open(false); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") open(false); });
  menu?.addEventListener("click", (e) => { if (e.target.closest("[role=menuitem]")) open(false); });

  document.addEventListener("click", (e) => {
    if (!e.target.closest("[data-script]")) return;
    const list = files();
    const q = (s) => `'${String(s).replace(/'/g, "'\\''")}'`;
    const lines = list.map((f) => `get ${q(f.path)} ${f.links.map((l) => q(l.href)).join(" ")}`);
    const sums = list.map((f) => `${f.address.split(":")[1]}  ${f.path}`);
    const text = `#!/usr/bin/env sh
# ${head.dataset.repo} at ${head.dataset.revision}
# Downloads every file, trying each source in turn, then checks every SHA-256 against the Hologram index.
set -eu
mkdir -p ${q(head.dataset.name)} && cd ${q(head.dataset.name)}
get() { path="$1"; shift; mkdir -p "$(dirname "$path")"; for url in "$@"; do curl -fL --retry 3 -C - -o "$path" "$url" && return 0; done; echo "could not download $path" >&2; return 1; }
${lines.join("\n")}
cat > SHA256SUMS <<'SUMS'
${sums.join("\n")}
SUMS
if command -v sha256sum >/dev/null 2>&1; then sha256sum -c SHA256SUMS; else shasum -a 256 -c SHA256SUMS; fi
`;
    save(new Blob([text], { type: "text/x-shellscript" }), `${head.dataset.name}-download.sh`);
    say(`Saved ${head.dataset.name}-download.sh. Run it with sh in a terminal; it checks every file when done.`, "ok");
  });

  // One zip per source: every file streams straight into the archive while its SHA-256 is computed. With the
  // save picker (Chromium) nothing is held in memory; elsewhere the zip is assembled in memory up to a limit.
  const IN_MEMORY_LIMIT = 1.5e9;
  let cancelled = false;
  document.addEventListener("click", async (e) => {
    const button = e.target.closest("button[data-zip]");
    if (!button || button.classList.contains("busy") || toggle?.classList.contains("busy")) return;
    const source = button.dataset.zip;
    const label = toggle?.querySelector("span");
    const list = files().map((f) => ({ ...f, href: f.links.find((l) => l.source === source)?.href })).filter((f) => f.href);
    const skipped = table.tBodies[0].rows.length - list.length;
    const total = list.reduce((s, f) => s + f.size, 0);
    const zipName = `${head.dataset.name}-${head.dataset.revision.slice(0, 8)}.zip`;

    let out = null, parts = null;
    if (window.showSaveFilePicker) {
      try {
        const handle = await window.showSaveFilePicker({ suggestedName: zipName, types: [{ description: "Zip archive", accept: { "application/zip": [".zip"] } }] });
        out = await handle.createWritable();
      } catch { return; }
    } else if (total <= IN_MEMORY_LIMIT) {
      parts = [];
    } else {
      say(`${formatBytes(total)} is too large to assemble in this browser. Use Chrome or Edge, or the terminal script.`, "bad");
      return;
    }

    button.classList.add("busy");
    toggle?.classList.add("busy");
    cancelled = false;
    const cancel = Object.assign(document.createElement("button"), { type: "button", className: "link", textContent: "Cancel" });
    cancel.onclick = () => { cancelled = true; };
    const [{ ZipWriter }, { createSHA256 }] = await Promise.all([
      import("./zip.mjs"),
      import("https://humuhumu33.github.io/hologram-api/vendor/hash-wasm/index.esm.min.js"),
    ]);
    const zip = new ZipWriter((bytes) => (out ? out.write(bytes) : parts.push(bytes)));
    let done = 0, doneBytes = 0, failure = null;
    // The next response is requested while the current one streams, so the network never idles between files.
    let next = fetch(list[0].href);
    try {
      for (let i = 0; i < list.length; i++) {
        const f = list[i];
        const response = await next;
        if (i + 1 < list.length) next = fetch(list[i + 1].href);
        if (!response.ok || !response.body) throw new Error(`${source} answered ${response.status} for ${f.path}`);
        const hasher = await createSHA256();
        let last = 0;
        const watched = response.body.pipeThrough(new TransformStream({
          transform(chunk, controller) {
            if (cancelled) { controller.error(new Error("cancelled")); return; }
            doneBytes += chunk.byteLength;
            if (doneBytes - last > 4e6) {
              last = doneBytes;
              say(`Zipping ${done + 1} of ${list.length} from ${source}, ${formatBytes(doneBytes)} of ${formatBytes(total)}`);
              progress.append(" ", cancel);
              if (label) label.textContent = `${Math.min(99, Math.floor((doneBytes / total) * 100))}%`;
            }
            controller.enqueue(chunk);
          },
        }));
        await zip.add(f.path, f.size, watched, (chunk) => hasher.update(chunk));
        if (`sha256:${hasher.digest("hex")}` !== f.address) throw new Error(`${source} served different bytes for ${f.path}. Nothing was kept.`);
        done++;
      }
      await zip.finish();
    } catch (error) {
      failure = cancelled ? null : error;
    }
    if (failure || cancelled) {
      if (out) await out.abort().catch(() => {});
      say(cancelled ? `Stopped after ${done} of ${list.length} files.` : failure.message, cancelled ? "" : "bad");
      if (failure) button.classList.add("bad");
    } else {
      if (out) await out.close(); else save(new Blob(parts, { type: "application/zip" }), zipName);
      button.classList.add("done");
      say(`Saved ${zipName}: ${done} files from ${source}, every one matching its address.${skipped ? ` ${skipped} not on ${source} were left out.` : ""}`, "ok");
    }
    button.classList.remove("busy");
    toggle?.classList.remove("busy");
    if (label) label.textContent = "Download";
  });
}

function formatBytes(n) {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < u.length - 1) { n /= 1000; i++; }
  return `${i ? n.toFixed(1) : n} ${u[i]}`;
}

function copyButtons() {
  document.addEventListener("click", async (e) => {
    const b = e.target.closest("[data-copy]");
    if (!b) return;
    try {
      await navigator.clipboard.writeText(b.dataset.copy);
      const icon = b.querySelector(".i");
      const was = icon?.outerHTML;
      if (icon) icon.outerHTML = R.icon.check;
      setTimeout(() => { const now = b.querySelector(".i"); if (now && was) now.outerHTML = was; }, 1300);
    } catch {}
  });
}

// Dark, Light, Immersive. Dark for first visits; the choice is kept on this device.
function themeSwitch() {
  const KEY = "hologram-models-hub.theme";
  const root = document.documentElement, button = $("#theme-button"), menu = $("#theme-menu");
  if (!button) return;
  const walls = JSON.parse($("#wallpapers").textContent);
  const read = () => { try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch { return {}; } };
  let warmed = false;

  function sync() {
    const mode = root.dataset.theme, wall = root.dataset.wallpaper;
    for (const b of menu.querySelectorAll("[data-theme-mode]")) b.setAttribute("aria-checked", String(b.dataset.themeMode === mode));
    for (const b of menu.querySelectorAll("button.wall")) b.setAttribute("aria-checked", String(mode === "immersive" && b.dataset.wallpaper === wall));
    const w = walls.find((x) => x.key === wall);
    $("#walls").classList.toggle("on", mode === "immersive");
    $("#wall-credit").innerHTML = w ? `${w.name}, photo by <a href="${w.url}" target="_blank" rel="noopener">${w.by}</a> on Unsplash` : "";
  }

  function apply(mode, wallpaper = root.dataset.wallpaper) {
    const run = () => {
      root.dataset.theme = mode;
      root.dataset.wallpaper = wallpaper;
      root.classList.toggle("dark", mode !== "light");
      try { localStorage.setItem(KEY, JSON.stringify({ mode, wallpaper })); } catch {}
      sync();
    };
    const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (document.startViewTransition && !calm && !document.hidden) {
      const t = document.startViewTransition(run);
      for (const p of [t.ready, t.finished, t.updateCallbackDone]) p?.catch(() => {});
    } else run();
  }

  // Full size wallpapers load the moment the menu opens, so Immersive appears instantly.
  function warm() {
    if (warmed) return;
    warmed = true;
    for (const w of walls) { const img = new Image(); img.decoding = "async"; img.src = `${base}wallpapers/${w.key}.jpg`; }
  }

  const items = () => [...menu.querySelectorAll('[role="menuitemradio"]')];
  function open(show, focusFirst) {
    menu.hidden = !show;
    button.setAttribute("aria-expanded", String(show));
    if (show) { warm(); sync(); if (focusFirst) (menu.querySelector('[aria-checked="true"]') || items()[0]).focus(); }
  }

  button.addEventListener("click", (e) => { e.stopPropagation(); open(menu.hidden, e.detail === 0); });
  button.addEventListener("pointerenter", warm, { once: true });
  menu.addEventListener("click", (e) => {
    const mode = e.target.closest("button[data-theme-mode]"), wall = e.target.closest("button.wall");
    if (mode) apply(mode.dataset.themeMode);
    else if (wall) apply("immersive", wall.dataset.wallpaper);
  });
  menu.addEventListener("keydown", (e) => {
    const list = items(), i = list.indexOf(document.activeElement);
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
    if (step) { e.preventDefault(); list[(i + step + list.length) % list.length].focus(); }
    if (e.key === "Escape") { open(false); button.focus(); }
  });
  document.addEventListener("click", (e) => { if (!menu.hidden && !e.target.closest(".appearance")) open(false); });
  window.addEventListener("storage", (e) => { if (e.key === KEY) { const s = read(); if (s.mode) apply(s.mode, s.wallpaper || "alps"); } });
  sync();
}
