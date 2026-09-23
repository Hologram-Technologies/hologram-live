// The star badge above the hero headline.
//
// A capsule on a 1px line, in the same control language as everything else on the page, with the count behind
// a divider and the star in brand orange. It is the second place the accent is spent on this view, after the
// action's chevron, and the only one above the headline. The accent fills a mark and never sits behind words:
// white on it is 4.14:1, under AA.
//
// The count is baked in by the build (scripts/data.mjs reads it from GitHub's public API) so the badge is
// right on first paint, and refreshed in the viewer's browser by stars() in app.js so it stays honest between
// nightly builds. A build that could not reach GitHub ships the label with no number, and the page fills it in.

import { count, esc, icon } from "./render.mjs";

export function heroBadge({ repo }) {
  if (!repo?.name) return "";
  const n = repo.stars;
  const label = n == null ? `Star ${repo.name} on GitHub` : `Star ${repo.name} on GitHub, ${n} star${n === 1 ? "" : "s"}`;
  return `<a class="land-badge" data-repo="${esc(repo.name)}" href="${esc(repo.url)}" target="_blank" rel="noopener" aria-label="${esc(label)}">
      <span class="lead">${icon.github}<span class="label">Star on GitHub</span></span>
      <span class="tally">${icon.star.replace('class="i"', 'class="i star"')}<b class="n" id="gh-stars">${n == null ? "" : esc(count(n))}</b></span>
    </a>`;
}
