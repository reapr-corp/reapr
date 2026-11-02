# reapr.dev

The public website: plain HTML and CSS, a few lines of JavaScript for the copy buttons, no build step, no
external fonts, scripts or trackers, no cookies. It is served by Cloudflare Workers static assets.

| Path | Page |
|---|---|
| `public/index.html` | `/` |
| `public/telemetry.html` | `/telemetry` |
| `public/404.html` | Every path that does not match a file (served with status 404) |
| `public/style.css` | All styles. Brand tokens (colors, fonts, radius) are in the `:root` block at the top. |
| `public/mark.svg` | The placeholder mark, used in the top bar and as the favicon. Replace this one file to change the logo. |
| `public/_headers` | Response headers (CSP, `noindex` on `workers.dev`). Not served as a file. |
| `public/copy.js` | Copy buttons. The page works without it. |

## Preview

```sh
cd web
npx wrangler@latest dev
```

This serves `public/` at http://localhost:8787 with the same routing as production (`/telemetry` without
`.html`, the 404 page with status 404, the `_headers` rules). Any static file server works for a quick look,
but only `wrangler dev` reproduces the routing.

Check the HTML:

```sh
npx html-validate web/public/*.html
```

## Deploy

```sh
cd web
npx wrangler@latest deploy
```

`wrangler.jsonc` holds the account, the assets directory and `not_found_handling: "404-page"`. Today it deploys
to the `workers.dev` URL only, as a preview. To serve reapr.dev and www.reapr.dev, add the `routes` block that
is commented in `wrangler.jsonc` and deploy again. Wrangler then creates the custom domains, their DNS records
and certificates. Neither hostname may already have a CNAME record. The API token needs Workers write access on
the account and access to the `reapr.dev` zone.

## Keeping the telemetry page in sync

`/telemetry` renders [`TELEMETRY.md`](../TELEMETRY.md), which is the source of truth. When that file changes,
regenerate the body of `public/telemetry.html` (everything between the page's intro paragraph and
`</article>`) from it:

```sh
npx marked --gfm TELEMETRY.md | sed '1d'   # drops the H1, which the page has already
```
