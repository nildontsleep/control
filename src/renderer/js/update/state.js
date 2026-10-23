// État de la mise à jour de l'application, partagé entre la notification, la barre latérale et les réglages.

const api = window.control?.update;

let state = { phase: 'idle' };
const listeners = new Set();

export const updateState = () => state;

export function onUpdate(fn) {
  listeners.add(fn);
  fn(state);
  return () => listeners.delete(fn);
}

/** `onAvailable` est appelé une seule fois par version détectée. */
export function startUpdates({ onAvailable } = {}) {
  if (!api) return;
  let announced = '';
  const apply = (next) => {
    state = next || { phase: 'idle' };
    listeners.forEach((fn) => fn(state));
    if (state.phase === 'available' && announced !== state.version) {
      announced = state.version;
      onAvailable?.(state);
    }
  };
  api.state().then(apply).catch(() => {});
  api.onState(apply);
}

export const checkNow = () => api?.check();

/** Télécharge, vérifie, puis installe et relance : un seul clic pour l'utilisateur. */
export async function updateNow() {
  if (!api) return;
  const res = state.phase === 'ready' ? state : await api.download();
  if (res?.phase === 'ready') await api.install();
}
