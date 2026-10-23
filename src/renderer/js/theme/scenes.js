// Fonds animés (canvas 2D) : une scène à la fois, teintée par la couleur d'accentuation, discrète et peu gourmande.

const TAU = Math.PI * 2;
const rand = (a, b) => a + Math.random() * (b - a);
const rgba = (c, a) => `rgba(${c.r},${c.g},${c.b},${a})`;

export const SCENES = [
  'none', 'embers', 'stars', 'fireflies', 'ripples', 'nebula', 'rain', 'plasma', 'meteors',
  'snow', 'hyperspace', 'grid', 'bubbles', 'constellation', 'mist', 'aurora',
];

// Un halo flou, dessiné une fois puis réutilisé : bien plus léger qu'un dégradé radial par particule.
const sprites = new Map();
function glow(c) {
  const key = `${c.r},${c.g},${c.b}`;
  if (!sprites.has(key)) {
    const size = 64;
    const el = document.createElement('canvas');
    el.width = size;
    el.height = size;
    const g = el.getContext('2d');
    const grad = g.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
    grad.addColorStop(0, rgba({ r: 255, g: 255, b: 255 }, 0.95));
    grad.addColorStop(0.18, rgba(c, 0.8));
    grad.addColorStop(1, rgba(c, 0));
    g.fillStyle = grad;
    g.fillRect(0, 0, size, size);
    sprites.set(key, el);
    if (sprites.size > 8) sprites.delete(sprites.keys().next().value);
  }
  return sprites.get(key);
}

const hueShift = (c, deg) => {
  const [r, g, b] = [c.r / 255, c.g / 255, c.b / 255];
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  let h = 0;
  if (d) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
  }
  h = (h * 60 + deg + 360) % 360;
  const s = d ? d / (1 - Math.abs(2 * l - 1)) : 0;
  const f = (n) => {
    const k = (n + h / 30) % 12;
    return Math.round(255 * (l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1))));
  };
  return { r: f(0), g: f(8), b: f(4) };
};

