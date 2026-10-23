// Applique l'apparence choisie : mode (dont OLED), échelle, arrondi, translucidité, fond animé.

import { settings } from '../settings.js';
import { createScene } from './scenes.js';

const root = document.documentElement;
let scene = null;

const clamp = (v, a, b) => Math.min(b, Math.max(a, Number(v) || 0));

/** Surcharges propres au mode OLED : noir profond, dont la profondeur se règle. */
function paintOled(on) {
  const names = ['--bg', '--sidebar-bg', '--bar-bg', '--veil', '--panel', '--menu', '--panel-solid'];
  if (!on) {
    for (const n of names) root.style.removeProperty(n);
    return;
  }
  const k = clamp(settings.oledLevel, 0, 100) / 100;
  const v = (lo, hi) => Math.round(lo + (hi - lo) * k);
  root.style.setProperty('--bg', `rgb(${v(0, 13)} ${v(0, 13)} ${v(0, 16)})`);
  root.style.setProperty('--veil', `rgb(${v(0, 13)} ${v(0, 13)} ${v(0, 16)} / 0.55)`);
  root.style.setProperty('--sidebar-bg', `rgb(${v(2, 24)} ${v(2, 24)} ${v(3, 30)} / calc(0.62 * var(--tl, 1)))`);
  root.style.setProperty('--bar-bg', `rgb(${v(4, 30)} ${v(4, 30)} ${v(5, 36)} / calc(0.66 * var(--tl, 1)))`);
  root.style.setProperty('--panel', `rgb(${v(6, 28)} ${v(6, 28)} ${v(7, 32)} / calc(0.66 * var(--tl, 1)))`);
  root.style.setProperty('--menu', `rgb(${v(8, 38)} ${v(8, 38)} ${v(9, 44)} / calc(0.74 * var(--tl, 1)))`);
  root.style.setProperty('--panel-solid', `rgb(${v(8, 36)} ${v(8, 36)} ${v(9, 42)})`);
}

/** À appeler après chaque changement de réglage d'apparence. `dark` : le thème effectif est sombre. */
export function applyAmbience({ dark, rgb, active = true }) {
  const oled = settings.theme === 'oled';
  root.dataset.oled = oled ? '1' : '';
  paintOled(oled && dark);

  root.style.setProperty('--rk', String(clamp(settings.radius, 20, 180) / 100));
  root.style.setProperty('--tl', String(clamp(settings.translucency, 30, 170) / 100));
  root.style.setProperty('--scene-blur', `${clamp(settings.sceneBlur, 0, 14)}px`);
  window.control?.win?.setZoom?.(clamp(settings.uiScale, 80, 130) / 100);

  const canvas = document.getElementById('scene');
  if (!canvas) return;
  scene ??= createScene(canvas);
  scene.setColor(rgb);
  scene.setSpeed(clamp(settings.sceneSpeed, 20, 250) / 100);
  scene.setStill(!settings.animations);
  scene.setSlow(!active);
  if (scene.id !== settings.scene) scene.set(settings.scene);
}

export const setSceneActive = (active) => scene?.setSlow(!active);
