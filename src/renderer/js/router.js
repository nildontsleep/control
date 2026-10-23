// Navigation entre les vues, avec historique (précédent / suivant) et rechargement en gardant le défilement.

const views = new Map();
const history = [];
let position = -1;
let container = null;
let scroller = null;
let active = null;
const listeners = new Set();

export const registerView = (name, render) => views.set(name, render);
export const onRoute = (fn) => {
  listeners.add(fn);
  return () => listeners.delete(fn);
};

export function mount(el, scrollEl) {
  container = el;
  scroller = scrollEl;
}

export const currentRoute = () => history[position] || null;
export const canBack = () => position > 0;
export const canForward = () => position < history.length - 1;

function render(route, { keepScroll = false } = {}) {
  const view = views.get(route.name);
  if (!view || !container) return;
  const top = keepScroll ? scroller.scrollTop : 0;
  active?.destroy?.();
  container.replaceChildren();
  const fresh = !keepScroll;
  active = view(container, route.param, { scroller, route }) || {};
  scroller.scrollTop = top;
  if (fresh) {
    container.classList.remove('view-in');
    void container.offsetWidth;
    container.classList.add('view-in');
  }
  listeners.forEach((fn) => fn(route));
}

export function navigate(name, param = null, { replace = false } = {}) {
  const route = { name, param };
  const cur = currentRoute();
  if (cur && cur.name === name && cur.param === param && !replace) return;
  if (replace && position >= 0) history[position] = route;
  else {
    history.splice(position + 1);
    history.push(route);
    position = history.length - 1;
  }
  render(route);
}

export function back() {
  if (!canBack()) return;
  position -= 1;
  render(history[position]);
}

export function forward() {
  if (!canForward()) return;
  position += 1;
  render(history[position]);
}

export function refresh() {
  const route = currentRoute();
  if (route) render(route, { keepScroll: true });
}

export const activeView = () => active;
