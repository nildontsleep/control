// Couleurs d’accentuation macOS et conversion de température de couleur.

export const ACCENTS = {
  blue: { dark: '#0a84ff', light: '#007aff' },
  purple: { dark: '#bf5af2', light: '#a550d8' },
  pink: { dark: '#ff375f', light: '#f2295a' },
  red: { dark: '#ff453a', light: '#ff3b30' },
  orange: { dark: '#ff9f0a', light: '#f08c00' },
  yellow: { dark: '#ffd60a', light: '#e6b800' },
  green: { dark: '#30d158', light: '#28b745' },
  graphite: { dark: '#98989d', light: '#8e8e93' },
};

export function hexToRgb(hex) {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex || '');
  if (!m) return [10, 132, 255];
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function luminance([r, g, b]) {
  const f = (c) => {
    c /= 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** Approximation de Tanner Helland : kelvins → RGB (0–255). */
export function kelvinToRgb(kelvin) {
  const t = kelvin / 100;
  let r;
  let g;
  let b;
  if (t <= 66) {
    r = 255;
    g = 99.4708025861 * Math.log(t) - 161.1195681661;
    b = t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447927307;
  } else {
    r = 329.698727446 * (t - 60) ** -0.1332047592;
    g = 288.1221695283 * (t - 60) ** -0.0755148492;
    b = 255;
  }
  const c = (v) => Math.round(Math.min(255, Math.max(0, v)));
  return [c(r), c(g), c(b)];
}
