// Sons synthétisés (aucun fichier audio) : obturateur, bips du retardateur, enregistrement.

let ctx = null;

function audio() {
  if (!ctx) {
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    ctx = new AC();
  }
  if (ctx.state === 'suspended') ctx.resume();
  return ctx;
}

function noiseBurst(ac, at, duration, freq, gain) {
  const len = Math.ceil(ac.sampleRate * duration);
  const buf = ac.createBuffer(1, len, ac.sampleRate);
  const data = buf.getChannelData(0);
  for (let i = 0; i < len; i++) data[i] = (Math.random() * 2 - 1) * (1 - i / len) ** 2.5;
  const src = ac.createBufferSource();
  src.buffer = buf;
  const filter = ac.createBiquadFilter();
  filter.type = 'bandpass';
  filter.frequency.value = freq;
  filter.Q.value = 0.9;
  const g = ac.createGain();
  g.gain.value = gain;
  src.connect(filter).connect(g).connect(ac.destination);
  src.start(at);
}

function tone(ac, at, freq, duration, gain, type = 'sine') {
  const osc = ac.createOscillator();
  osc.type = type;
  osc.frequency.value = freq;
  const g = ac.createGain();
  g.gain.setValueAtTime(0, at);
  g.gain.linearRampToValueAtTime(gain, at + 0.008);
  g.gain.exponentialRampToValueAtTime(0.0001, at + duration);
  osc.connect(g).connect(ac.destination);
  osc.start(at);
  osc.stop(at + duration + 0.02);
}

export function shutter() {
  const ac = audio();
  if (!ac) return;
  const now = ac.currentTime + 0.005;
  noiseBurst(ac, now, 0.05, 3200, 0.55);
  tone(ac, now, 180, 0.05, 0.12, 'triangle');
  noiseBurst(ac, now + 0.085, 0.07, 2200, 0.4);
  tone(ac, now + 0.085, 140, 0.06, 0.1, 'triangle');
}

export function tick(last = false) {
  const ac = audio();
  if (!ac) return;
  tone(ac, ac.currentTime + 0.005, last ? 1318.5 : 987.8, 0.12, 0.07);
}

export function recordStart() {
  const ac = audio();
  if (!ac) return;
  const n = ac.currentTime + 0.005;
  tone(ac, n, 659.3, 0.12, 0.08);
  tone(ac, n + 0.1, 987.8, 0.18, 0.08);
}

export function recordStop() {
  const ac = audio();
  if (!ac) return;
  const n = ac.currentTime + 0.005;
  tone(ac, n, 987.8, 0.12, 0.08);
  tone(ac, n + 0.1, 659.3, 0.18, 0.08);
}
