// Checks the website's files without a browser: every link and anchor, GitHub links against this
// checkout, alt text, image sizes, heading order, nothing loaded from elsewhere, and nothing personal.
// Usage: node tools/website/check.cjs
const fs = require('node:fs'), os = require('node:os'), path = require('node:path');
const project = path.resolve(__dirname, '../..'), docs = path.join(project, 'docs');
const GITHUB = 'https://github.com/JasXTD/rocket', SITE = 'https://jasxtd.github.io/rocket/';
const pages = ['index.html', 'get.html', '404.html'];
const problems = [], seen = { links: 0, images: 0, github: 0 };
const fail = (page, text) => problems.push(`${page}: ${text}`);
const read = page => fs.readFileSync(path.join(docs, page), 'utf8');
const attrs = tag => Object.fromEntries([...tag.matchAll(/([a-zA-Z-]+)(?:="([^"]*)")?/g)].slice(1).map(m => [m[1].toLowerCase(), m[2] ?? '']));
const ids = page => new Set([...read(page).matchAll(/\sid="([^"]+)"/g)].map(m => m[1]));

for (const page of pages) {
  const html = read(page);
  const base = (html.match(/<base href="([^"]+)"/) || [])[1];
  if (!/<html lang="en">/.test(html)) fail(page, 'no lang on <html>');
  if (!/<title>[^<]+<\/title>/.test(html)) fail(page, 'no title');
  if (!/<meta name="viewport"/.test(html)) fail(page, 'no viewport');

  for (const [, name, value] of html.matchAll(/\s(href|src|poster)="([^"]*)"/g)) {
    seen.links++;
    if (/^https?:/.test(value)) {
      if (value === GITHUB || value === GITHUB + '.git') continue;
      if (value.startsWith(GITHUB + '/blob/main/')) {
        seen.github++;
        const target = value.slice((GITHUB + '/blob/main/').length);
        if (!fs.existsSync(path.join(project, target))) fail(page, `GitHub link to a missing file: ${target}`);
        continue;
      }
      if (value.startsWith(SITE) && /<link rel="canonical"/.test(html) && html.includes(`"${value}"`)) {
        const target = value.slice(SITE.length) || 'index.html';
        if (!fs.existsSync(path.join(docs, target))) fail(page, `${name} names a missing file: ${value}`);
        continue;
      }
      fail(page, `leaves the site: ${value}`);
      continue;
    }
    if (value === base) continue;
    if (/^(\/|[a-z]+:)/i.test(value)) { fail(page, `not relative: ${value}`); continue; }
    if (name !== 'href' && !value) { fail(page, `empty ${name}`); continue; }
    const [file, anchor] = value.split('#');
    const target = !file ? page : file === './' ? 'index.html' : file;
    if (!fs.existsSync(path.join(docs, target))) { fail(page, `missing file: ${value}`); continue; }
    if (anchor && target.endsWith('.html') && !ids(target).has(anchor)) fail(page, `missing anchor: ${value}`);
  }

  for (const [tag] of html.matchAll(/<img\b[^>]*>/g)) {
    seen.images++;
    const a = attrs(tag);
    if (a.alt === undefined) fail(page, `image without alt: ${a.src}`);
    if (!a.width || !a.height) fail(page, `image without a size: ${a.src}`);
  }
  for (const [tag] of html.matchAll(/<video\b[^>]*>/g)) {
    const a = attrs(tag);
    if (!('controls' in a) || a.preload !== 'none' || !a.poster || !a['aria-label'] || 'autoplay' in a || 'loop' in a) fail(page, `video must be click-to-play with a poster and a label: ${a.src}`);
  }

  let level = 0;
  for (const [, depth] of html.matchAll(/<h([1-6])\b/g)) {
    if (Number(depth) > level + 1) fail(page, `heading jumps from h${level} to h${depth}`);
    level = Number(depth);
  }
  const unique = [...html.matchAll(/\sid="([^"]+)"/g)].map(m => m[1]);
  if (new Set(unique).size !== unique.length) fail(page, 'duplicate id');
  if (/\sstyle="(?!--z)/.test(html)) fail(page, 'inline style other than a crop');
  if (/\[owner\]|\[repository\]/.test(html)) fail(page, 'placeholder left in');
}

// Text files only: pictures and video are checked by eye.
const text = fs.readdirSync(docs).filter(n => /\.(html|css|js|xml)$/.test(n));
const media = fs.existsSync(path.join(docs, 'media')) ? fs.readdirSync(path.join(docs, 'media')) : [];
const used = pages.map(read).join('\n');
for (const name of media) if (!used.includes('media/' + name)) fail('media', `not used by any page: ${name}`);
const personal = [os.homedir(), os.homedir().replaceAll('\\', '/'), os.userInfo().username, 'rocket-ui-test', '@gmail', 'PRIVATE_'];
for (const name of text) {
  const body = fs.readFileSync(path.join(docs, name), 'utf8');
  for (const word of personal) if (body.includes(word)) fail(name, `names something personal: ${word}`);
  if (/fonts\.googleapis|analytics|gtag|cdn\./i.test(body)) fail(name, 'loads something from elsewhere');
}
if (!fs.existsSync(path.join(docs, '.nojekyll'))) fail('docs', 'no .nojekyll');

let bytes = 0;
for (const name of media) bytes += fs.statSync(path.join(docs, 'media', name)).size;
console.log(`${pages.length} pages, ${seen.links} links, ${seen.github} checked against this checkout, ${seen.images} images, ${media.length} media files, ${(bytes / 1048576).toFixed(1)} MB`);
if (problems.length) { console.log(problems.join('\n')); process.exitCode = 1; } else console.log('No problems found.');
