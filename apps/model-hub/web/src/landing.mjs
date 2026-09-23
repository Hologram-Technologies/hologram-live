// The landing page: one hero, one screen, no scroll.
//
// The figure is the hub's own signature, not decoration: the braille address cells a model page draws for its
// manifest, scaled up into a field and masked away from the type. It is neutral on purpose. The brand accent is
// spent twice on this page and nowhere else: the star in the badge above the headline, and the chevron inside
// the primary action. It fills a mark both times and never sits behind words.

import { esc, icon } from "./render.mjs";
import { heroBadge } from "./hero-badge.mjs";

// Deterministic: the same field every build, so a rebuild is a no-op in the diff and in the cache.
const seeded = (seed) => () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);

const W = 1600, H = 900;

// One braille cell is two columns of four dots. Cells sit on a coarse pitch, most of them empty, so the field
// reads as an address rather than as a texture.
function cells() {
  const rand = seeded(0x484f4c4f);
  const PITCH_X = 58, PITCH_Y = 78, DOT_X = 17, DOT_Y = 18;
  const out = [];
  for (let x = -PITCH_X; x < W + PITCH_X; x += PITCH_X) {
    for (let y = -PITCH_Y; y < H + PITCH_Y; y += PITCH_Y) {
      const live = rand();
      if (live > 0.7) continue;
      const o = (0.28 + live).toFixed(2);
      for (let i = 0; i < 8; i++) {
        if (rand() > 0.5) continue;
        out.push(`<circle cx="${x + (i > 3 ? DOT_X : 0)}" cy="${y + (i % 4) * DOT_Y}" r="2.4" opacity="${o}"/>`);
      }
    }
  }
  return out.join("");
}

// Two planes seen edge on at each margin. Hairlines only: they hold the width of the page without filling it.
const planes = [
  "M64 128 L268 196 L268 712 L64 780 Z",
  "M-48 262 L156 312 L156 628 L-48 678 Z",
  `M${W - 64} 128 L${W - 268} 196 L${W - 268} 712 L${W - 64} 780 Z`,
  `M${W + 48} 262 L${W - 156} 312 L${W - 156} 628 L${W + 48} 678 Z`,
];

// The mask keeps the figure off the type: black hides, white shows, so the centre is empty and the edges carry it.
export const art = () => `<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="xMidYMid slice" aria-hidden="true" focusable="false">
  <defs>
    <radialGradient id="land-fade" cx="50%" cy="50%" r="60%">
      <stop offset="0%" stop-color="black"/>
      <stop offset="42%" stop-color="black"/>
      <stop offset="100%" stop-color="white"/>
    </radialGradient>
    <mask id="land-mask"><rect width="${W}" height="${H}" fill="url(#land-fade)"/></mask>
  </defs>
  <g mask="url(#land-mask)" fill="currentColor">
    <g class="land-dots">${cells()}</g>
    <g class="land-planes" fill="none" stroke="currentColor" stroke-width="1" opacity="0.8">${planes.map((d) => `<path d="${d}"/>`).join("")}</g>
  </g>
</svg>`;

// The strip along the bottom edge: the open model families this index actually carries, in the order a reader
// recognises them. Each one is checked against the catalogue before it is drawn, so the strip cannot claim a
// family the hub does not hold, and the marks are the same avatars the model cards already show.
const FAMILIES = [
  ["meta-llama", "Llama"], ["Qwen", "Qwen"], ["deepseek-ai", "DeepSeek"], ["mistralai", "Mistral AI"],
  ["google", "Gemma"], ["microsoft", "Phi"], ["openai", "Whisper"], ["nvidia", "NVIDIA"],
  ["stabilityai", "Stability AI"], ["black-forest-labs", "FLUX"], ["BAAI", "BGE"],
  ["CohereLabs", "Cohere"], ["sentence-transformers", "Sentence Transformers"],
];

function marquee(base, models) {
  const avatars = new Map();
  for (const m of models) if (m.avatar && !avatars.has(m.org)) avatars.set(m.org, m.avatar);
  const present = FAMILIES.filter(([org]) => avatars.has(org));
  // An index without any of them, or a build whose avatar fetches all failed, gets no strip rather than a
  // band with nothing in it.
  if (!present.length) return "";
  const items = present.map(([org, label]) => `<li><img src="${base}avatars/${esc(avatars.get(org))}" alt="" width="36" height="36" loading="lazy" decoding="async"><span>${esc(label)}</span></li>`).join("");
  // Two identical runs: the track slides exactly half its width, so the loop has no seam.
  return `<div class="land-marquee" aria-hidden="true"><div class="land-track"><ul>${items}</ul><ul>${items}</ul></div></div>`;
}

// Rounded down to the hundred and marked open, so the label reads as a size rather than a tally and never
// says something like "487+". Under a hundred it just says the number.
const count = (n) => (n < 100 ? `${n}` : `${Math.floor(n / 100) * 100}+`);

export function landing({ base, models, endpoint, repo }) {
  return `<main class="land" id="land">
  <div class="land-art" aria-hidden="true">${art()}</div>
  <div class="land-copy">
    ${heroBadge({ repo })}
    <h1 class="land-title"><span>The Open Platform</span><span>for Sovereign AI</span></h1>
    <p class="land-sub">Discover, use and share self-verifying models, skills and artifacts.</p>
    <div class="land-actions">
      <a class="button primary" href="${base}models/">Browse ${count(models.length)} models${icon.right}</a>
      <button type="button" class="land-second copy" data-copy="HF_ENDPOINT=${esc(endpoint)}" title="Copy HF_ENDPOINT=${esc(endpoint)}" aria-label="Copy the endpoint">${esc(endpoint.replace(/^https:\/\//, ""))}${icon.copy}</button>
    </div>
  </div>
</main>
${marquee(base, models)}`;
}
