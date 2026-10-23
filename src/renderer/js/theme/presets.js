// Ambiances : des réglages d'apparence réunis en un clic (mode, couleur, fond animé, arrondi, échelle, translucidité).
// « control » est l'apparence d'origine : elle reste celle par défaut et sert de retour à l'état initial.

export const LOOK_KEYS = ['theme', 'accent', 'customAccent', 'ambient', 'scene', 'sceneSpeed', 'sceneBlur', 'radius', 'uiScale', 'translucency', 'oledLevel'];

const base = { theme: 'auto', accent: 'pink', customAccent: '#7c5cff', ambient: true, scene: 'none', sceneSpeed: 100, sceneBlur: 0, radius: 100, uiScale: 100, translucency: 100, oledLevel: 25 };

// Les ambiances habillent la fenêtre elles-mêmes : pas de teinte de pochette par-dessus (réactivable dans « Couleurs »).
const look = (patch) => ({ ...base, ambient: false, ...patch, accent: 'custom' });

export const DEFAULT_PRESET = 'control';

export const PRESETS = [
  { key: 'control', patch: { ...base }, swatch: { from: '#1c1c22', to: '#34202a', accent: '#ff375f' } },
  { key: 'ember', patch: look({ theme: 'oled', customAccent: '#e05a3a', scene: 'embers', radius: 78, translucency: 92, oledLevel: 30 }), swatch: { from: '#0a0605', to: '#241009', accent: '#e05a3a' } },
  { key: 'midnight', patch: look({ theme: 'dark', customAccent: '#4b6ea8', scene: 'stars', sceneSpeed: 80, radius: 122, translucency: 75 }), swatch: { from: '#0a0e17', to: '#111a2b', accent: '#4b6ea8' } },
  { key: 'forest', patch: look({ theme: 'dark', customAccent: '#4a7c59', scene: 'fireflies', sceneSpeed: 70, sceneBlur: 1, radius: 133, translucency: 117 }), swatch: { from: '#0a110c', to: '#152018', accent: '#4a7c59' } },
  { key: 'ocean', patch: look({ theme: 'oled', customAccent: '#3aa0c2', scene: 'ripples', sceneSpeed: 90, radius: 111, translucency: 92, oledLevel: 20 }), swatch: { from: '#020608', to: '#0a1a24', accent: '#3aa0c2' } },
  { key: 'iris', patch: look({ theme: 'dark', customAccent: '#8a5eb8', scene: 'nebula', sceneSpeed: 60, sceneBlur: 2, radius: 156, translucency: 108 }), swatch: { from: '#0d0812', to: '#1a1129', accent: '#8a5eb8' } },
  { key: 'sakura', patch: look({ theme: 'light', customAccent: '#d16a94', scene: 'rain', sceneSpeed: 70, sceneBlur: 1, radius: 167, translucency: 125 }), swatch: { from: '#fbeef2', to: '#efd3de', accent: '#d16a94' } },
  { key: 'gold', patch: look({ theme: 'oled', customAccent: '#d1a54a', scene: 'plasma', sceneSpeed: 90, radius: 89, translucency: 83, oledLevel: 10 }), swatch: { from: '#0a0805', to: '#1e1608', accent: '#d1a54a' } },
  { key: 'bloodmoon', patch: look({ theme: 'oled', customAccent: '#9a1b2c', scene: 'meteors', sceneSpeed: 130, radius: 67, translucency: 75, oledLevel: 5 }), swatch: { from: '#080304', to: '#1e050a', accent: '#9a1b2c' } },
  { key: 'snowfall', patch: look({ theme: 'dark', customAccent: '#a8c8e8', scene: 'snow', sceneSpeed: 70, sceneBlur: 1, radius: 133, translucency: 133 }), swatch: { from: '#0a0f18', to: '#141e2c', accent: '#a8c8e8' } },
  { key: 'cyberpunk', patch: look({ theme: 'oled', customAccent: '#ff2fbf', scene: 'hyperspace', sceneSpeed: 140, radius: 44, translucency: 67, oledLevel: 15 }), swatch: { from: '#06020a', to: '#210824', accent: '#ff2fbf' } },
  { key: 'monochrome', patch: look({ theme: 'dark', customAccent: '#c4c4c4', scene: 'none', radius: 30, translucency: 67 }), swatch: { from: '#0a0a0a', to: '#1a1a1a', accent: '#c4c4c4' } },
];

/** Les réglages d'apparence actuels, tels qu'on les enregistrerait dans une ambiance. */
export function currentLook(settings) {
  const out = {};
  for (const k of LOOK_KEYS) out[k] = settings[k];
  return out;
}

const same = (a, b) => LOOK_KEYS.every((k) => (k === 'customAccent' && a.accent !== 'custom' ? true : a[k] === b[k]));

/** Clé de l'ambiance qui correspond exactement aux réglages actuels (ou null). */
export function matchingPreset(settings, customs = []) {
  const now = currentLook(settings);
  for (const p of PRESETS) if (same(p.patch, now)) return p.key;
  for (const p of customs) if (same(p.patch, now)) return p.key;
  return null;
}

/** Tuile de l'ambiance enregistrée : le fond est déduit de la couleur d'accentuation. */
export function swatchFor(patch) {
  const accent = patch.accent === 'custom' ? patch.customAccent : null;
  const hex = accent || '#ff375f';
  const light = patch.theme === 'light';
  return { from: light ? '#f6eef2' : '#0a0a0d', to: light ? '#ead6df' : shade(hex, -0.72), accent: hex };
}

function shade(hex, t) {
  const n = parseInt(hex.replace('#', ''), 16);
  const f = (c) => Math.max(0, Math.min(255, Math.round(t < 0 ? c * (1 + t) : c + (255 - c) * t)));
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(f);
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}
