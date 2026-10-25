'use strict';

// File de téléchargements yt-dlp : analyse du lien, progression, ajout à la bibliothèque.

const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { run } = require('./tools');

const TAB = '\t';
const NOISE = String.raw`(?i)\s*[\(\[]\s*(official\s+(music\s+)?(video|audio|lyric\s+video)|lyrics?(\s+video)?|audio|video|hd|hq|4k(\s+remaster(ed)?)?|remaster(ed)?(\s+\d{4})?)\s*[\)\]]`;
const na = (v) => (v === undefined || v === 'NA' || v === 'None' || v === '' ? null : v);
const num = (v) => {
  const n = Number(na(v));
  return Number.isFinite(n) ? n : null;
};

/** Titre lisible tiré de l'adresse (« …/sets/city-ports » → « City ports ») quand yt-dlp n'en donne pas. */
function titleFromUrl(url) {
  try {
    const last = decodeURIComponent(new URL(url).pathname.split('/').filter(Boolean).pop() || '');
    const words = last.replace(/[-_]+/g, ' ').trim();
    return words ? words.charAt(0).toUpperCase() + words.slice(1) : null;
  } catch {
    return null;
  }
}

const SEARCH_PREFIX = { youtube: 'ytsearch', soundcloud: 'scsearch' };

function isMix(url) {
  // Les « mix » YouTube (list=RD…) sont infinis : on ne télécharge que la vidéo.
  return /[?&]list=RD/.test(url);
}

class Downloads {
  constructor({ tools, library, store, file, emit }) {
    this.tools = tools;
    this.library = library;
    this.store = store;
    this.file = file;
    this.emit = emit;
    this.jobs = [];
    this.processes = new Map();
    this.saveTimer = null;
    this.seq = Date.now();
    this.#load();
  }

