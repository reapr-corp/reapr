// Renders every brand/social/src/*.html to brand/social/<name>.png at the size in its <html data-size>.
//
//   npm i --prefix /some/tmp playwright-core
//   NODE_PATH=/some/tmp/node_modules node brand/social/render.cjs [--sheet /path/contact-sheet.png]
//
// Fonts are not committed: put InstrumentSans.ttf, JetBrainsMono-Regular.woff2 and
// JetBrainsMono-Medium.woff2 (both OFL) in brand/social/src/fonts/. The script fails if a font does not load.
// CHROME overrides the browser path (default /usr/bin/google-chrome).
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');

const brand = path.resolve(__dirname, '..');
const src = path.join(__dirname, 'src');
const types = { '.html': 'text/html', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png',
  '.ttf': 'font/ttf', '.woff2': 'font/woff2' };

// The templates load ../../lockup.svg as a CSS mask, which needs http, not file://.
const server = http.createServer((req, res) => {
  const file = path.join(brand, decodeURIComponent(new URL(req.url, 'http://x').pathname));
  if (!file.startsWith(brand) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
  fs.createReadStream(file).pipe(res);
});

(async () => {
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome' });
  const page = await browser.newPage();
  const outputs = [];
  for (const name of fs.readdirSync(src).filter((f) => f.endsWith('.html')).sort()) {
    const [w, h] = fs.readFileSync(path.join(src, name), 'utf8').match(/data-size="(\d+)x(\d+)"/).slice(1).map(Number);
    await page.setViewportSize({ width: w, height: h });
    await page.goto(`${base}/social/src/${name}`, { waitUntil: 'networkidle' });
    // Load every face, used or not, so a missing font file fails the run instead of falling back.
    const ok = await page.evaluate(() => Promise.all([...document.fonts].map((f) => f.load())).then((f) => f.length === 3, () => false));
    if (!ok) throw new Error(`${name}: fonts did not load; see the header of render.cjs`);
    const out = path.join(__dirname, name.replace(/\.html$/, '.png'));
    await page.screenshot({ path: out, clip: { x: 0, y: 0, width: w, height: h } });
    outputs.push({ name: path.basename(out), w, h });
    console.log(`${path.basename(out)}  ${w}x${h}`);
  }

  const i = process.argv.indexOf('--sheet');
  if (i > 0) {
    const tiles = outputs.map((o) => `<figure><img src="/social/${o.name}" style="width:${Math.min(o.w, 1600) / 2}px">
      <figcaption>${o.name}</figcaption></figure>`).join('');
    await page.setViewportSize({ width: 1720, height: 800 });
    await page.setContent(`<base href="${base}/"><style>body{margin:0;padding:40px;background:#050605;
      font:16px monospace;color:#8B918A;display:flex;flex-wrap:wrap;gap:32px 40px;align-items:flex-start;
      width:1720px;box-sizing:border-box}figure{margin:0}img{display:block;border:1px solid #262B27}
      figcaption{margin-top:8px}</style>${tiles}`, { waitUntil: 'networkidle' });
    await page.screenshot({ path: process.argv[i + 1], fullPage: true });
    console.log(`sheet: ${process.argv[i + 1]}`);
  }
  await browser.close();
  server.close();
})().catch((e) => { console.error(e); process.exit(1); });
