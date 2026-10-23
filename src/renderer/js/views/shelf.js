// « Tout voir » : une rangée en grille, chargée page par page (défilement infini).

import { h } from '../dom.js';
import { t } from '../i18n.js';
import { media } from '../media/api.js';
import { posterCard, errorState } from '../media/ui.js';
import { pageHead } from './shared.js';

export function shelfView(root, param, { scroller }) {
  const [source, title = ''] = String(param || '').split('|');
  let page = 0;
  let totalPages = 1;
  let loading = false;
  let destroyed = false;
  let observer = null;
  const grid = h('div.grid.posters');
  const foot = h('div.shelf-foot');
  const sentinel = h('div.sentinel');
  const showKind = source === 'tmdb:trending';

  root.append(pageHead({ title: title || t('shelf.all') }), grid, sentinel, foot, h('div.page-foot'));

  async function more() {
    if (loading || destroyed || page >= totalPages) return;
    loading = true;
    foot.replaceChildren(h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))));
    try {
      const res = await media.list(source, page + 1);
      if (destroyed) return;
      page = res.page;
      totalPages = res.totalPages;
      grid.append(...res.items.map((item) => posterCard(item, { showKind })));
      foot.replaceChildren();
      if (page >= totalPages) {
        sentinel.remove();
        observer?.disconnect();
      }
    } catch (err) {
      if (destroyed) return;
      foot.replaceChildren(errorState(err, () => { foot.replaceChildren(); loading = false; more(); }));
      observer?.disconnect();
    } finally {
      loading = false;
    }
  }

  observer = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) more();
  }, { root: scroller, rootMargin: '600px 0px' });
  observer.observe(sentinel);
  more();
  return { destroy: () => { destroyed = true; observer?.disconnect(); } };
}
