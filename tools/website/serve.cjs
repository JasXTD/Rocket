// Previews the website the way GitHub Pages serves it: under /Rocket/, with 404.html for anything
// missing and byte ranges so video can seek. Loopback only.
// Usage: node tools/website/serve.cjs [port]
const http = require('node:http'), fs = require('node:fs'), path = require('node:path');
const root = path.resolve(__dirname, '../../docs'), port = Number(process.argv[2]) || 8792, prefix = '/Rocket/';
const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript', '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.mp4': 'video/mp4', '.xml': 'application/xml', '.md': 'text/markdown' };

function send(response, status, file, range) {
  const size = fs.statSync(file).size, headers = { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store' };
  const match = range && /^bytes=(\d*)-(\d*)$/.exec(range);
  if (match && status === 200) {
    const start = match[1] ? Number(match[1]) : Math.max(0, size - Number(match[2]));
    const end = match[1] && match[2] ? Math.min(Number(match[2]), size - 1) : size - 1;
    response.writeHead(206, { ...headers, 'Content-Range': `bytes ${start}-${end}/${size}`, 'Content-Length': end - start + 1 });
    fs.createReadStream(file, { start, end }).pipe(response);
    return;
  }
  response.writeHead(status, { ...headers, 'Content-Length': size });
  fs.createReadStream(file).pipe(response);
}

http.createServer((request, response) => {
  let url;
  try { url = decodeURIComponent(request.url.split(/[?#]/)[0]); }
  catch { response.writeHead(400); response.end(); return; }
  if (url === '/Rocket') { response.writeHead(301, { Location: prefix }); response.end(); return; }
  let file = null;
  if (url.startsWith(prefix)) {
    file = path.resolve(root, url.slice(prefix.length));
    if (file.endsWith(path.sep) || file === root) file = path.join(file, 'index.html');
  }
  if (file && file.startsWith(root + path.sep) && fs.existsSync(file) && fs.statSync(file).isFile()) send(response, 200, file, request.headers.range);
  else send(response, 404, path.join(root, '404.html'));
}).listen(port, '127.0.0.1', () => console.log(`http://127.0.0.1:${port}${prefix}`));
