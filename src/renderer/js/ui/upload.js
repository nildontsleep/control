// Envoi de musique depuis le navigateur (version web) : choix de fichiers ou d'un dossier, glisser-déposer (dossiers
// compris), progression de chaque envoi dans une petite carte en bas à droite.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { toast } from './feedback.js';

const MEDIA = /\.(mp3|flac|m4a|m4b|aac|ogg|oga|opus|wav|mp4|m4v|mov|mkv|webm)$/i;
const PARALLEL = 2;

let card = null;
let queue = [];
let running = 0;
let stats = { total: 0, done: 0, failed: 0 };
let hideTimer = null;

const caps = () => window.control.web?.caps || {};

function ensureCard() {
  if (card) return card;
  const list = h('div.up-list');
  const bar = h('i');
  const title = h('strong');
  const close = h('button.icon-btn', { type: 'button', html: icon('x', 14, 2), onclick: () => { card.el.remove(); card = null; queue = []; } });
  const el = h('div.up-card', {}, h('div.up-head', {}, title, close), h('div.up-bar', {}, bar), list);
  document.getElementById('layer').append(el);
  card = { el, list, bar, title };
  return card;
}

function paintCard() {
  if (!card) return;
  const finished = stats.done + stats.failed;
  card.title.textContent = finished === stats.total ? t('up.done', { n: stats.done }) : t('up.title', { n: stats.total - finished });
  card.bar.style.width = `${stats.total ? Math.round((finished / stats.total) * 100) : 0}%`;
}

function send(job) {
  const row = h('div.up-row', {}, h('span.up-name', { text: job.name }), h('span.up-state', { text: '0 %' }));
  ensureCard().list.prepend(row);
  while (card.list.children.length > 6) card.list.lastChild.remove();
  const state = row.querySelector('.up-state');
  const fail = (message) => {
    stats.failed++;
    row.classList.add('failed');
    state.textContent = message;
  };
  return new Promise((resolve) => {
    const xhr = new XMLHttpRequest();
    const query = `name=${encodeURIComponent(job.file.name)}${job.dir ? `&dir=${encodeURIComponent(job.dir)}` : ''}`;
    xhr.open('POST', `/api/upload?${query}`);
    xhr.setRequestHeader('x-control', '1');
    xhr.setRequestHeader('content-type', 'application/octet-stream');
    xhr.upload.onprogress = (e) => { if (e.lengthComputable) state.textContent = `${Math.round((e.loaded / e.total) * 100)} %`; };
    xhr.onerror = () => { fail(t('up.failed')); resolve(); };
    xhr.onload = () => {
      let body = null;
      try { body = JSON.parse(xhr.responseText); } catch { /* réponse illisible */ }
      if (xhr.status === 200 && body?.ok) {
        stats.done++;
        row.classList.add('ok');
        state.innerHTML = icon('check', 14, 2.4);
      } else fail(body?.error?.message || `${t('up.failed')} (${xhr.status})`);
      resolve();
    };
    xhr.send(job.file);
  });
}

async function pump() {
  while (running < PARALLEL && queue.length) {
    const job = queue.shift();
    running++;
    send(job).then(() => {
      running--;
      paintCard();
      if (queue.length) pump();
      else if (!running) {
        clearTimeout(hideTimer);
        // Tout est parti : la carte disparaît d'elle-même, sauf s'il y a eu des échecs à lire.
        if (!stats.failed) hideTimer = setTimeout(() => { card?.el.remove(); card = null; }, 5000);
        if (stats.done) toast(t('up.done', { n: stats.done }), 'music-2');
        stats = { total: 0, done: 0, failed: 0 };
      }
    });
  }
  paintCard();
}

/** Envoie des fichiers ({ file, dir? }[] ou File[]) : ceux qui ne sont ni audio ni vidéo, ou trop gros, sont écartés. */
export function uploadFiles(items) {
  if (!caps().upload) {
    toast(t('up.denied'), 'triangle-alert');
    return;
  }
  const max = (caps().uploadMaxMb || 500) * 1048576;
  let skipped = 0;
  for (const item of items) {
    const file = item.file || item;
    const dir = item.dir || (file.webkitRelativePath ? file.webkitRelativePath.split('/').slice(0, -1).join('/') : '');
    if (!MEDIA.test(file.name)) { skipped++; continue; }
    if (file.size > max) { toast(t('up.tooBig', { name: file.name, mb: caps().uploadMaxMb || 500 }), 'triangle-alert'); continue; }
    queue.push({ file, dir, name: file.name });
    stats.total++;
  }
  if (skipped) toast(t('up.skipped', { n: skipped }), 'info');
  if (stats.total) {
    clearTimeout(hideTimer);
    pump();
  }
}

/** Ouvre le sélecteur du navigateur (fichiers, ou un dossier entier) puis envoie. */
export function pickAndUpload({ directory = false } = {}) {
  const input = Object.assign(document.createElement('input'), { type: 'file', multiple: true, accept: directory ? '' : 'audio/*,video/mp4,video/webm,.flac,.m4a,.opus,.mp3,.wav,.ogg,.aac' });
  if (directory) input.webkitdirectory = true;
  input.style.display = 'none';
  input.addEventListener('change', () => { uploadFiles([...input.files]); input.remove(); });
  input.addEventListener('cancel', () => input.remove());
  document.body.append(input);
  input.click();
}

/** Les fichiers d'un glisser-déposer, dossiers parcourus : [{ file, dir }]. */
export async function filesFromDrop(dataTransfer) {
  const out = [];
  const walk = async (entry, dir) => {
    if (entry.isFile) {
      const file = await new Promise((resolve) => entry.file(resolve, () => resolve(null)));
      if (file) out.push({ file, dir });
    } else if (entry.isDirectory) {
      const reader = entry.createReader();
      for (;;) {
        // eslint-disable-next-line no-await-in-loop
        const batch = await new Promise((resolve) => reader.readEntries(resolve, () => resolve([])));
        if (!batch.length) break;
        // eslint-disable-next-line no-await-in-loop
        for (const child of batch) await walk(child, dir ? `${dir}/${entry.name}` : entry.name);
      }
    }
  };
  const entries = [...dataTransfer.items].map((item) => item.webkitGetAsEntry?.()).filter(Boolean);
  if (entries.length) for (const entry of entries) await walk(entry, ''); // eslint-disable-line no-await-in-loop
  else out.push(...[...dataTransfer.files].map((file) => ({ file, dir: '' })));
  return out;
}
