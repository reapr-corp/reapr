# reapr.dev

The public website, built with [Astro](https://astro.build) as a static site and served by Cloudflare Workers
static assets. No external fonts, scripts or trackers, no cookies. The only JavaScript is the copy-button
script in `src/layouts/Base.astro`, which Astro inlines (about 650 bytes); the pages work without it.

Needs Node.js 22.12 or later (Astro 7).

```sh
cd web
npm ci
npm run dev       # http://localhost:4321, live reload
npm run build     # static output in dist/
npm run check     # build, then validate every page with html-validate
npm run preview   # build, then serve dist/ with wrangler, exactly as Cloudflare routes it
npm run deploy    # build, then deploy dist/ with wrangler
```

`npm run preview` is the one to use before deploying: it serves `/telemetry` without `.html`, the 404 page with
status 404 and the `_headers` rules, as production does. `npm run dev` does not apply `_headers`.

## Layout

| Path | What |
|---|---|
| `src/pages/index.astro` | `/`: one component per section, in page order |
| `src/pages/telemetry.astro` | `/telemetry`: renders the repo's [`TELEMETRY.md`](../TELEMETRY.md) at build time, so it never drifts |
| `src/pages/404.astro` | Served with status 404 for every path that matches no file |
| `src/layouts/Base.astro` | `<head>`, top bar, footer, copy-button script |
| `src/components/` | `Hero`, `Measurement`, `Why`, `HowItWorks`, `Output`, `Action`, `Principles`, `Engines`, `OpenCore`, `FinalCta`, plus `Header`, `Footer`, `CopyButton` |
| `src/styles/global.css` | All styles |
| `public/mark.svg` | The placeholder mark: top bar and favicon |
| `public/_headers` | Response headers for Workers static assets (`frame-ancestors`, `noindex` on `workers.dev`) |
| `public/robots.txt` | Points at the sitemap, which `@astrojs/sitemap` writes at build time |

## Brand

The brand is not decided yet. To change it:

- Colors, fonts and radius: the `:root` block at the top of `src/styles/global.css`. Nothing else in the CSS
  holds a color value except the white hover of the primary button. The `theme-color` meta tag in
  `src/layouts/Base.astro` repeats the background color.
- Logo and favicon: replace `public/mark.svg`.
- Wordmark: the text `reapr` in `src/components/Header.astro`.

## Security headers

Astro writes a Content-Security-Policy `<meta>` tag on every page (`security.csp` in `astro.config.mjs`), with a
hash of the inline script, so no `unsafe-inline` is needed. `frame-ancestors` cannot be set from a `<meta>` tag,
so `public/_headers` sends it as a header.

## Deploy

`wrangler.jsonc` points `assets.directory` at `./dist` with `not_found_handling: "404-page"`. Today it deploys
to the `workers.dev` URL only, as a preview. To serve reapr.dev and www.reapr.dev, add the `routes` block that is
commented in `wrangler.jsonc` and run `npm run deploy` again. Wrangler creates the custom domains, their DNS
records and certificates. Neither hostname may already have a CNAME record. The API token needs Workers write
access on the account and access to the `reapr.dev` zone.
