// In-page QA audit. Paste into the console (or run through a browser tool) on any Hologram Hub page.
// Returns violations of the layout system: control heights, type sizes, spacing tokens, tag overflow, page overflow.
(() => {
  const px = (v) => Math.round(parseFloat(v) * 100) / 100;
  const visible = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== "hidden"; };
  const out = { heights: {}, fontSizes: {}, badHeights: [], badFonts: [], badSpacing: [], clippedTags: [], pageOverflow: false, alignment: {} };

  const HEIGHTS = new Set([24, 32, 40, 48, 64]);
  for (const el of document.querySelectorAll("button, .chip, .tab, .tag, .field, .pill, .pager a, .pager span, .button, .control")) {
    if (!visible(el)) continue;
    const h = Math.round(el.getBoundingClientRect().height);
    out.heights[h] = (out.heights[h] || 0) + 1;
    if (!HEIGHTS.has(h) && !el.closest(".copy, th, .card, .wall-row") && !el.matches(".copy, .card")) out.badHeights.push(`${el.className || el.tagName} ${h}`);
  }

  const SIZES = new Set([14, 16, 18, 30]);
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.textContent.trim() ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT) });
  while (walker.nextNode()) {
    const el = walker.currentNode.parentElement;
    if (!visible(el)) continue;
    const s = px(getComputedStyle(el).fontSize);
    out.fontSizes[s] = (out.fontSizes[s] || 0) + 1;
    if (!SIZES.has(s)) out.badFonts.push(`${el.className || el.tagName} ${s}`);
  }

  const SPACE = new Set([0, 1, 2, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64]);
  for (const el of document.querySelectorAll(".shell *")) {
    if (!visible(el) || el.closest("svg, .bx")) continue;
    const cs = getComputedStyle(el);
    for (const prop of ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "rowGap", "columnGap"]) {
      const v = px(cs[prop]);
      if (!Number.isNaN(v) && !SPACE.has(v)) out.badSpacing.push(`${el.className || el.tagName} ${prop} ${v}`);
    }
  }

  for (const row of document.querySelectorAll(".card .tags")) {
    const box = row.getBoundingClientRect();
    for (const tag of row.children) {
      const r = tag.getBoundingClientRect();
      const onRow = r.top < box.bottom;
      if (onRow && r.right > box.right + 0.5) out.clippedTags.push(`${row.closest(".card").title}: ${tag.textContent}`);
    }
  }

  out.pageOverflow = document.documentElement.scrollWidth > document.documentElement.clientWidth;
  out.viewport = { inner: innerWidth, client: document.documentElement.clientWidth, scroll: document.documentElement.scrollWidth };
  const left = (s) => { const e = document.querySelector(s); return e && visible(e) ? Math.round(e.getBoundingClientRect().left) : null; };
  out.alignment = { title: left(".head h1"), search: left(".bar .field"), firstCard: left(".grid .card"), filterTabs: left(".tabs"), firstFacet: left(".facet h2"), firstChip: left(".chip") };
  out.badSpacing = [...new Set(out.badSpacing)].slice(0, 20);
  out.badHeights = [...new Set(out.badHeights)].slice(0, 20);
  out.badFonts = [...new Set(out.badFonts)].slice(0, 20);
  return out;
})();
