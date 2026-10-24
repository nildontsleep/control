// Discord Rich Presence : décrit ce qui se joue (musique, vidéo locale, streaming) au processus principal,
// qui le transmet au client Discord. Rien n'est envoyé si la présence est désactivée.

import { player, on } from '../player.js';
import { settings, subscribe } from '../settings.js';
import { t } from '../i18n.js';

const api = window.control?.discord;
const IDLE_CLEAR = 10 * 60 * 1000; // une pause de plus de 10 minutes efface la présence
const REFRESH = 1500;

let streaming = null; // { type, title, subtitle, poster, position, duration, playing, at }
let timer = null;
let lastSent = '';
let pausedSince = 0;
let sentPosition = 0;
let sentAt = 0;

const options = () => settings.discord || {};

function fromStreaming(s, d) {
  const hide = !!d.hideTitles;
  const position = s.playing ? s.position + (Date.now() - s.at) / 1000 : s.position;
  return {
    kind: 'watching',
    details: hide ? t(`dc.hide.${s.type}`) : s.title,
    state: hide ? null : s.subtitle,
    image: hide || d.covers === false ? null : s.poster,
    imageText: hide ? null : s.title,
    icon: s.playing ? 'watching' : 'pause',
    iconText: s.playing ? t('dc.watchingNow') : t('dc.paused'),
    playing: s.playing,
    position,
    duration: s.duration,
  };
}

function fromTrack(track, d) {
  const video = track.kind === 'video';
  const hide = !!d.hideTitles;
  const playing = player.playing;
  const artist = track.artist && track.artist !== 'Unknown' ? track.artist : '';
  const base = {
    kind: video ? 'watching' : 'listening',
    icon: playing ? (video ? 'video' : 'play') : 'pause',
    iconText: playing ? t(video ? 'dc.watchingNow' : 'dc.listeningNow') : t('dc.paused'),
    playing,
    position: player.time,
    duration: player.duration,
  };
  if (hide) return { ...base, details: t(video ? 'dc.hide.video' : 'dc.hide.music') };
  return {
    ...base,
    details: track.title,
    state: video ? (track.artist || t('dc.localVideo')) : artist || t('dc.unknownArtist'),
    imageText: track.album || undefined,
    // La pochette du fichier (tag) ou l'image de la vidéo : le processus principal la rend accessible à Discord.
    cover: d.covers === false ? null : track.cover ? { kind: 'cover', id: track.cover } : video ? { kind: 'vthumb', id: track.id } : null,
  };
}

function compute() {
  const d = options();
  if (d.enabled === false) return null;
  if (streaming && d.streaming !== false) return fromStreaming(streaming, d);
  const track = player.track;
  if (!track) return null;
  if (track.kind === 'video' ? d.video === false : d.music === false) return null;
  if (!player.playing && pausedSince && Date.now() - pausedSince > IDLE_CLEAR) return null;
  return fromTrack(track, d);
}

function send() {
  timer = null;
  if (!api) return;
  const activity = compute();
  // Évite d'inonder le processus principal : seule une vraie différence (hors position) est renvoyée,
  // et la position n'est renvoyée que si elle s'écarte de ce que Discord compte déjà.
  const { position, ...rest } = activity || {};
  const key = JSON.stringify(activity ? rest : null);
  const expected = activity?.playing && sentAt ? sentPosition + (Date.now() - sentAt) / 1000 : null;
  const drifted = expected !== null && Math.abs(expected - position) > 2.5;
  if (key === lastSent && !drifted) return;
  lastSent = key;
  sentPosition = position || 0;
  sentAt = activity?.playing ? Date.now() : 0;
  api.setActivity(activity);
}

function schedule(delay = 250) {
  clearTimeout(timer);
  timer = setTimeout(send, delay);
}

/** Appelé par la page de lecture (films, séries, animés) ; `null` quand on la quitte. */
export function setWatching(info) {
  streaming = info ? { ...info, at: Date.now() } : null;
  schedule(150);
}

export function startPresence() {
  if (!api) return;
  on('track', () => { pausedSince = 0; schedule(); });
  on('state', () => {
    pausedSince = player.playing ? 0 : (pausedSince || Date.now());
    schedule();
  });
  on('time', () => { if (!timer) timer = setTimeout(send, REFRESH); });
  subscribe((patch) => { if ('discord' in patch || 'language' in patch) { lastSent = '\u0000'; schedule(100); } });
  window.addEventListener('pagehide', () => api.setActivity(null));
  setInterval(() => { if (pausedSince) schedule(100); }, 60 * 1000);
  schedule(1500);
}
