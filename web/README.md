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
| `public/fonts/` | Self-hosted fonts and their licenses |
| `src/assets/` | Brand SVGs inlined into pages |
| `public/_headers` | Response headers for Workers static assets (`frame-ancestors`, `nosniff`, referrer policy) |
| `public/robots.txt` | Points at the sitemap, which `@astrojs/sitemap` writes at build time |

## Brand

The brand is defined in [`brand/BRAND.md`](../brand/BRAND.md). The site copies what it needs; nothing is read
from `../brand` at runtime.

- Tokens (palette for dark and light, fonts, radius): the `:root` block at the top of `src/styles/global.css`,
  taken from BRAND.md's "Design tokens". The `theme-color` tags in `src/layouts/Base.astro` repeat the two
  background colors.
- Lockup in the top bar: `src/assets/lockup.svg`, inlined so it takes the text color (bone on dark, ink on
  light).
- Icons: `public/favicon.ico`, `public/icon.svg` (from `icon-small.svg`), `public/apple-touch-icon.png` (180 px,
  from `export/icon-512.png`), `public/og-image.png` (from `export/og-image-1200x630.png`).
- Fonts: `public/fonts/`, self-hosted WOFF2 with each family's `OFL.txt`. Instrument Sans is the upstream
  variable font instanced to `wdth` 100 and `wght` 400-600, subset to Latin; JetBrains Mono Regular and Medium,
  subset to Latin. Rebuild them with fonttools:

  ```sh
  U="U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+2000-206F,U+2074,U+20AC,U+2122,U+2190-21FF,U+2212,U+2215,U+FEFF,U+FFFD"
  pyftsubset 'InstrumentSans[wdth,wght].woff2' --unicodes="$U" --flavor=woff2 --layout-features='*' --output-file=is.woff2
  fonttools varLib.instancer is.woff2 wdth=100 wght=400:600 -o public/fonts/InstrumentSans-latin.woff2
  pyftsubset JetBrainsMono-Regular.woff2 --unicodes="$U" --flavor=woff2 --layout-features='*' --output-file=public/fonts/JetBrainsMono-Regular-latin.woff2
  pyftsubset JetBrainsMono-Medium.woff2 --unicodes="$U" --flavor=woff2 --layout-features='*' --output-file=public/fonts/JetBrainsMono-Medium-latin.woff2
  ```

  Sources: `fonts/webfonts/` in [Instrument/instrument-sans](https://github.com/Instrument/instrument-sans) and
  [JetBrains/JetBrainsMono](https://github.com/JetBrains/JetBrainsMono).

## Security headers

Astro writes a Content-Security-Policy `<meta>` tag on every page (`security.csp` in `astro.config.mjs`), with a
hash of the inline script, so no `unsafe-inline` is needed. `frame-ancestors` cannot be set from a `<meta>` tag,
so `public/_headers` sends it as a header.

## Deploy

```sh
npm run deploy
```

`wrangler.jsonc` points `assets.directory` at `./dist` with `not_found_handling: "404-page"`, and serves the site
on the custom domains reapr.dev and www.reapr.dev (both serve the same pages; the canonical URL is
reapr.dev). `workers_dev` is `false`, so there is no `*.workers.dev` URL. The API token needs Workers write
access on the account and access to the `reapr.dev` zone. A custom domain cannot be created on a hostname that
already has a CNAME record.
