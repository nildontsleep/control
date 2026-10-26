// Version web : ce qui change par rapport à l'application de bureau, et ce que le compte connecté a le droit de faire.

export const isWeb = window.control?.platform === 'web';
const caps = window.control?.env?.caps || {};

/** Sur le bureau tout est permis ; sur le web, selon les droits du compte (editFiles, download, manageServer…). */
export const can = (key) => !isWeb || !!caps[key];

/** Bibliothèque d'un autre compte, parcourue en lecture seule (web) : { id, username, name, avatar, color }, sinon null. */
export const viewing = isWeb ? window.control.web.viewing || null : null;
export const readOnly = !!viewing;
