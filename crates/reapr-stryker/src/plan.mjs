// Reapr plan step for StrykerJS. Run with cwd = project dir:
//   node plan.mjs <input.json>
// input.json: { "config": "<absolute config path>", "ranges": [{ "path", "start", "end" }] }
// Prints { "mutate": ["path:start-end", ...], "ranges": [the selected input ranges],
// "thresholds_break": <user config has a numeric thresholds.break>, "warnings": [...] } on stdout.
// `mutate` is glob-escaped for Stryker; `ranges` is the same selection, unescaped.
//
// Mirrors StrykerJS 9 (dist/src/fs/project-reader.js, dist/src/config/file-matcher.js):
// mutate patterns apply in order, a `!` pattern excludes what earlier patterns included, and
// patterns and files are compared as absolute, forward-slash paths resolved against the cwd, with
// the `minimatch` that the installed Stryker itself uses (case-sensitive, `{ dot: false }`).
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

// Import attributes (`with { type: 'json' }`), used here and in the wrapper.
const [major, minor] = process.versions.node.split('.').map(Number);
if (major < 20 || (major === 20 && minor < 10)) {
  throw new Error(`reapr needs Node >= 20.10; this is ${process.version}`);
}

// Only our JSON goes to stdout; anything the user's config prints goes to stderr.
const stdout = process.stdout.write.bind(process.stdout);
process.stdout.write = process.stderr.write.bind(process.stderr);

const { config, ranges } = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const require = createRequire(path.join(process.cwd(), 'package.json'));

const url = pathToFileURL(config).href;
const options =
  path.extname(config).toLowerCase() === '.json'
    ? (await import(url, { with: { type: 'json' } })).default
    : (await import(url)).default;
if (typeof options !== 'object' || options === null) {
  throw new Error(`${config} must export an object as default export, as Stryker requires`);
}

// The installed Stryker. Its schema is not in the package's `exports`, so resolve package.json
// (which is) and read the schema next to it; `minimatch` resolves from there as Stryker's does.
let corePkg;
try {
  corePkg = require.resolve('@stryker-mutator/core/package.json');
} catch (e) {
  throw new Error(`@stryker-mutator/core is not installed in ${process.cwd()} (${e.message})`);
}
let minimatch;
try {
  ({ minimatch } = createRequire(corePkg)('minimatch'));
} catch (e) {
  throw new Error(`cannot load the minimatch that ${corePkg} uses (${e.message})`);
}

// Stryker's own default, from the installed version.
function strykerDefaultMutate() {
  const schema = path.join(path.dirname(corePkg), 'schema', 'stryker-schema.json');
  return JSON.parse(readFileSync(schema, 'utf8')).properties.mutate.default;
}

const patterns = options.mutate ?? strykerDefaultMutate();
const norm = (p) => path.resolve(p).replace(/\\/g, '/');
const globs = patterns.map((p) => (p.startsWith('!') ? [false, norm(p.slice(1))] : [true, norm(p)]));

// ponytail: user patterns that carry their own range (`file:1-9`) never match here; intersect
// ranges if anyone puts one in a config file.
function included(file) {
  const abs = norm(file);
  let mutate = false;
  for (const [positive, glob] of globs) {
    if (minimatch(abs, glob, { dot: false })) mutate = positive;
  }
  return mutate;
}

// Stryker rejects a ranged pattern that minimatch sees as a glob (options-validator.js), and would
// glob-match it anyway. Single-char classes are literal to minimatch and survive path.resolve on
// Windows, unlike backslash escapes. ponytail: `{a,b}` and a leading `!` in file names stay unescaped.
const slashes = (p) => (process.platform === 'win32' ? p.replace(/\\/g, '/') : p);
const literal = (p) => slashes(p).replace(/[*?[\]()]/g, (c) => `[${c}]`);

const selected = ranges.filter((r) => included(r.path));
const mutate = selected.map((r) => `${literal(r.path)}:${r.start}-${r.end}`);

const warnings = [];
if (options.testRunner === 'vitest' && options.disableBail !== true) {
  let major;
  try {
    major = Number.parseInt(require('vitest/package.json').version, 10);
  } catch {
    // vitest not resolvable from the project: Stryker will say so itself.
  }
  if (major >= 4) {
    warnings.push(
      'vitest >= 4 with bail can report killed mutants as survived (observed: 87.8% with bail vs 98.0% with disableBail on the same code); consider disableBail: true',
    );
  }
}

// Stryker sets exit code 1 when the score is under a numeric `thresholds.break`
// (mutation-test-report-helper.js); any other non-zero exit is a failure of the run itself.
const thresholds_break = typeof options.thresholds?.break === 'number';

stdout(JSON.stringify({ mutate, ranges: selected, thresholds_break, warnings }));
