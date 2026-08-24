# Releasing Hologram

Hologram has two independent product release trains and a separately versioned documentation deployment. A tag publishes exactly one product or documentation version.

## Standalone server

The server version comes from the root `Cargo.toml`. Push a matching `server-v<version>` tag:

```bash
git tag server-v1.0.0
git push origin server-v1.0.0
```

The `release-server` workflow tests the server and publishes standalone `hologram` archives for Linux, macOS, and Windows, plus `SHA256SUMS`. These archives do not contain the desktop application.

## Desktop application

The desktop version must match in:

- `apps/desktop/package.json`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/tauri.conf.json`

Push a matching `desktop-v<version>` tag:

```bash
git tag desktop-v1.0.0
git push origin desktop-v1.0.0
```

The `release-desktop` workflow publishes Tauri installers for Linux, macOS, and Windows. The application bundles the server as a sidecar so it can run locally, but the resulting installers belong only to the desktop release.

The server and desktop version numbers may advance independently. Creating one release tag never publishes or changes the other product's release.

## Documentation website

The documentation version comes from `apps/docs/package.json` and its lockfile. Release the current version with:

```bash
just docs-release
```

Or provide the expected version explicitly:

```bash
just docs-release 1.0.0
```

For the next release, update both files together before committing:

```bash
cd apps/docs
npm version --no-git-tag-version 1.0.1
```

The command requires a clean worktree, validates and builds the GitHub Pages form of the site, creates an annotated `docs-v<version>` tag, and pushes only that tag. The `release-docs` workflow then deploys the tagged site to `https://hologram-technologies.github.io/hologram-live/`.

This repository uses **GitHub Actions** as its Pages build source. Forks must select the same source in GitHub Settings → Pages before their first deployment.

## Local release builds

```bash
just server-build
just desktop-build
```

Validate a tag before pushing it:

```bash
./scripts/check-release-tag.sh server server-v1.0.0
./scripts/check-release-tag.sh desktop desktop-v1.0.0
./scripts/check-release-tag.sh docs docs-v1.0.0
```
