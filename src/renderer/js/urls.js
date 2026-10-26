// Adresses des médias : le protocole app:// dans l'application de bureau, le serveur lui-même sur le web.

const BASE = window.control?.platform === 'web' ? '' : 'app://nilcam';

export const mediaUrl = (id) => `${BASE}/media/${id}`;
export const coverFile = (hash, small = false) => `${BASE}/cover/${hash}${small ? '-s' : ''}.jpg`;
export const videoThumbUrl = (id) => `${BASE}/vthumb/${id}`;
