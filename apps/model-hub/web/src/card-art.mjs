// The mesh behind every catalogue card — Models, Registry, Spaces — and the ONE place it is defined.
//
// It is a faceted surface seeded by the entry's own address: the same bytes draw the same surface, on
// every page and on every visit. It carries no state and marks nothing; the tag chips on the card say
// what the entry is. (It used to have a dim "unlit" variant, which made the Registry grid read as a
// weaker page than Models rather than as the same page with different rows.)
//
// The geometry is FIXED in CSS pixels, which is the whole point: `.art` is 260 x 200 px (chrome.css)
// and the viewBox is 260 x 200, so one unit is one pixel on every page whatever the card's height or
// the column count. A card shorter than 200px simply clips the bottom. Change either the box or the
// viewBox and you must change both, or the three pages stop matching.
export const ART_W = 260, ART_H = 200;
const COLS = 9, ROWS = 6; // 32.5 x 40 px cells

export function art(seed) {
  let h = 2166136261;
  for (const c of String(seed)) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  const r = () => { h ^= h << 13; h ^= h >>> 17; h ^= h << 5; return ((h >>> 0) % 100000) / 100000; };
  const f = (n) => n.toFixed(1);
  const gx = ART_W / (COLS - 1), gy = ART_H / (ROWS - 1), p = [];
  for (let y = 0; y < ROWS; y++) for (let x = 0; x < COLS; x++) {
    p.push([x * gx + (r() - 0.5) * gx * 0.5, y * gy + (y && y < ROWS - 1 ? (r() - 0.5) * gy * 0.5 : 0)]);
  }
  let edges = "", faces = "";
  for (let y = 0; y < ROWS - 1; y++) for (let x = 0; x < COLS - 1; x++) {
    const a = p[y * COLS + x], b = p[y * COLS + x + 1], c = p[(y + 1) * COLS + x], d = p[(y + 1) * COLS + x + 1];
    for (const t of [[a, b, d], [a, d, c]]) {
      const path = `M${t.map((q) => `${f(q[0])} ${f(q[1])}`).join("L")}Z`;
      edges += path;
      const v = r();
      if (v < 0.35) faces += `<path d="${path}" opacity="${f(0.02 + v * 0.12)}"/>`;
    }
  }
  return `<svg class="art" viewBox="0 0 ${ART_W} ${ART_H}" preserveAspectRatio="xMaxYMin slice" aria-hidden="true"><g class="facets">${faces}</g><path class="edges" d="${edges}"/></svg>`;
}
