'use strict';

// Outils externes (yt-dlp, ffmpeg, deno) : téléchargés au premier lancement dans le dossier de données.

const fs = require('node:fs');
const fsp = fs.promises;
const path = require('node:path');
const { once } = require('node:events');
const { spawn } = require('node:child_process');
const { net } = require('electron');

const GH = 'https://github.com';
const ASSETS = {
  win32: {
    ytdlp: `${GH}/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe`,
    ffmpeg: `${GH}/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip`,
    deno: `${GH}/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip`,
  },
  linux: {
    ytdlp: `${GH}/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux`,
    ffmpeg: `${GH}/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linux64-gpl.tar.xz`,
    deno: `${GH}/denoland/deno/releases/latest/download/deno-x86_64-unknown-linux-gnu.zip`,
  },
  darwin: {
    ytdlp: `${GH}/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos`,
    ffmpeg: null,
    deno: `${GH}/denoland/deno/releases/latest/download/deno-${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-apple-darwin.zip`,
  },
};

const exe = (name) => (process.platform === 'win32' ? `${name}.exe` : name);
const NAMES = { ytdlp: 'yt-dlp', ffmpeg: 'ffmpeg', deno: 'deno' };

function which(name) {
  const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
  for (const dir of dirs) {
    const file = path.join(dir, exe(name));
    try {
      if (fs.statSync(file).isFile()) return file;
    } catch { /* suivant */ }
  }
  return null;
}

function run(command, args, { timeout = 60000, env } = {}) {
  return new Promise((resolve, reject) => {
    let out = '';
    let err = '';
    const child = spawn(command, args, { windowsHide: true, env: env || process.env });
    const timer = setTimeout(() => child.kill(), timeout);
    child.stdout.on('data', (d) => { out += d; });
    child.stderr.on('data', (d) => { err += d; });
    child.on('error', (e) => { clearTimeout(timer); reject(e); });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (code === 0) resolve(out.trim());
      else reject(new Error((err || out || `code ${code}`).trim().split('\n').pop()));
    });
  });
}

async function findFile(dir, wanted) {
  for (const entry of await fsp.readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const hit = await findFile(full, wanted);
      if (hit) return hit;
    } else if (entry.name === wanted) return full;
  }
  return null;
}

async function extract(archive, dest) {
  await fsp.mkdir(dest, { recursive: true });
  const attempts = [];
  if (process.platform === 'win32') {
    const systemTar = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe');
    attempts.push([systemTar, ['-xf', archive, '-C', dest]]);
    attempts.push(['powershell.exe', ['-NoProfile', '-Command', `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${dest}' -Force`]]);
  } else {
    attempts.push(['tar', ['-xf', archive, '-C', dest]]);
    attempts.push(['unzip', ['-o', '-q', archive, '-d', dest]]);
  }
  let last;
  for (const [cmd, args] of attempts) {
    try {
      await run(cmd, args, { timeout: 300000 });
      return;
    } catch (err) {
      last = err;
    }
  }
  throw new Error(`Extraction impossible : ${last?.message || 'erreur inconnue'}`);
}

async function download(url, dest, onProgress) {
  const res = await net.fetch(url, { redirect: 'follow' });
  if (!res.ok) throw new Error(`Téléchargement impossible (HTTP ${res.status})`);
  const total = Number(res.headers.get('content-length')) || 0;
  const part = `${dest}.part`;
  await fsp.mkdir(path.dirname(dest), { recursive: true });
  const out = fs.createWriteStream(part);
  let received = 0;
  let last = 0;
  try {
    for await (const chunk of res.body) {
      received += chunk.length;
      if (!out.write(chunk)) await once(out, 'drain');
      const now = Date.now();
      if (now - last > 150) {
        last = now;
        onProgress(received, total);
      }
    }
    out.end();
    await once(out, 'close');
  } catch (err) {
    out.destroy();
    await fsp.rm(part, { force: true });
    throw err;
  }
  onProgress(received, total);
  await fsp.rename(part, dest);
}

