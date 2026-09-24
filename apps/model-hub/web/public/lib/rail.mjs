// One facet rail for the catalogue pages that ship as their own files: Registry, Spaces. The same rail the
// Models page has — tabs across the top, facets with counts under the chosen tab, a filter box and an A-to-Z
// toggle on the long ones, three rows then "+N more", Reset per facet — built once here so the two pages
// cannot drift from each other, and so a third page gets the same rail by asking for it.
//
//   const rail = createRail({ el, tabs, labels, icons, counts, picked, onChange });
//   rail.render();
//
// tabs     [[name, iconSvg, [facetKey, ...]], ...]         what the tab row says, and what each tab holds
// labels   { facetKey: "Facet name" }
// icons    { facetKey: iconSvg }                           the small mark on every chip of that facet
// counts   (facetKey) => { value: n, ... }                 how many rows carry each value, right now
// picked   { facetKey: Set }                               the caller's own filter state; the rail toggles it
// onChange ()                                              called after a chip or a Reset changed `picked`
// order    { facetKey: [value, ...] }                      optional: a fixed order for a facet's values
// lead     { facetKey: { value, className, title } }       optional: one value that leads its facet, marked

export const ICONS = {
  grid: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6"><rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/></svg>',
  tag: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M4 11V5a1 1 0 0 1 1-1h6l8 8-7 7-8-8Z"/></svg>',
  layers: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="m12 4 8 4-8 4-8-4 8-4Z"/><path d="m4 14 8 4 8-4"/></svg>',
  chip: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6"><rect x="7" y="7" width="10" height="10" rx="2"/><path d="M10 4v3M14 4v3M10 17v3M14 17v3M4 10h3M4 14h3M17 10h3M17 14h3"/></svg>',
  box: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M4 8l8-4 8 4v8l-8 4-8-4V8Z"/><path d="M4 8l8 4 8-4M12 12v8"/></svg>',
  seal: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M12 3l7 3v5c0 4.2-2.8 7.6-7 9-4.2-1.4-7-4.8-7-9V6l7-3Z"/><path d="m9 12 2 2 4-4"/></svg>',
  globe: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.8 3.3 2.8 13.7 0 17M12 3.5c-2.8 3.3-2.8 13.7 0 17"/></svg>',
  search: '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/></svg>',
  sort: '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M4 7h10M4 12h7M4 17h4"/><path d="m17 8 3-3 3 3M20 5v14"/></svg>',
  clock: '<svg class="i" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/></svg>',
  undo: '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M4 12a8 8 0 1 1 2.5 5.8"/><path d="M4 7v5h5"/></svg>',
};

const CLAMP_AT = 9;   // three rows of chips, then "+N more"
const SEARCH_AT = 12; // a facet with more values than this gets a filter box and an order toggle

export function createRail({ el, tabs, labels, icons, counts, picked, onChange, order = {}, lead = {} }) {
  let tab = tabs[0][0];
  const openFacets = new Set();
  const facetQuery = {};
  const facetAz = {};

  function facet(key) {
    const count = counts(key) || {};
    const all = Object.keys(count);
    if (!all.length) return null;
    const q = (facetQuery[key] || "").trim().toLowerCase();
    const names = q ? all.filter((n) => n.toLowerCase().includes(q)) : all.slice();
    names.sort(facetAz[key] ? (a, b) => a.localeCompare(b) : (a, b) => count[b] - count[a]);
    const fixed = order[key];
    if (fixed) names.sort((a, b) => fixed.indexOf(a) - fixed.indexOf(b));
    const first = lead[key];
    if (first) names.sort((a, b) => (a === first.value ? -1 : b === first.value ? 1 : 0));

    const section = document.createElement("section");
    section.className = "facet";
    const many = all.length > SEARCH_AT;
    section.innerHTML =
      '<header><h2>' + labels[key] + '</h2>' +
      '<button type="button" class="reset"' + (picked[key].size ? '' : ' disabled') + '>' + ICONS.undo + 'Reset</button></header>' +
      (many
        ? '<div class="facet-tools"><label class="field">' + ICONS.search +
          '<input type="search" placeholder="Filter ' + labels[key].toLowerCase() + '" autocomplete="off" aria-label="Filter ' + labels[key].toLowerCase() + '"></label>' +
          '<button type="button" class="square" aria-label="Change order" title="' + (facetAz[key] ? 'Sort by count' : 'Sort A to Z') + '">' + ICONS.sort + '</button></div>'
        : '') +
      '<div class="chips"></div>';

    const chips = section.querySelector(".chips");
    const clamp = names.length > CLAMP_AT && !openFacets.has(key);
    if (clamp) chips.classList.add("clamped");
    for (const name of names) {
      const b = document.createElement("button");
      b.type = "button";
      const isLead = first && name === first.value;
      b.className = "chip" + (isLead && first.className ? " " + first.className : "");
      b.setAttribute("aria-pressed", String(picked[key].has(name)));
      if (isLead && first.title) b.title = first.title;
      b.innerHTML = icons[key] + '<span class="label"></span><span class="n">' + count[name] + '</span>';
      b.querySelector(".label").textContent = name;
      b.addEventListener("click", () => {
        if (picked[key].has(name)) picked[key].delete(name); else picked[key].add(name);
        render();
        onChange();
      });
      chips.appendChild(b);
    }
    if (clamp) {
      const more = document.createElement("button");
      more.type = "button";
      more.className = "more";
      more.textContent = "+" + (names.length - CLAMP_AT) + " more";
      more.addEventListener("click", () => { openFacets.add(key); render(); });
      section.appendChild(more);
    }
    section.querySelector(".reset").addEventListener("click", () => {
      picked[key].clear();
      render();
      onChange();
    });
    const field = section.querySelector(".facet-tools input");
    if (field) {
      field.value = facetQuery[key] || "";
      field.addEventListener("input", (e) => {
        facetQuery[key] = e.target.value;
        openFacets.add(key);
        render();
        const again = el.querySelector('.facet-tools input[data-live="' + key + '"]');
        if (again) { again.focus(); again.setSelectionRange(again.value.length, again.value.length); }
      });
      field.dataset.live = key;
    }
    const toggle = section.querySelector(".facet-tools .square");
    if (toggle) toggle.addEventListener("click", () => { facetAz[key] = !facetAz[key]; render(); });
    return section;
  }

  function render() {
    el.innerHTML = "";
    const row = document.createElement("div");
    row.className = "tabs";
    row.setAttribute("role", "tablist");
    for (const [name, icon] of tabs) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "tab";
      b.setAttribute("role", "tab");
      b.setAttribute("aria-selected", String(name === tab));
      b.innerHTML = icon + "<span>" + name + "</span>";
      b.addEventListener("click", () => { tab = name; render(); });
      row.appendChild(b);
    }
    el.appendChild(row);

    const sections = document.createElement("div");
    sections.className = "sections";
    const keys = (tabs.find((t) => t[0] === tab) || tabs[0])[2];
    for (const key of keys) {
      const s = facet(key);
      if (s) sections.appendChild(s);
    }
    el.appendChild(sections);
  }

  return { render, get tab() { return tab; } };
}
