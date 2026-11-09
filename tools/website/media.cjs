// Turns the screenshots from capture.cjs into the website's pictures and demo videos, in docs/media.
// Everything happens in Edge, already on this PC: media.html crops, encodes and hands each file back
// here. Nothing is downloaded or installed. Usage: node tools/website/media.cjs [shots folder]
// --social regenerates only the GitHub card from the existing reviewed hero image.
const http = require('node:http'), fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const project = path.resolve(__dirname, '../..'), output = path.join(project, 'docs/media');
const socialOnly = process.argv.includes('--social');
const shots = path.resolve(process.argv.slice(2).find(arg => arg !== '--social') || path.join(project, 'work/website-shots'));
const edge = 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const types = { '.html': 'text/html; charset=utf-8', '.png': 'image/png', '.mp4': 'video/mp4', '.jpg': 'image/jpeg' };
let browser;

function serve(response, file) {
  if (!fs.existsSync(file)) { response.writeHead(404); response.end('missing'); return; }
  response.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
  fs.createReadStream(file).pipe(response);
}
function finish(code) {
  if (browser) cp.spawnSync('taskkill', ['/T', '/F', '/PID', String(browser.pid)], { windowsHide: true, stdio: 'ignore' });
  process.exit(code);
}

const server = http.createServer((request, response) => {
  const url = decodeURIComponent(request.url.split('?')[0]);
  if (request.method === 'POST') {
    const parts = [];
    request.on('data', part => parts.push(part)).on('end', () => {
      const body = Buffer.concat(parts);
      response.writeHead(200); response.end('ok');
      if (url.startsWith('/save/')) {
        fs.writeFileSync(path.join(output, path.basename(url)), body);
        console.log(`${path.basename(url)} ${(body.length / 1024).toFixed(0)} KB`);
      } else if (url === '/log') console.log(body.toString());
      else if (url === '/fail') { console.error(body.toString()); finish(1); }
      else if (url === '/done') finish(0);
    });
    return;
  }
  if (url === '/media.html') serve(response, path.join(__dirname, 'media.html'));
  else if (url === '/logo.png') serve(response, path.join(project, 'docs/images/rocket-logo.png'));
  else if (url === '/social-source.webp') serve(response, path.join(output, 'hero-dark.webp'));
  else if (url === '/report.json') serve(response, path.join(shots, 'report.json'));
  else if (url.startsWith('/shots/')) serve(response, path.join(shots, path.basename(url)));
  else { response.writeHead(404); response.end('missing'); }
});

server.listen(0, '127.0.0.1', () => {
  fs.mkdirSync(output, { recursive: true });
  fs.mkdirSync(path.join(project, 'work'), { recursive: true });
  const profile = socialOnly ? fs.mkdtempSync(path.join(project, 'work/website-edge-social-')) : path.join(project, 'work/website-edge');
  browser = cp.spawn(edge, ['--headless=new', '--user-data-dir=' + profile, '--no-first-run', '--hide-scrollbars', '--window-size=1920,1100',
    '--force-device-scale-factor=1', `http://127.0.0.1:${server.address().port}/media.html${socialOnly ? '?social=1' : ''}`], { stdio: 'ignore', windowsHide: true });
  setTimeout(() => { console.error('Timed out.'); finish(1); }, 300000);
});
