// Adresses des médias : le protocole app:// dans l'application de bureau, le serveur lui-même sur le web.

const BASE = window.control?.platform === 'web' ? '' : 'app://nilcam';

// Sur le web, dans la bibliothèque d'un autre compte, les fichiers se demandent avec ?lib=.
const LIB = window.control?.web?.viewing ? `?lib=${encodeURIComponent(window.control.web.viewing.id)}` : '';

export const mediaUrl = (id) => `${BASE}/media/${id}${LIB}`;
export const coverFile = (hash, small = false) => `${BASE}/cover/${hash}${small ? '-s' : ''}.jpg`;
export const videoThumbUrl = (id) => `${BASE}/vthumb/${id}${LIB}`;
export const avatarUrl = (hash) => (hash ? `${BASE}/avatar/${hash}.jpg` : null);
