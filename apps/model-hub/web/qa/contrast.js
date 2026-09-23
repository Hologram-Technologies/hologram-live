// In-page WCAG contrast audit for every visible text node, in the current theme.
// Backgrounds are composited through every translucent layer. In Immersive the photo is unknown, so each
// text is checked against both extremes (a pure white and a pure black photo under the scrim); the worst wins.
(() => {
  const root = document.documentElement;
  const parse = (c) => {
    let m = c.match(/rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)/);
    if (m) return [+m[1], +m[2], +m[3], m[4] === undefined ? 1 : +m[4]];
    m = c.match(/color\(srgb ([\d.e-]+) ([\d.e-]+) ([\d.e-]+)(?: \/ ([\d.e-]+))?\)/);
    if (m) return [m[1] * 255, m[2] * 255, m[3] * 255, m[4] === undefined ? 1 : +m[4]];
    return [0, 0, 0, 0];
  };
  const over = (top, under) => { const a = top[3]; return [top[0] * a + under[0] * (1 - a), top[1] * a + under[1] * (1 - a), top[2] * a + under[2] * (1 - a), 1]; };
  const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };

  const immersive = root.dataset.theme === "immersive";
  // The page decides how much ground covers the wallpaper; the landing lays on more than the app does.
  const veil = (parseFloat(getComputedStyle(root).getPropertyValue("--hh-veil")) || 22) / 100;
  const ground = parse(getComputedStyle(root).backgroundColor);
  const scrimColor = parse(getComputedStyle(root).getPropertyValue("--background").trim() ? getComputedStyle(root).backgroundColor : "rgb(0,0,0)");
  const bases = immersive
    ? [[255, 255, 255, 1], [0, 0, 0, 1]].map((photo) => over([scrimColor[0], scrimColor[1], scrimColor[2], veil], photo))
    : [ground[3] ? ground : [255, 255, 255, 1]];

  const layers = (el) => { const out = []; for (let e = el; e && e !== root; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) out.push(c); } return out.reverse(); };
  const failures = [], checked = { count: 0, min: Infinity };
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.textContent.trim() ? 1 : 3) });
  const seen = new Set();
  while (walker.nextNode()) {
    const el = walker.currentNode.parentElement;
    if (seen.has(el)) continue;
    seen.add(el);
    const r = el.getBoundingClientRect(), cs = getComputedStyle(el);
    if (!r.width || !r.height || cs.visibility === "hidden" || el.closest("[hidden], :disabled, .off, [aria-hidden=\"true\"]")) continue;
    const size = parseFloat(cs.fontSize), bold = +cs.fontWeight >= 700;
    const need = size >= 24 || (size >= 18.66 && bold) ? 3 : 4.5;
    const fg = parse(cs.color);
    let worst = Infinity;
    for (const base of bases) {
      const bg = layers(el).reduce((acc, layer) => over(layer, acc), base);
      worst = Math.min(worst, ratio(fg[3] < 1 ? over(fg, bg) : fg, bg));
    }
    checked.count++;
    checked.min = Math.min(checked.min, worst);
    if (worst < need) failures.push(`${(el.className && String(el.className)) || el.tagName} "${el.textContent.trim().slice(0, 24)}" ${worst.toFixed(2)} < ${need}`);
  }
  return { theme: root.dataset.theme, checked: checked.count, lowest: +checked.min.toFixed(2), failures: [...new Set(failures)].slice(0, 25) };
})();
