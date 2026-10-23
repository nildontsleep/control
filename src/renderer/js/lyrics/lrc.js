// Format LRC : « [mm:ss.xx] texte ». Gère plusieurs repères par ligne, le décalage [offset:] et les mots minutés <mm:ss.xx>.

const STAMP = /\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]/g;

function seconds(min, sec, frac) {
  const f = frac ? Number(`0.${frac}`) : 0;
  return Number(min) * 60 + Number(sec) + f;
}

/** Retourne { synced, lines: [{ t, text }] } ; sans repères de temps, lines = lignes de texte (t = null). */
export function parseLrc(raw) {
  const text = String(raw || '').replace(/^﻿/, '').replace(/\r\n?/g, '\n');
  const timed = [];
  const plain = [];
  // Le décalage s'applique à tout le fichier, où qu'il soit déclaré.
  const declared = /^\[offset:\s*([+-]?\d+)\s*\]/im.exec(text);
  const offset = declared ? Number(declared[1]) / 1000 : 0;
  for (const line of text.split('\n')) {
    if (/^\[offset:/i.test(line.trim())) continue;
    if (/^\[[a-z]{2,}:[^\]]*\]$/i.test(line.trim())) continue; // [ar:…] [ti:…] [al:…]
    const stamps = [...line.matchAll(STAMP)];
    const body = line.replace(STAMP, '').replace(/<\d{1,3}:\d{2}(?:[.:]\d{1,3})?>/g, '').trim();
    if (stamps.length) {
      for (const m of stamps) timed.push({ t: Math.max(0, seconds(m[1], m[2], m[3]) - offset), text: body });
    } else {
      plain.push(line.trim());
    }
  }
  if (timed.length >= 3) {
    timed.sort((a, b) => a.t - b.t);
    return { synced: true, lines: timed };
  }
  const all = plain.length ? plain : timed.map((x) => x.text);
  // Un seul saut de ligne vide entre deux strophes suffit.
  const cleaned = [];
  for (const line of all) {
    if (!line && (!cleaned.length || !cleaned[cleaned.length - 1])) continue;
    cleaned.push(line);
  }
  while (cleaned.length && !cleaned[cleaned.length - 1]) cleaned.pop();
  return { synced: false, lines: cleaned.map((text) => ({ t: null, text })) };
}

/** Index de la ligne en cours à l'instant `time` (-1 avant la première). */
export function lineAt(lines, time) {
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].t <= time) { found = mid; lo = mid + 1; } else hi = mid - 1;
  }
  return found;
}
