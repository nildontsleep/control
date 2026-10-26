// Version web : ce qui change par rapport à l'application de bureau, et ce que le compte connecté a le droit de faire.

export const isWeb = window.control?.platform === 'web';
const caps = window.control?.env?.caps || {};

/** Sur le bureau tout est permis ; sur le web, selon les droits du compte (editFiles, download, manageServer…). */
export const can = (key) => !isWeb || !!caps[key];
