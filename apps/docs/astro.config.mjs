import { defineConfig } from 'astro/config';

const githubPages = process.env.GITHUB_PAGES === 'true';
const [owner = 'Hologram-Technologies', repository = 'hologram-live'] =
  (process.env.GITHUB_REPOSITORY ?? 'Hologram-Technologies/hologram-live').split('/');

export default defineConfig({
  output: 'static',
  site: githubPages ? `https://${owner.toLowerCase()}.github.io` : undefined,
  base: githubPages ? `/${repository}` : '/',
});
