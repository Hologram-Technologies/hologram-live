// The one origin every dialect of the hub answers on. Everything that prints the hub's own address (the hero
// line, agent.md, llms.txt, the OpenAPI servers entry, the run snippets, the star badge's data build) reads it from
// here, so the name lives in one file. HUB_ORIGIN at build time builds the site for another name.
export const ORIGIN = (process.env.HUB_ORIGIN || "https://gethologram.ai").replace(/\/$/, "");
export const HOST = new URL(ORIGIN).host;