  #load() {
    try {
      const saved = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      this.jobs = (saved.jobs || []).map((job) => (
        ['queued', 'info', 'downloading', 'processing'].includes(job.status)
          ? { ...job, status: 'error', error: 'Interrompu par la fermeture de l’application.', speed: null, eta: null }
          : job));
    } catch { /* aucun historique */ }
  }

  #save() {
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      try {
        fs.mkdirSync(path.dirname(this.file), { recursive: true });
        fs.writeFileSync(this.file, JSON.stringify({ jobs: this.jobs.slice(-80) }));
      } catch { /* non critique */ }
    }, 400);
  }

  list() {
    return this.jobs;
  }

  #update(job, patch) {
    Object.assign(job, patch);
    this.emit('dl:update', job);
    this.#save();
  }

  get downloadDir() {
    return this.store.get('downloadDir', '') || this.defaultDir;
  }

  /** Cookies donnés à yt-dlp : un fichier cookies.txt (exporté du navigateur) passe avant les cookies d'un navigateur. */
  #cookieArgs() {
    const dl = this.store.get('dl', {}) || {};
    if (dl.cookiesFile && fs.existsSync(dl.cookiesFile)) return ['--cookies', dl.cookiesFile];
    if (dl.cookies) return ['--cookies-from-browser', dl.cookies];
    return [];
  }

  // ─── Analyse d'un lien ─────────────────────────────────────────────────────
  async info(url) {
    const ytdlp = this.tools.path('ytdlp');
    if (!ytdlp) throw new Error('yt-dlp n’est pas installé.');
    const args = ['-J', '--flat-playlist', '--no-warnings', '--encoding', 'utf-8', '--ignore-config'];
    if (isMix(url)) args.push('--no-playlist');
    args.push(...this.#cookieArgs());
    args.push('--', url);
    const text = await run(ytdlp, args, { timeout: 90000, env: this.tools.env() });
    const data = JSON.parse(text);
    const thumb = (d) => d.thumbnail || d.thumbnails?.filter((t) => t.url).pop()?.url || null;
    if (data._type === 'playlist') {
      const entries = (data.entries || []).filter(Boolean);
      return {
        type: 'playlist',
        title: data.title || 'Playlist',
        uploader: data.uploader || data.channel || null,
        count: data.playlist_count || entries.length,
        thumbnail: thumb(data) || thumb(entries[0] || {}),
        entries: entries.slice(0, 60).map((e) => ({
          title: e.title || titleFromUrl(e.url || e.webpage_url || ''),
          duration: e.duration || null,
          uploader: e.uploader || e.channel || null,
        })),
        music: entries.some((e) => e.artist) || /Topic$/.test(entries[0]?.uploader || entries[0]?.channel || ''),
      };
    }
    return {
      type: 'video',
      title: data.track || data.title || titleFromUrl(url),
      uploader: data.artist || data.uploader || data.channel || null,
      duration: data.duration || null,
      thumbnail: thumb(data),
      music: !!data.artist || /Topic$/.test(data.uploader || data.channel || ''),
    };
  }

  // ─── Recherche (YouTube, SoundCloud) ───────────────────────────────────────
  async search(source, query, limit = 12) {
    const ytdlp = this.tools.path('ytdlp');
    if (!ytdlp) throw new Error('yt-dlp n’est pas installé.');
    const prefix = SEARCH_PREFIX[source];
    const text = String(query || '').replace(/\s+/g, ' ').trim().slice(0, 200);
    if (!prefix || !text) return [];
    const args = ['-J', '--flat-playlist', '--no-warnings', '--encoding', 'utf-8', '--ignore-config'];
    if (source === 'youtube') args.push(...this.#cookieArgs());
    args.push('--', `${prefix}${Math.max(1, Math.min(25, limit))}:${text}`);
    const data = JSON.parse(await run(ytdlp, args, { timeout: 60000, env: this.tools.env() }));
    return (data.entries || []).filter(Boolean).map((e) => {
      const url = e.webpage_url || e.url;
      return {
        title: e.title || titleFromUrl(url || ''),
        uploader: e.uploader || e.channel || e.artist || null,
        duration: e.duration || null,
        thumbnail: e.thumbnail || e.thumbnails?.filter((t) => t.url).pop()?.url || null,
        url,
        source,
      };
    }).filter((r) => r.url && /^https?:\/\//.test(r.url));
  }

  // ─── File ──────────────────────────────────────────────────────────────────
  add(opts) {
    const job = {
      id: ++this.seq,
      url: opts.url,
      title: opts.title || titleFromUrl(opts.url) || opts.url,
      thumbnail: opts.thumbnail || null,
      isPlaylist: !!opts.isPlaylist,
      kind: opts.kind === 'video' ? 'video' : 'audio',
      format: opts.format || 'mp3',
      quality: opts.quality || '320',
      parseTitle: opts.parseTitle !== false,
      playlistName: opts.createPlaylist ? (opts.playlistName || opts.title || 'Téléchargements') : null,
      status: 'queued',
      progress: 0,
      speed: null,
      eta: null,
      current: null,
      index: null,
      count: opts.count || null,
      files: [],
      trackIds: [],
      error: null,
      createdAt: Date.now(),
    };
    this.jobs.push(job);
    this.emit('dl:update', job);
    this.#save();
    this.#pump();
    return job;
  }

  #pump() {
    const limit = Math.max(1, Math.min(4, Number(this.store.get('dl', {}).concurrency) || 2));
    const running = this.jobs.filter((j) => ['downloading', 'processing'].includes(j.status)).length;
    let free = limit - running;
    for (const job of this.jobs) {
      if (free <= 0) break;
      if (job.status === 'queued') {
        free--;
        this.#run(job);
      }
    }
  }

  cancel(id) {
    const job = this.jobs.find((j) => j.id === id);
    if (!job) return;
    const child = this.processes.get(id);
    if (child) {
      job.canceled = true;
      this.#kill(child);
    } else if (job.status === 'queued') {
      this.#update(job, { status: 'canceled' });
    }
  }

  retry(id) {
    const job = this.jobs.find((j) => j.id === id);
    if (!job || ['queued', 'downloading', 'processing'].includes(job.status)) return;
    this.#update(job, { status: 'queued', progress: 0, error: null, speed: null, eta: null, canceled: false, files: [], trackIds: [] });
    this.#pump();
  }

  clearFinished() {
    this.jobs = this.jobs.filter((j) => ['queued', 'downloading', 'processing'].includes(j.status));
    this.emit('dl:reset', this.jobs);
    this.#save();
  }

  remove(id) {
    const job = this.jobs.find((j) => j.id === id);
    if (!job || this.processes.has(id)) return;
    this.jobs = this.jobs.filter((j) => j.id !== id);
    this.emit('dl:reset', this.jobs);
    this.#save();
  }

  #kill(child) {
    if (process.platform === 'win32') {
      spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true });
    } else {
      child.kill('SIGTERM');
    }
  }

  killAll() {
    for (const child of this.processes.values()) this.#kill(child);
  }

  // ─── Exécution ─────────────────────────────────────────────────────────────
  args(job, outDir) {
    const a = [
      '--ignore-config', '--no-colors', '--newline', '--no-mtime', '--windows-filenames', '--no-warnings',
      '--ignore-errors', '--encoding', 'utf-8', '--progress',
      '-o', path.join(outDir, `${job.isPlaylist ? '%(playlist_title&{}/|)s' : ''}%(title).150B.%(ext)s`),
      '--progress-template', ['download:CTRLPROG', '%(progress.status)s', '%(progress.downloaded_bytes)s', '%(progress.total_bytes)s', '%(progress.total_bytes_estimate)s', '%(progress.speed)s', '%(progress.eta)s', '%(info.playlist_index)s', '%(info.n_entries)s', '%(info.title)s'].join(TAB),
      '--progress-template', ['postprocess:CTRLPOST', '%(progress.status)s', '%(info.title)s'].join(TAB),
      '--print', ['after_move:CTRLFILE', '%(filepath)s', '%(webpage_url)s'].join(TAB),
      '--no-simulate',
      '--embed-metadata', '--embed-thumbnail', '--convert-thumbnails', 'jpg',
      job.isPlaylist ? '--yes-playlist' : '--no-playlist',
    ];
    if (job.kind === 'audio') {
      a.push('-f', 'bestaudio/best', '-x', '--audio-format', job.format);
      if (job.format === 'mp3' && /^\d+$/.test(job.quality)) a.push('--audio-quality', `${job.quality}K`);
      if (job.parseTitle) {
        a.push('--parse-metadata', 'title:%(artist)s - %(title)s');
        // « (Official Video) », « [Lyrics] », « (4K Remaster) » : du bruit dans le titre.
        a.push('--replace-in-metadata', 'title', NOISE, '');
      }
    } else {
      const cap = /^\d+$/.test(job.quality) ? `res:${job.quality},` : '';
      a.push('-S', `${cap}vcodec:h264,acodec:aac`, '--merge-output-format', 'mp4', '--embed-chapters');
    }
    a.push(...this.#cookieArgs());
    const ffmpeg = this.tools.path('ffmpeg');
    if (ffmpeg) a.push('--ffmpeg-location', path.dirname(ffmpeg));
    const deno = this.tools.path('deno');
    if (deno) a.push('--js-runtimes', `deno:${deno}`);
    a.push('--', job.url);
    return a;
  }

  async #run(job) {
    const ytdlp = this.tools.path('ytdlp');
    if (!ytdlp) {
      this.#update(job, { status: 'error', error: 'yt-dlp n’est pas installé : ouvrez l’onglet Téléchargements pour installer les outils.' });
      return;
    }
    const outDir = this.downloadDir;
    fs.mkdirSync(outDir, { recursive: true });
    this.#update(job, { status: 'downloading', progress: 0, error: null, files: [], trackIds: [], canceled: false });

    const child = spawn(ytdlp, this.args(job, outDir), { windowsHide: true, env: this.tools.env() });
    this.processes.set(job.id, child);
    const errors = [];
    const pending = [];
    let buffer = '';
    let lastEmit = 0;

    const handle = (line) => {
      if (line.startsWith('CTRLPROG')) {
        const [, status, got, total, estimate, speed, eta, index, count, title] = line.split(TAB);
        const done = num(got);
        const size = num(total) || num(estimate);
        const itemProgress = size ? Math.min(1, done / size) : 0;
        const idx = num(index);
        const n = num(count) || job.count;
        const overall = idx && n ? ((idx - 1) + itemProgress) / n : itemProgress;
        const now = Date.now();
        if (status === 'finished' || now - lastEmit > 250) {
          lastEmit = now;
          this.#update(job, {
            status: 'downloading',
            progress: Math.max(job.progress, overall),
            speed: num(speed),
            eta: num(eta),
            index: idx,
            count: n,
            current: na(title) || job.current,
          });
        }
      } else if (line.startsWith('CTRLPOST')) {
        const [, , title] = line.split(TAB);
        this.#update(job, { status: 'processing', current: na(title) || job.current, speed: null, eta: null });
      } else if (line.startsWith('CTRLFILE')) {
        const [, file, url] = line.split(TAB);
        if (file) {
          job.files.push(file);
          pending.push(this.library.addPaths([file], { sourceUrl: url || job.url }).then((ids) => {
            job.trackIds.push(...ids);
          }));
          this.#update(job, { files: job.files, status: 'downloading' });
        }
      } else if (/^ERROR:/.test(line)) {
        errors.push(line.replace(/^ERROR:\s*(\[[^\]]+\]\s*)?/, ''));
      }
    };

    const onData = (chunk) => {
      buffer += chunk.toString('utf8');
      let nl;
      while ((nl = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, nl).replace(/\r$/, '');
        buffer = buffer.slice(nl + 1);
        if (line) handle(line);
      }
    };
    child.stdout.on('data', onData);
    child.stderr.on('data', onData);

    const code = await new Promise((resolve) => {
      child.on('error', (err) => {
        errors.push(err.message);
        resolve(-1);
      });
      child.on('close', resolve);
    });
    this.processes.delete(job.id);
    if (buffer.trim()) handle(buffer.trim());
    await Promise.allSettled(pending);

    if (job.canceled) {
      this.#update(job, { status: 'canceled', speed: null, eta: null });
    } else if (job.files.length) {
      let playlistId = null;
      if (job.playlistName && job.trackIds.length) {
        playlistId = this.library.createPlaylist(job.playlistName);
        this.library.addToPlaylist(playlistId, job.trackIds);
      }
      const skipped = code !== 0 ? Math.max(0, (job.count || job.files.length) - job.files.length) : 0;
      this.#update(job, {
        status: 'done',
        progress: 1,
        speed: null,
        eta: null,
        playlistId,
        skipped,
        error: skipped ? (errors.pop() || null) : null,
      });
    } else {
      this.#update(job, { status: 'error', speed: null, eta: null, error: errors.pop() || `yt-dlp s’est arrêté (code ${code}).` });
    }
    this.#pump();
  }
}

module.exports = { Downloads };
