// AutoMix : analyse d'un morceau (tempo, début et fin audibles, repères de battements).
// Fonctions pures, sans dépendance à l'interface : elles reçoivent le son en mono basse fréquence d'échantillonnage.

export const ANALYSIS_RATE = 11025; // Hz : de quoi suivre les kicks et les caisses claires, léger à calculer
const HOP = 256; // échantillons par image d'analyse (≈ 23 ms)
const MIN_BPM = 70;
const MAX_BPM = 180;

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const round = (v, n = 3) => Math.round(v * 10 ** n) / 10 ** n;

/** Valeur d'un signal échantillonné en un point fractionnaire (interpolation linéaire). */
function at(arr, x) {
  const i = Math.floor(x);
  if (i < 0 || i >= arr.length - 1) return 0;
  const f = x - i;
  return arr[i] * (1 - f) + arr[i + 1] * f;
}

/**
 * Enveloppe d'attaques : où le son « frappe ». Deux bandes (graves : kick ; reste : caisse claire, charleston) dont
 * on garde la hausse d'énergie d'une image à l'autre, puis on retire la moyenne locale pour ne garder que les pics.
 */
function onsetEnvelope(x) {
  const frames = Math.floor(x.length / HOP);
  const low = new Float32Array(frames);
  const high = new Float32Array(frames);
  const level = new Float32Array(frames);
  let lp = 0;
  const a = 1 - Math.exp((-2 * Math.PI * 180) / ANALYSIS_RATE);
  for (let f = 0; f < frames; f++) {
    let el = 0;
    let eh = 0;
    let all = 0;
    for (let i = f * HOP, end = i + HOP; i < end; i++) {
      const s = x[i];
      lp += a * (s - lp);
      const hp = s - lp;
      el += lp * lp;
      eh += hp * hp;
      all += s * s;
    }
    low[f] = el / HOP;
    high[f] = eh / HOP;
    level[f] = Math.sqrt(all / HOP);
  }
  const env = new Float32Array(frames);
  const lift = (arr, f) => Math.max(0, Math.log(1e-6 + arr[f]) - Math.log(1e-6 + arr[f - 1]));
  for (let f = 1; f < frames; f++) env[f] = lift(low, f) * 1.0 + lift(high, f) * 0.6;
  // Retire la moyenne locale (≈ 1 s) : seuls les pics bien au-dessus du fond restent.
  const win = Math.round(ANALYSIS_RATE / HOP);
  const out = new Float32Array(frames);
  let sum = 0;
  for (let f = 0; f < frames; f++) {
    sum += env[f];
    if (f >= win) sum -= env[f - win];
    const mean = sum / Math.min(f + 1, win);
    out[f] = Math.max(0, env[f] - mean * 1.2);
  }
  return { env: out, level };
}

/** Début et fin audibles (hors silences de tête et de queue), en secondes. */
function audibleRange(level) {
  const sorted = Float32Array.from(level).sort();
  const loud = sorted[Math.floor(sorted.length * 0.95)] || 0;
  const hopSec = HOP / ANALYSIS_RATE;
  if (loud < 1e-4) return { start: 0, end: level.length * hopSec };
  const thrStart = Math.max(loud * 0.01, 3e-4); // −40 dB
  const thrEnd = Math.max(loud * 0.03, 3e-4); // −30 dB
  let s = 0;
  while (s < level.length && level[s] < thrStart) s++;
  let e = level.length - 1;
  while (e > s && level[e] < thrEnd) e--;
  return { start: Math.max(0, s - 1) * hopSec, end: Math.min(level.length, e + 2) * hopSec };
}

/** Corrélation de l'enveloppe avec elle-même décalée de `lag` images (fractionnaire). */
function autocorr(env, lag, from, to) {
  let sum = 0;
  let n = 0;
  for (let i = from; i < to - lag - 1; i++) {
    sum += env[i] * at(env, i + lag);
    n++;
  }
  return n > 0 ? sum / n : 0;
}