class Tools {
  constructor({ binDir, tmpDir, emit }) {
    this.binDir = binDir;
    this.tmpDir = tmpDir;
    this.emit = emit;
    this.installing = null;
    this.versions = {};
    fs.mkdirSync(binDir, { recursive: true });
  }

  path(tool) {
    const local = path.join(this.binDir, exe(NAMES[tool]));
    if (fs.existsSync(local)) return local;
    return which(NAMES[tool]);
  }

  async refreshVersions() {
    const ytdlp = this.path('ytdlp');
    this.versions.ytdlp = ytdlp ? await run(ytdlp, ['--version'], { timeout: 20000 }).catch(() => null) : null;
  }

  status() {
    const item = (tool) => ({ installed: !!this.path(tool), path: this.path(tool), version: this.versions[tool] || null, supported: !!ASSETS[process.platform]?.[tool] });
    return {
      ytdlp: item('ytdlp'),
      ffmpeg: item('ffmpeg'),
      deno: item('deno'),
      installing: !!this.installing,
      ready: !!this.path('ytdlp') && !!this.path('ffmpeg'),
    };
  }

  /** Dossier à ajouter au PATH des processus lancés (deno doit y être trouvé). */
  env() {
    const dirs = [this.binDir];
    const deno = this.path('deno');
    if (deno) dirs.push(path.dirname(deno));
    return { ...process.env, PATH: [...dirs, process.env.PATH || ''].join(path.delimiter), PYTHONUTF8: '1', PYTHONIOENCODING: 'utf-8' };
  }

  install(which = ['ytdlp', 'ffmpeg', 'deno']) {
    if (this.installing) return this.installing;
    this.installing = (async () => {
      const assets = ASSETS[process.platform] || {};
      for (const tool of which) {
        if (this.path(tool)) continue;
        const url = assets[tool];
        if (!url) continue;
        const progress = (phase, received = 0, total = 0) => this.emit('tools:progress', { tool, phase, received, total });
        try {
          progress('download');
          const file = path.join(this.tmpDir, path.basename(new URL(url).pathname));
          await fsp.mkdir(this.tmpDir, { recursive: true });
          await download(url, file, (r, t) => progress('download', r, t));
          if (tool === 'ytdlp') {
            await fsp.copyFile(file, path.join(this.binDir, exe('yt-dlp')));
            if (process.platform !== 'win32') await fsp.chmod(path.join(this.binDir, exe('yt-dlp')), 0o755);
          } else {
            progress('extract');
            const work = path.join(this.tmpDir, `${tool}-x`);
            await fsp.rm(work, { recursive: true, force: true });
            await extract(file, work);
            const wanted = tool === 'ffmpeg' ? ['ffmpeg', 'ffprobe'] : ['deno'];
            for (const name of wanted) {
              const hit = await findFile(work, exe(name));
              if (!hit) throw new Error(`${exe(name)} introuvable dans l’archive`);
              await fsp.copyFile(hit, path.join(this.binDir, exe(name)));
              if (process.platform !== 'win32') await fsp.chmod(path.join(this.binDir, exe(name)), 0o755);
            }
            await fsp.rm(work, { recursive: true, force: true });
          }
          await fsp.rm(file, { force: true });
          progress('done');
        } catch (err) {
          progress('error');
          this.emit('tools:progress', { tool, phase: 'error', message: err.message });
          throw err;
        }
      }
      await this.refreshVersions();
    })().finally(() => {
      this.installing = null;
      this.emit('tools:status', this.status());
    });
    return this.installing;
  }

  /** Mise à jour de yt-dlp (YouTube change souvent : sans ça, ça casse en quelques semaines). */
  async update() {
    const ytdlp = path.join(this.binDir, exe('yt-dlp'));
    if (!fs.existsSync(ytdlp)) return { updated: false, version: this.versions.ytdlp };
    const before = this.versions.ytdlp;
    try {
      await run(ytdlp, ['-U'], { timeout: 120000, env: this.env() });
    } catch (err) {
      return { updated: false, error: err.message, version: before };
    }
    await this.refreshVersions();
    this.emit('tools:status', this.status());
    return { updated: before !== this.versions.ytdlp, version: this.versions.ytdlp };
  }
}

module.exports = { Tools, run, exe };
