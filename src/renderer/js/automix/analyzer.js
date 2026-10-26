// AutoMix : analyse des morceaux en arrière-plan (décodage léger + mesure du tempo), avec un cache sur disque.

import { ANALYSIS_RATE, analyzeSamples } from './analysis.js';
import { mediaUrl } from '../urls.js';

const KEY = window.control?.web ? `control.automix.${window.control.web.user.id}` : 'control.automix';
const VERSION = 2; // à incrémenter quand l'analyse change : les résultats en cache sont alors refaits
const MAX_DURATION = 20 * 60; // au-delà (mix de DJ, livres audio), pas la peine
const MAX_BYTES = 160e6;

const memo = new Map(); // id → { sig, a } (a = null : analyse impossible, on n'insiste pas)
const pending = new Map(); // id → Promise
let chain = Promise.resolve();
let saveTimer = null;
let disk = null;

const sigOf = (track) => `${track.size || 0}:${Math.round(track.duration || 0)}`;

function readDisk() {
  if (disk) return disk;
  disk = {};
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) || 'null');
    if (raw && raw.v === VERSION && raw.items) disk = raw.items;
  } catch { /* cache absent ou illisible : on repart de zéro */ }
  return disk;
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ v: VERSION, items: readDisk() }));
    } catch { /* stockage plein ou indisponible */ }
  }, 2500);
}

/** Analyse déjà connue (mémoire ou disque) ; undefined si elle n'a pas encore été faite, null si elle a échoué. */
export function peekAnalysis(track) {
  if (!track) return undefined;
  const sig = sigOf(track);
  const m = memo.get(track.id);
  if (m && m.sig === sig) return m.a;
  const d = readDisk()[track.id];
  if (d && d.s === sig) {
    memo.set(track.id, { sig, a: d.a });
    return d.a;
  }
  return undefined;
}

function remember(track, a) {
  const sig = sigOf(track);
  memo.set(track.id, { sig, a });
  if (a) {
    readDisk()[track.id] = { s: sig, a };
    scheduleSave();
  }
}

async function decodeMono(track) {
  const res = await fetch(mediaUrl(track.id));
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const length = Number(res.headers.get('content-length')) || 0;
  if (length > MAX_BYTES) {
    res.body?.cancel();
    throw new Error('fichier trop gros');
  }
  const data = await res.arrayBuffer();
  const ctx = new OfflineAudioContext(1, 1, ANALYSIS_RATE);
  const audio = await ctx.decodeAudioData(data);
  const mono = new Float32Array(audio.length);
  const channels = audio.numberOfChannels;
  for (let c = 0; c < channels; c++) {
    const ch = audio.getChannelData(c);
    for (let i = 0; i < mono.length; i++) mono[i] += ch[i] / channels;
  }
  return mono;
}

async function run(track) {
  try {
    const mono = await decodeMono(track);
    await new Promise((r) => setTimeout(r, 0)); // laisse respirer l'interface entre le décodage et le calcul
    remember(track, analyzeSamples(mono));
  } catch (err) {
    console.warn('AutoMix : analyse impossible', track.title, err?.message || err);
    remember(track, null);
  }
}

/** Lance l'analyse d'un morceau si elle n'est pas connue ; une seule à la fois pour ne pas peser sur la lecture. */
/** Sur le web, sur une connexion mobile ou en économie de données, on ne télécharge pas un morceau en avance pour l'analyser. */
const metered = () => {
  const c = navigator.connection;
  return !!(window.control?.platform === 'web' && c && (c.saveData || c.type === 'cellular'));
};

export function warm(track) {
  if (!track || track.kind !== 'audio' || !track.id || metered()) return Promise.resolve();
  if (peekAnalysis(track) !== undefined) return Promise.resolve();
  if (!(track.duration > 0) || track.duration > MAX_DURATION) {
    remember(track, null);
    return Promise.resolve();
  }
  let p = pending.get(track.id);
  if (!p) {
    p = chain.then(() => run(track)).finally(() => pending.delete(track.id));
    chain = p.catch(() => {});
    pending.set(track.id, p);
  }
  return p;
}

/** Résultat quand il est prêt (utile aux tests). */
export async function analysisOf(track) {
  await warm(track);
  return peekAnalysis(track);
}

export function forgetAnalyses() {
  memo.clear();
  disk = {};
  try {
    localStorage.removeItem(KEY);
  } catch { /* rien */ }
}
