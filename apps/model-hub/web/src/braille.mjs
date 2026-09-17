// Braille drawn as Hologram dots. One cell is eight dots, exactly one byte, so a 32 byte address is 32 cells.
// Shared by the static build (markup) and the browser (motion).
//
// Loader frames: unicode braille animations by Ledi Hildawan, MIT license
// https://github.com/ledihildawan/unicode-braille-animations @ 28b7b88 (scan slowed for a calm, waiting state).

// Unicode braille bit order to dot position (column, row).
const DOTS = [[0, 0], [0, 1], [0, 2], [1, 0], [1, 1], [1, 2], [0, 3], [1, 3]];

export const LOADERS = {
  scan: { ms: 1400, steps: [[0, "⣿⠀⠀⠀"], [16.6, "⢸⡇⠀⠀"], [33.3, "⠀⣿⠀⠀"], [50, "⠀⢸⡇⠀"], [66.6, "⠀⠀⣿⠀"], [83.3, "⠀⠀⢸⡇"], [100, "⠀⠀⠀⣿"]] },
  orbit: { ms: 600, steps: [[0, "⠉"], [12.5, "⠘"], [25, "⠰"], [37.5, "⢠"], [50, "⣀"], [62.5, "⡄"], [75, "⠆"], [87.5, "⠃"], [100, "⠉"]] },
};

const cell = (byte) => `<svg class="cell" viewBox="0 0 7 13" aria-hidden="true">${DOTS.map(([c, r], b) => `<circle cx="${1.5 + c * 4}" cy="${(1.5 + r * 3.33).toFixed(2)}" r="1.3"${(byte >> b) & 1 ? "" : ' class="off"'}/>`).join("")}</svg>`;

export const textToBytes = (text) => [...text].map((ch) => ch.codePointAt(0) - 0x2800);
export const hexToBytes = (hex) => hex.match(/../g).map((h) => parseInt(h, 16));

/** Static markup: one svg per byte. */
export const cells = (bytes) => bytes.map(cell).join("");

/** A loader placeholder, drawn at its middle frame so it reads correctly before any script runs. */
export function loader(key) {
  const l = LOADERS[key];
  return `<span class="bx loader" data-loader="${key}" aria-hidden="true">${cells(textToBytes(l.steps[Math.floor(l.steps.length / 2)][1]))}</span>`;
}

/** Set one cell's dots to a byte, optionally with a state class (ok, bad, lock). */
export function setCell(svg, byte, state) {
  const dots = svg.children;
  for (let b = 0; b < 8; b++) dots[b].classList.toggle("off", !((byte >> b) & 1));
  if (state !== undefined) svg.setAttribute("class", `cell${state ? ` ${state}` : ""}`);
}

// One shared clock drives every loader on the page; loaders removed from the DOM simply drop out.
const playing = new Set();
let clock = 0;
function tick(now) {
  for (const el of playing) {
    if (!el.isConnected) { playing.delete(el); continue; }
    const l = LOADERS[el.dataset.loader], pct = ((now % l.ms) / l.ms) * 100;
    let frame = l.steps[0][1];
    for (const [p, text] of l.steps) if (p <= pct) frame = text;
    if (el._frame === frame) continue;
    el._frame = frame;
    const bytes = textToBytes(frame), svgs = el.children;
    for (let i = 0; i < svgs.length; i++) setCell(svgs[i], bytes[i] || 0);
  }
  clock = playing.size ? requestAnimationFrame(tick) : 0;
}

/** Start every loader inside root. Reduced motion keeps the static middle frame. */
export function play(root = document) {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  for (const el of root.querySelectorAll(".bx.loader")) playing.add(el);
  if (!clock && playing.size) clock = requestAnimationFrame(tick);
}
