// Petits utilitaires DOM.

// append(), replaceChildren()… transforment un enfant null / undefined / false en texte (« null »), ce qui
// affichait « null » à côté des boutons, dans les infobulles et les notifications. h() les ignorait déjà ; ce filet
// fait de même pour tous les appels directs.
for (const proto of [Element.prototype, DocumentFragment.prototype]) {
  for (const method of ['append', 'prepend', 'replaceChildren', 'before', 'after']) {
    const native = proto[method];
    if (typeof native !== 'function') continue;
    proto[method] = function skipEmpty(...nodes) {
      return native.apply(this, nodes.filter((n) => n !== null && n !== undefined && n !== false));
    };
  }
}

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

/** Crée un élément : h('div.class#id', { attrs }, ...enfants). */
export function h(tag, props = {}, ...children) {
  const [, name = 'div', rest = ''] = tag.match(/^([a-z0-9-]*)(.*)$/i);
  const el = document.createElement(name || 'div');
  for (const part of rest.match(/[.#][^.#]+/g) || []) {
    if (part[0] === '.') el.classList.add(part.slice(1));
    else el.id = part.slice(1);
  }
  for (const [k, v] of Object.entries(props || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k === 'html') el.innerHTML = v;
    else if (k === 'text') el.textContent = v;
    else if (k === 'style' && typeof v === 'object') {
      for (const [prop, val] of Object.entries(v)) {
        if (prop.startsWith('--')) el.style.setProperty(prop, String(val));
        else el.style[prop] = val;
      }
    }
    else if (k === 'dataset') Object.assign(el.dataset, v);
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2).toLowerCase(), v);
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

export function getPath(obj, path) {
  return path.split('.').reduce((o, k) => (o == null ? undefined : o[k]), obj);
}

export function patchFor(path, value) {
  const keys = path.split('.');
  const root = {};
  let cur = root;
  keys.forEach((k, i) => {
    if (i === keys.length - 1) cur[k] = value;
    else cur = cur[k] = {};
  });
  return root;
}

export const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

export function debounce(fn, ms) {
  let id;
  return (...args) => {
    clearTimeout(id);
    id = setTimeout(() => fn(...args), ms);
  };
}

export const wait = (ms) => new Promise((r) => setTimeout(r, ms));
