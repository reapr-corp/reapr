// @ts-check
import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";

export default defineConfig({
  site: "https://reapr.dev",
  output: "static",
  trailingSlash: "never",
  build: {
    format: "file", // telemetry.html, served at /telemetry by Workers static assets
    inlineStylesheets: "never", // one cached stylesheet; keeps CSP style-src 'self'
  },
  markdown: { syntaxHighlight: false }, // no inline style attributes from a highlighter
  // CSP as a <meta> tag with hashes of the inline copy-button script (frame-ancestors is set in public/_headers,
  // because a <meta> CSP cannot carry it).
  security: { csp: { directives: ["default-src 'self'", "base-uri 'none'", "form-action 'none'", "object-src 'none'"] } },
  integrations: [sitemap()],
  vite: { server: { fs: { allow: [".."] } } }, // TELEMETRY.md lives in the repo root
});