// Chaque fabrique retourne une fonction de dessin (ctx, largeur, hauteur, dt en s, t en s, couleur).
const FACTORIES = {
  embers() {
    const p = Array.from({ length: 80 }, () => ({ x: Math.random(), y: Math.random(), s: rand(5, 15), v: rand(0.03, 0.1), ph: rand(0, TAU) }));
    return (c, w, h, dt, t, col) => {
      const sprite = glow(col);
      for (const e of p) {
        e.y -= e.v * dt;
        e.x += Math.sin(t * 0.7 + e.ph) * 0.012 * dt;
        if (e.y < -0.05) { e.y = 1.05; e.x = Math.random(); }
        c.globalAlpha = (0.25 + 0.55 * Math.abs(Math.sin(t * 1.6 + e.ph))) * Math.min(1, e.y * 2.5);
        c.drawImage(sprite, e.x * w - e.s, e.y * h - e.s, e.s * 2, e.s * 2);
      }
      c.globalAlpha = 1;
    };
  },

  stars() {
    const stars = Array.from({ length: 150 }, () => ({ x: Math.random(), y: Math.random(), r: rand(0.5, 1.7), ph: rand(0, TAU), f: rand(0.6, 2) }));
    let shoot = null;
    let next = rand(3, 7);
    return (c, w, h, dt, t, col) => {
      c.fillStyle = rgba({ r: 255, g: 255, b: 255 }, 1);
      for (const s of stars) {
        c.globalAlpha = 0.25 + 0.65 * (0.5 + 0.5 * Math.sin(t * s.f + s.ph));
        c.beginPath();
        c.arc(s.x * w, s.y * h, s.r, 0, TAU);
        c.fill();
      }
      next -= dt;
      if (next <= 0) { shoot = { x: rand(0.1, 0.9), y: rand(0, 0.4), life: 0 }; next = rand(5, 11); }
      if (shoot) {
        shoot.life += dt * 1.4;
        const k = shoot.life;
        const x = shoot.x * w + k * 380;
        const y = shoot.y * h + k * 190;
        const g = c.createLinearGradient(x, y, x - 140, y - 70);
        g.addColorStop(0, rgba(col, 0.9));
        g.addColorStop(1, rgba(col, 0));
        c.globalAlpha = Math.max(0, 1 - k);
        c.strokeStyle = g;
        c.lineWidth = 1.6;
        c.beginPath();
        c.moveTo(x, y);
        c.lineTo(x - 140, y - 70);
        c.stroke();
        if (k >= 1) shoot = null;
      }
      c.globalAlpha = 1;
    };
  },

  fireflies() {
    const f = Array.from({ length: 34 }, () => ({ x: Math.random(), y: Math.random(), r: rand(8, 20), ph: rand(0, TAU), sp: rand(0.15, 0.4), ax: rand(0.02, 0.07), ay: rand(0.02, 0.06) }));
    return (c, w, h, dt, t, col) => {
      const sprite = glow(col);
      for (const e of f) {
        const x = (e.x + Math.sin(t * e.sp + e.ph) * e.ax) * w;
        const y = (e.y + Math.cos(t * e.sp * 0.8 + e.ph) * e.ay) * h;
        c.globalAlpha = 0.15 + 0.7 * Math.max(0, Math.sin(t * 0.9 + e.ph));
        c.drawImage(sprite, x - e.r, y - e.r, e.r * 2, e.r * 2);
      }
      c.globalAlpha = 1;
    };
  },

  ripples() {
    const rings = [];
    let next = 0;
    return (c, w, h, dt, t, col) => {
      next -= dt;
      if (next <= 0) { rings.push({ x: rand(0.1, 0.9) * w, y: rand(0.15, 0.85) * h, r: 0, max: rand(160, 320) }); next = rand(0.9, 1.9); }
      c.lineWidth = 1.5;
      for (let i = rings.length - 1; i >= 0; i--) {
        const r = rings[i];
        r.r += dt * 55;
        const k = r.r / r.max;
        if (k >= 1) { rings.splice(i, 1); continue; }
        for (let j = 0; j < 3; j++) {
          const rr = r.r - j * 16;
          if (rr <= 0) continue;
          c.strokeStyle = rgba(col, (1 - k) * 0.4 * (1 - j * 0.3));
          c.beginPath();
          c.arc(r.x, r.y, rr, 0, TAU);
          c.stroke();
        }
      }
    };
  },

  nebula() {
    const blobs = Array.from({ length: 5 }, (_, i) => ({ x: rand(0.1, 0.9), y: rand(0.1, 0.9), r: rand(0.3, 0.55), ph: rand(0, TAU), sp: rand(0.05, 0.12), hue: (i - 2) * 28 }));
    return (c, w, h, dt, t, col) => {
      c.globalCompositeOperation = 'lighter';
      for (const b of blobs) {
        const x = (b.x + Math.sin(t * b.sp + b.ph) * 0.1) * w;
        const y = (b.y + Math.cos(t * b.sp * 0.9 + b.ph) * 0.1) * h;
        const r = b.r * Math.max(w, h);
        const k = hueShift(col, b.hue);
        const g = c.createRadialGradient(x, y, 0, x, y, r);
        g.addColorStop(0, rgba(k, 0.3));
        g.addColorStop(0.5, rgba(k, 0.1));
        g.addColorStop(1, rgba(k, 0));
        c.fillStyle = g;
        c.fillRect(x - r, y - r, r * 2, r * 2);
      }
      c.globalCompositeOperation = 'source-over';
    };
  },

  rain() {
    const d = Array.from({ length: 130 }, () => ({ x: Math.random(), y: Math.random(), l: rand(10, 24), v: rand(0.5, 1.0) }));
    return (c, w, h, dt, t, col) => {
      c.lineWidth = 1.1;
      for (const e of d) {
        e.y += e.v * dt * 0.9;
        e.x -= e.v * dt * 0.12;
        if (e.y > 1.05) { e.y = -0.05; e.x = Math.random() * 1.2; }
        c.strokeStyle = rgba(col, 0.15 + e.v * 0.3);
        c.beginPath();
        c.moveTo(e.x * w, e.y * h);
        c.lineTo(e.x * w - e.l * 0.13, e.y * h + e.l);
        c.stroke();
      }
    };
  },

  plasma() {
    const W = 72;
    const H = 40;
    const buf = document.createElement('canvas');
    buf.width = W;
    buf.height = H;
    const bg = buf.getContext('2d');
    const img = bg.createImageData(W, H);
    return (c, w, h, dt, t, col) => {
      const a = hueShift(col, -35);
      const b = hueShift(col, 35);
      for (let y = 0; y < H; y++) {
        for (let x = 0; x < W; x++) {
          const v = (Math.sin(x * 0.17 + t * 0.9) + Math.sin(y * 0.23 - t * 0.7) + Math.sin((x + y) * 0.11 + t * 0.5) + Math.sin(Math.hypot(x - W / 2, y - H / 2) * 0.2 - t * 0.8)) / 4;
          const k = 0.5 + 0.5 * v;
          const i = (y * W + x) * 4;
          img.data[i] = a.r + (b.r - a.r) * k;
          img.data[i + 1] = a.g + (b.g - a.g) * k;
          img.data[i + 2] = a.b + (b.b - a.b) * k;
          img.data[i + 3] = 40 + 120 * k;
        }
      }
      bg.putImageData(img, 0, 0);
      c.imageSmoothingEnabled = true;
      c.globalAlpha = 0.55;
      c.drawImage(buf, 0, 0, w, h);
      c.globalAlpha = 1;
    };
  },

  meteors() {
    const m = [];
    let next = 0;
    return (c, w, h, dt, t, col) => {
      next -= dt;
      if (next <= 0) { m.push({ x: rand(-0.1, 1) * w, y: rand(-0.2, 0.3) * h, v: rand(520, 900), l: rand(120, 260), life: 0 }); next = rand(0.35, 1.1); }
      for (let i = m.length - 1; i >= 0; i--) {
        const e = m[i];
        e.life += dt;
        e.x += e.v * dt * 0.8;
        e.y += e.v * dt * 0.6;
        if (e.x > w + 300 || e.y > h + 300) { m.splice(i, 1); continue; }
        const g = c.createLinearGradient(e.x, e.y, e.x - e.l * 0.8, e.y - e.l * 0.6);
        g.addColorStop(0, rgba({ r: 255, g: 255, b: 255 }, 0.9));
        g.addColorStop(0.15, rgba(col, 0.6));
        g.addColorStop(1, rgba(col, 0));
        c.strokeStyle = g;
        c.lineWidth = 1.8;
        c.beginPath();
        c.moveTo(e.x, e.y);
        c.lineTo(e.x - e.l * 0.8, e.y - e.l * 0.6);
        c.stroke();
      }
    };
  },

  snow() {
    const f = Array.from({ length: 110 }, () => ({ x: Math.random(), y: Math.random(), r: rand(1, 3.4), v: rand(0.02, 0.07), ph: rand(0, TAU) }));
    return (c, w, h, dt, t) => {
      c.fillStyle = 'rgb(255,255,255)';
      for (const e of f) {
        e.y += e.v * dt;
        if (e.y > 1.03) { e.y = -0.03; e.x = Math.random(); }
        c.globalAlpha = 0.35 + e.r * 0.15;
        c.beginPath();
        c.arc((e.x + Math.sin(t * 0.5 + e.ph) * 0.02) * w, e.y * h, e.r, 0, TAU);
        c.fill();
      }
      c.globalAlpha = 1;
    };
  },

  hyperspace() {
    const s = Array.from({ length: 170 }, () => ({ x: rand(-1, 1), y: rand(-1, 1), z: Math.random() }));
    return (c, w, h, dt, t, col) => {
      const cx = w / 2;
      const cy = h / 2;
      const f = Math.max(w, h) * 0.6;
      c.lineCap = 'round';
      for (const e of s) {
        const pz = e.z;
        e.z -= dt * 0.35;
        if (e.z <= 0.02) { e.x = rand(-1, 1); e.y = rand(-1, 1); e.z = 1; continue; }
        const x1 = cx + (e.x / pz) * f * 0.25;
        const y1 = cy + (e.y / pz) * f * 0.25;
        const x2 = cx + (e.x / e.z) * f * 0.25;
        const y2 = cy + (e.y / e.z) * f * 0.25;
        const a = Math.min(1, (1 - e.z) * 1.4);
        c.strokeStyle = rgba(col, a * 0.7);
        c.lineWidth = 0.6 + (1 - e.z) * 1.8;
        c.beginPath();
        c.moveTo(x1, y1);
        c.lineTo(x2, y2);
        c.stroke();
      }
    };
  },

  grid() {
    return (c, w, h, dt, t, col) => {
      const horizon = h * 0.52;
      c.lineWidth = 1;
      const fade = c.createLinearGradient(0, horizon, 0, h);
      fade.addColorStop(0, rgba(col, 0));
      fade.addColorStop(1, rgba(col, 0.55));
      c.strokeStyle = fade;
      for (let i = -14; i <= 14; i++) {
        c.beginPath();
        c.moveTo(w / 2 + i * 22, horizon);
        c.lineTo(w / 2 + i * w * 0.11, h);
        c.stroke();
      }
      const phase = (t * 0.25) % 1;
      for (let i = 0; i < 14; i++) {
        const k = ((i + phase) / 14) ** 2.2;
        const y = horizon + k * (h - horizon);
        c.strokeStyle = rgba(col, 0.08 + k * 0.45);
        c.beginPath();
        c.moveTo(0, y);
        c.lineTo(w, y);
        c.stroke();
      }
    };
  },

  bubbles() {
    const b = Array.from({ length: 36 }, () => ({ x: Math.random(), y: Math.random(), r: rand(5, 22), v: rand(0.02, 0.07), ph: rand(0, TAU) }));
    return (c, w, h, dt, t, col) => {
      for (const e of b) {
        e.y -= e.v * dt;
        if (e.y < -0.08) { e.y = 1.08; e.x = Math.random(); }
        const x = (e.x + Math.sin(t * 0.6 + e.ph) * 0.015) * w;
        const y = e.y * h;
        c.strokeStyle = rgba(col, 0.4);
        c.fillStyle = rgba(col, 0.07);
        c.lineWidth = 1.2;
        c.beginPath();
        c.arc(x, y, e.r, 0, TAU);
        c.fill();
        c.stroke();
        c.fillStyle = 'rgba(255,255,255,0.35)';
        c.beginPath();
        c.arc(x - e.r * 0.35, y - e.r * 0.35, e.r * 0.18, 0, TAU);
        c.fill();
      }
    };
  },

  constellation() {
    const p = Array.from({ length: 64 }, () => ({ x: Math.random(), y: Math.random(), vx: rand(-0.012, 0.012), vy: rand(-0.012, 0.012) }));
    return (c, w, h, dt, t, col) => {
      const pts = p.map((e) => {
        e.x += e.vx * dt;
        e.y += e.vy * dt;
        if (e.x < 0 || e.x > 1) e.vx *= -1;
        if (e.y < 0 || e.y > 1) e.vy *= -1;
        return [e.x * w, e.y * h];
      });
      c.lineWidth = 1;
      for (let i = 0; i < pts.length; i++) {
        for (let j = i + 1; j < pts.length; j++) {
          const d = Math.hypot(pts[i][0] - pts[j][0], pts[i][1] - pts[j][1]);
          if (d < 150) {
            c.strokeStyle = rgba(col, (1 - d / 150) * 0.4);
            c.beginPath();
            c.moveTo(pts[i][0], pts[i][1]);
            c.lineTo(pts[j][0], pts[j][1]);
            c.stroke();
          }
        }
      }
      c.fillStyle = rgba(col, 0.8);
      for (const [x, y] of pts) {
        c.beginPath();
        c.arc(x, y, 1.8, 0, TAU);
        c.fill();
      }
    };
  },

  mist() {
    const m = Array.from({ length: 6 }, () => ({ x: rand(0, 1), y: rand(0.2, 0.9), r: rand(0.25, 0.5), v: rand(0.008, 0.02), a: rand(0.07, 0.15) }));
    return (c, w, h, dt, t, col) => {
      for (const e of m) {
        e.x += e.v * dt;
        if (e.x - e.r > 1.1) e.x = -e.r;
        const x = e.x * w;
        const y = e.y * h;
        const r = e.r * Math.max(w, h);
        const g = c.createRadialGradient(x, y, 0, x, y, r);
        g.addColorStop(0, rgba(col, e.a));
        g.addColorStop(1, rgba(col, 0));
        c.fillStyle = g;
        c.fillRect(x - r, y - r, r * 2, r * 2);
      }
    };
  },

  aurora() {
    const bands = Array.from({ length: 4 }, (_, i) => ({ y: 0.22 + i * 0.1, ph: rand(0, TAU), sp: rand(0.12, 0.25), hue: (i - 1.5) * 32 }));
    return (c, w, h, dt, t, col) => {
      c.globalCompositeOperation = 'lighter';
      for (const b of bands) {
        const k = hueShift(col, b.hue);
        const top = b.y * h;
        const g = c.createLinearGradient(0, top - 90, 0, top + 190);
        g.addColorStop(0, rgba(k, 0));
        g.addColorStop(0.4, rgba(k, 0.2));
        g.addColorStop(1, rgba(k, 0));
        c.fillStyle = g;
        c.beginPath();
        c.moveTo(0, top + 190);
        for (let x = 0; x <= w; x += 24) {
          c.lineTo(x, top + Math.sin(x * 0.006 + t * b.sp + b.ph) * 46 + Math.sin(x * 0.013 - t * b.sp * 1.4) * 18);
        }
        c.lineTo(w, top + 190);
        c.closePath();
        c.fill();
      }
      c.globalCompositeOperation = 'source-over';
    };
  },
};

