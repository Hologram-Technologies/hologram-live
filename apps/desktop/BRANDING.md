# Branding: wired to the Hologram brand kit

The desktop app's visual system derives from
[Hologram-Technologies/hologram-brand-kit][kit]. The kit's DTCG tokens are the
single source of truth for colour, type, and radius; this app maps its own
semantics onto them and restates no brand decision of its own.

[kit]: https://github.com/Hologram-Technologies/hologram-brand-kit

## Update the brand everywhere (three steps)

1. Change tokens in the kit (`brand/tokens/hologram-tokens.json`; its pipeline
   regenerates `brand/css/hologram-warm.css`).
2. Here: bump `KIT_REV` in `scripts/sync-brand.mjs` to the new kit commit on
   its `develop` branch, then `npm run sync:brand`.
3. Review the diff of `src/brand/` and `icon.svg`, run `npx tauri icon icon.svg`
   if the icon changed, commit.

Every view, both themes, and the application icon follow.

## Which kit branch and which theme layer

- The kit's default branch is **`develop`**. Its `main` is the untouched Penpot
  mirror and carries no `brand/` directory. `KIT_REV` must be a `develop` commit.
- The kit ships two generated theme files. `hologram-theme.css` is the neutral
  shadcn baseline. `hologram-warm.css` is "the Hologram brand layer: warm paper
  (`:root`) and warm dark (`.dark`), accent `#e93b01`" (its own header), added by
  the kit commit "brand: Hologram brand direction, warm ground and one accent".
  This app consumes the **warm layer**. The same commit names Archivo as the
  display face, so `h1` uses `--holo-font-display`.

## Architecture

```
kit tokens (DTCG)  ->  kit hologram-warm.css  ->  scripts/sync-brand.mjs (pinned KIT_REV)
                                                     |  namespaces --x  ->  --holo-x
                                                     |  rescopes :root (light) -> :root[data-theme="light"]
                                                     |           .dark         -> :root   (app is dark by default)
                                                     |  hoists radius / font / tracking tokens into a base :root
                                                     |  composes icon.svg (white logomark on the dark ground)
                                                     v
              src/brand/{theme.css, fonts.css, fonts/, tokens.json, logomark-*.svg, wordmark-*.svg}
              icon.svg  ->  npx tauri icon  ->  src-tauri/icons/*
                                                     v
              src/styles.css :root blocks - APP SEMANTICS
              (--canvas, --card, --tag-*, ...) = var(--holo-*, pre-kit literal)
                                                     v
              components (unchanged: they already consumed app semantics)
```

- Kit properties are namespaced `--holo-*` because the app's semantic layer uses
  colliding names (`--card`, `--border`, `--primary`).
- Every mapping keeps the pre-kit literal as the `var()` fallback: a missing
  token degrades to the previous look, never to broken UI.
- Derived shades (hover, chip fills, borders from accents) are `color-mix()`
  over kit tokens, so they stay single-source.
- The light block in `styles.css` now holds only app extensions; every kit-backed
  semantic flips automatically because `--holo-*` flips in `theme.css`.
- Fonts are self-hosted from the kit's woff2 files (`src/brand/fonts.css`,
  `font-display: swap`): Geist (sans), Geist Mono (mono), Archivo (display).
- Marks: the sidebar lockup is the kit logomark plus the kit wordmark as two
  images, so the wordmark can hide when the rail collapses while the mark stays.
  The console service row and the chat welcome use the logomark. `applyTheme` in
  `src/main.ts` swaps white/black variants with the theme and sets
  `<meta name="theme-color">` from the computed `--canvas`.
- The `data-theme` toggle, its persisted preference, and the `--text-scale`
  accessibility control are untouched.

## Semantic to token mapping

Dark shown; light flips through the same expressions.

| app semantic | derives from |
|---|---|
| `--canvas`, `--workspace` | `--holo-background` |
| `--topbar` | `--holo-background` at 95% |
| `--sidebar` / `--nav-hover` | `--holo-sidebar` / `--holo-sidebar-accent` |
| `--nav-active` | `--holo-sidebar-accent` mixed 12% toward `--holo-sidebar-foreground` |
| `--nav-text` | `--holo-sidebar-foreground` mixed 28% toward `--holo-sidebar` |
| `--card`, `--card-hover` | `--holo-card` (+6% foreground on hover) |
| `--soft`, `--soft-hover`, `--badge`, `--loading`, `--loading-highlight` | `--holo-secondary` (+ mixes) |
| `--text-strong` / `--text` | `--holo-foreground` / 94% toward background |
| `--muted-strong` / `--muted` / `--code` | `--holo-muted-foreground` / 74% / 80% toward background |
| `--border`, `--border-soft`, `--border-strong` | `--holo-border` (65% alpha, +10% foreground) |
| `--primary`, `--on-primary`, `--primary-hover` | `--holo-primary`, `--holo-primary-foreground`, primary 88% toward background |
| `--focus`, `--accent-idle` | `--holo-ring` |
| `--accent-blue`, `--tag-text`, `--tag-bg` | `--holo-brand` (text mixed 28% toward foreground for AA; fill at 12%) |
| `--danger-*`, `--error-*` | `--holo-destructive` (+ mixes) |
| `--mono`, body font, `h1` font and tracking | `--holo-font-mono`, `--holo-font-sans`, `--holo-font-display`, `--holo-tracking-display` |

## Gap list (app extensions, no kit counterpart yet)

| gap | current handling | upstream status |
|---|---|---|
| success / ready green (`--accent`, `--success-*`, `--module-*`) | literal in the "App extensions" block, both themes | kit PR #1 proposes `--success`; open |
| `--shadow` | literal, both themes | none |
| radius scale (kit has one `--radius`) | component radii left literal (4 to 14 px) | none |
| light `--holo-destructive` `#e7000b` on paper `#f3f3ee` | kept as the kit value; contrast 4.29:1, below AA 4.5:1 for small text | to raise in the kit |

## Verification record (kit rev `bce3bdf59de1`)

- `npm run sync:brand` twice: byte-identical `src/brand/` and `icon.svg`.
- `npx tsc --noEmit` clean. `npx vite build` clean (fonts and marks emitted as hashed assets).
- No raw hex in the `:root` blocks except `var()` fallbacks and the marked app-extensions block.
- Headless render confirmed `document.fonts` loaded Geist 400/600/700, Geist Mono 400/500, Archivo 700; body font resolves to Geist; `--canvas` resolves to `#151312` dark and `#f3f3ee` light.
- WCAG contrast over the mapping (computed from the token values, `color-mix` emulated in sRGB):

  | pair | dark | light |
  |---|---|---|
  | text on canvas | 15.06 | 10.56 |
  | text on card | 13.80 | 11.76 |
  | muted-strong on card | 6.83 | 5.68 |
  | muted on card (secondary, AA-large 3:1) | 4.22 | 3.40 |
  | nav-text on sidebar | 8.60 | 5.11 |
  | tag-text on tag-bg | 5.54 | 4.56 |
  | danger-text on canvas | 6.41 | **4.29** |
  | error-text on error-bg | 7.04 | 4.99 |
  | on-primary on primary | 13.51 | 12.56 |

  Everything meets AA except light `danger-text`, a kit value recorded in the gap list.
- Screenshots of Console, Chat, Files, Applications, Modules, collapsed rail, and command palette in both themes before and after are attached to the pull request that introduced this wiring.
