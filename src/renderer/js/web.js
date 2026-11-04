// Version web : ce qui change par rapport à l'application de bureau, et ce que le compte connecté a le droit de faire.

export const isWeb = window.control?.platform === 'web';
const caps = window.control?.env?.caps || {};

/** Sur le bureau tout est permis ; sur le web, selon les droits du compte (editFiles, download, manageServer…). */
export const can = (key) => !isWeb || !!caps[key];

/** Bibliothèque d'un autre compte, parcourue en lecture seule (web) : { id, username, name, avatar, color }, sinon null. */
export const viewing = isWeb ? window.control.web.viewing || null : null;
export const readOnly = !!viewing;

// ─── Téléphones et tablettes ─────────────────────────────────────────────────
// iPadOS se présente comme un Mac : on le reconnaît à son écran tactile. Un iPad avec trackpad reste un iPad (le
// pointeur y est « fin », mais la lecture écran éteint obéit aux mêmes règles qu'un iPhone).
const ua = navigator.userAgent || '';
export const isIOS = /iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
export const isAndroid = /Android/.test(ua);
/** Version web sur téléphone ou tablette : la lecture écran éteint passe avant les effets audio. */
export const isMobile = isWeb && (isIOS || isAndroid || (typeof matchMedia === 'function' && matchMedia('(hover: none) and (pointer: coarse)').matches));