export function createScene(canvas) {
  const ctx = canvas.getContext('2d');
  let w = 1;
  let h = 1;
  let id = 'none';
  let draw = null;
  let speed = 1;
  let color = { r: 255, g: 55, b: 95 };
  let raf = 0;
  let last = 0;
  let time = 30; // on commence « en cours de route » : pas de départ à vide
  let still = false;
  let slow = false;

  function resize() {
    const rect = canvas.getBoundingClientRect();
    w = canvas.width = Math.max(2, Math.round(rect.width));
    h = canvas.height = Math.max(2, Math.round(rect.height));
    if (draw && still) paintOnce();
  }

  function paintOnce() {
    ctx.clearRect(0, 0, w, h);
    for (let i = 0; i < 40; i++) draw(ctx, w, h, 0.05, (time += 0.05), color);
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    if (document.hidden) return;
    const gap = slow ? 1000 / 12 : 1000 / 36;
    if (now - last < gap) return;
    const dt = Math.min(0.1, (now - last) / 1000) * speed;
    last = now;
    time += dt;
    ctx.clearRect(0, 0, w, h);
    draw(ctx, w, h, dt, time, color);
  }

  function run() {
    cancelAnimationFrame(raf);
    raf = 0;
    if (!draw) return;
    if (still) { paintOnce(); return; }
    last = performance.now();
    raf = requestAnimationFrame(frame);
  }

  window.addEventListener('resize', resize);

  return {
    resize,
    set(next) {
      id = FACTORIES[next] ? next : 'none';
      resize();
      draw = id === 'none' ? null : FACTORIES[id]();
      canvas.classList.toggle('on', !!draw);
      if (!draw) ctx.clearRect(0, 0, w, h);
      run();
    },
    get id() {
      return id;
    },
    setSpeed(v) { speed = Math.max(0.1, v); },
    setColor(rgb) { color = { r: rgb[0], g: rgb[1], b: rgb[2] }; if (draw && still) paintOnce(); },
    /** Animations désactivées : une image fixe plutôt qu'un mouvement. */
    setStill(on) { if (still !== !!on) { still = !!on; run(); } },
    /** Fenêtre en arrière-plan : on ralentit au minimum. */
    setSlow(on) { slow = !!on; },
  };
}