/** Tempo le plus probable de la zone [from, to[ (en images) : { bpm, conf }. */
function estimateTempo(env, from, to) {
  const hopSec = HOP / ANALYSIS_RATE;
  const candidates = [];
  for (let bpm = MIN_BPM; bpm <= MAX_BPM; bpm += 0.25) {
    const lag = 60 / bpm / hopSec;
    // On additionne la corrélation à 1, 2 et 4 temps : un vrai tempo se répète à chaque mesure.
    const score = autocorr(env, lag, from, to) + 0.6 * autocorr(env, lag * 2, from, to) + 0.4 * autocorr(env, lag * 4, from, to);
    // Préférence douce pour les tempos courants (autour de 115 BPM) : tranche entre un tempo et sa moitié ou son double.
    const prior = Math.exp(-0.5 * (Math.log2(bpm / 115) / 0.75) ** 2);
    candidates.push({ bpm, score: score * (0.6 + 0.4 * prior) });
  }
  let best = candidates[0];
  let total = 0;
  for (const c of candidates) {
    total += c.score;
    if (c.score > best.score) best = c;
  }
  const mean = total / candidates.length;
  // Affine autour du meilleur candidat : maximum local de la corrélation simple.
  let bpm = best.bpm;
  let top = -1;
  for (let b = best.bpm - 0.6; b <= best.bpm + 0.6; b += 0.02) {
    const sc = autocorr(env, 60 / b / hopSec, from, to);
    if (sc > top) { top = sc; bpm = b; }
  }
  return { bpm, conf: mean > 0 ? best.score / mean : 0 };
}

/** Instant (en secondes) d'un temps fort de la zone [from, to[ pour un tempo donné : repère de la grille de battements. */
function beatPhase(env, bpm, from, to) {
  const hopSec = HOP / ANALYSIS_RATE;
  const period = 60 / bpm / hopSec; // images par temps
  let bestPhase = 0;
  let bestScore = -1;
  for (let phase = 0; phase < period; phase += 0.25) {
    let s = 0;
    for (let x = from + phase; x < to - 1; x += period) s += at(env, x);
    if (s > bestScore) { bestScore = s; bestPhase = phase; }
  }
  return (from + bestPhase) * hopSec;
}

/**
 * Affine le tempo par mesures de plus en plus espacées : on repère un temps fort dans une fenêtre au début puis dans
 * une autre plus loin, et on divise la durée qui les sépare par le nombre de temps entre eux. Le nombre de temps n'est
 * juste que si le tempo déjà connu dérive de moins d'un demi-temps sur l'écart : on double donc l'écart à chaque tour,
 * chaque mesure rendant la suivante possible. Au bout, une erreur de 0,5 % qui décalerait la grille de plusieurs
 * dixièmes de seconde sur un morceau entier tombe à quelques millisecondes.
 */
function refineTempo(env, bpm0, startF, endF) {
  const hopSec = HOP / ANALYSIS_RATE;
  const toF = (s) => Math.round(s / hopSec);
  const win = toF(10);
  const margin = toF(3);
  const first = startF + margin;
  const last = endF - margin - win; // début de la dernière fenêtre possible
  if (last - first < toF(30)) return bpm0;
  let bpm = bpm0;
  const phaseAt = (from) => beatPhase(env, bpm, from, from + win);
  for (let gap = toF(15); ; gap *= 2) {
    const to = Math.min(first + gap, last);
    const a = phaseAt(first);
    const b = phaseAt(to);
    const beats = Math.round((b - a) / (60 / bpm));
    if (beats < 8) return bpm;
    const next = 60 / ((b - a) / beats);
    // Un écart de plus de 2 % avec le tempo mesuré à grande échelle : mesure aberrante, on garde l'ancien.
    if (Math.abs(next / bpm0 - 1) > 0.02) return bpm;
    bpm = next;
    if (to >= last) break;
  }
  // Une dernière passe sur les fenêtres extrêmes, avec le tempo désormais précis.
  const a = phaseAt(first);
  const b = phaseAt(last);
  const beats = Math.round((b - a) / (60 / bpm));
  if (beats >= 8) {
    const next = 60 / ((b - a) / beats);
    if (Math.abs(next / bpm - 1) < 0.005) bpm = next;
  }
  return bpm;
}

/**
 * @param {Float32Array} mono  signal mono à ANALYSIS_RATE
 * @returns {{ dur:number, bpm:number, conf:number, start:number, end:number, gridStart:number, gridEnd:number }}
 */
export function analyzeSamples(mono) {
  const hopSec = HOP / ANALYSIS_RATE;
  const dur = mono.length / ANALYSIS_RATE;
  const { env, level } = onsetEnvelope(mono);
  const { start, end } = audibleRange(level);
  const startF = Math.floor(start / hopSec);
  const endF = Math.min(env.length, Math.ceil(end / hopSec));
  // Le tempo se mesure sur la partie centrale (l'intro et l'outro sont rarement représentatives), sinon sur tout.
  const span = endF - startF;
  const midFrom = span > 60 / hopSec * 2 ? startF + Math.floor(span * 0.1) : startF;
  const midTo = span > 60 / hopSec * 2 ? endF - Math.floor(span * 0.1) : endF;
  const coarse = estimateTempo(env, midFrom, midTo);
  const bpm = refineTempo(env, coarse.bpm, startF, endF);
  const conf = coarse.conf;
  const win = Math.round(30 / hopSec);
  const gridStart = beatPhase(env, bpm, startF, Math.min(endF, startF + win));
  const gridEnd = beatPhase(env, bpm, Math.max(startF, endF - win), endF);
  return {
    dur: round(dur),
    bpm: round(bpm, 2),
    conf: round(conf, 2),
    start: round(start),
    end: round(end),
    gridStart: round(gridStart),
    gridEnd: round(gridEnd),
  };
}

/** Ce que l'on retient d'une analyse pour décider d'un mixage calé : tempo fiable et morceau assez long. */
export const usable = (a) => !!a && a.conf >= 1.35 && a.dur >= 40 && a.bpm >= MIN_BPM && a.bpm <= MAX_BPM;

/**
 * Plan de transition entre deux morceaux analysés : null si un mixage calé n'est pas possible
 * (alors on fait un fondu enchaîné simple).
 * `from` : instant (dans le précédent) avant lequel le mixage ne peut pas commencer, par exemple quand on arrive
 * dans la fin du morceau par un saut ; le mixage est alors plus court.
 */
export function planMix(out, inc, { rate = 1, maxStretch = 0.08, from = 0 } = {}) {
  if (!usable(out) || !usable(inc)) return null;
  // Tempo du suivant ramené à celui du précédent (un morceau « en double » ou « en moitié » compte comme proche).
  let fold = 1;
  for (const k of [2, 0.5]) {
    if (Math.abs((inc.bpm * k) / out.bpm - 1) < Math.abs((inc.bpm * fold) / out.bpm - 1)) fold = k;
  }
  const incBpm = inc.bpm * fold;
  const ratio = out.bpm / incBpm;
  if (Math.abs(ratio - 1) > maxStretch) return null;
  const beatOut = 60 / out.bpm;
  const beatIn = 60 / incBpm;
  // Durée du mixage : 16 temps, ou 8 si c'est trop long ; entre 5 et 12 s.
  let beats = 16;
  if (beatOut * beats > 12) beats = 8;
  let length = clamp(beatOut * beats, 5, 12);
  // Début du mixage dans le précédent : le dernier temps de sa grille qui laisse la place d'un mixage avant la fin audible.
  const latest = out.end - length;
  let k = Math.floor((latest - out.gridEnd) / beatOut);
  let startOut = out.gridEnd + k * beatOut;
  if (startOut < from) {
    // Trop tard pour le mixage prévu : premier temps possible, et un mixage plus court qui tient avant la fin.
    k = Math.ceil((from - out.gridEnd) / beatOut);
    startOut = out.gridEnd + k * beatOut;
    const fit = Math.floor((out.end - startOut) / beatOut);
    if (fit < 4) return null;
    length = Math.min(length, fit * beatOut);
  }
  if (startOut < 5) return null;
  // Entrée du suivant : le premier temps de sa grille à partir du début audible.
  const j = Math.ceil((inc.start - 0.03 - inc.gridStart) / beatIn);
  const startIn = inc.gridStart + j * beatIn;
  return {
    startOut: round(startOut),
    startIn: round(startIn),
    length: round(length),
    beatOut: round(beatOut, 4),
    beatIn: round(beatIn, 4),
    // Vitesse du suivant par rapport à sa vitesse normale : celle qui cale son tempo sur le précédent.
    ratio: round(ratio, 4),
    // Vitesse du suivant pendant le mixage, par rapport à la vitesse normale (le précédent joue à `rate`).
    rateIn: round(ratio * rate, 4),
    gridOut: out.gridEnd,
    gridIn: inc.gridStart,
  };
}
