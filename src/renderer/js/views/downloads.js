// Vue « Téléchargements » : outils yt-dlp, analyse d’un lien, file des téléchargements.

import { h } from '../dom.js';
import { icon } from '../icons.js';
import { t } from '../i18n.js';
import { fmtTime, fmtBytes } from '../format.js';
import { settings, update, subscribe as onSettings } from '../settings.js';
import { navigate } from '../router.js';
import { player } from '../player.js';
import { lib } from '../data.js';
import { segmented, popup, toggle, button } from '../ui/controls.js';
import { toast } from '../ui/feedback.js';
import { pageHead } from './shared.js';

const api = window.control.downloads;
const tools = window.control.tools;

const AUDIO_FORMATS = [
  { value: 'mp3', label: 'MP3' },
  { value: 'm4a', label: 'M4A (AAC)' },
  { value: 'opus', label: 'Opus' },
  { value: 'flac', label: 'FLAC' },
  { value: 'best', label: 'Original' },
];
const AUDIO_QUALITIES = ['320', '256', '192', '128'].map((v) => ({ value: v, label: `${v} kb/s` }));
const VIDEO_QUALITIES = [
  { value: 'best', label: 'Max' }, { value: '2160', label: '4K' }, { value: '1440', label: '1440p' },
  { value: '1080', label: '1080p' }, { value: '720', label: '720p' }, { value: '480', label: '480p' },
];

const dlBinding = (key) => ({
  get: () => settings.dl[key],
  set: (v) => update({ dl: { [key]: v } }),
  def: undefined,
});

/** Traduit les erreurs courantes de yt-dlp en conseils compréhensibles. */
export function friendlyError(raw) {
  const text = String(raw || '');
  if (/confirm you.?re not a bot|sign in to confirm|HTTP Error 429/i.test(text)) return t('dl.errBot');
  if (/confirm your age|age.restricted|inappropriate for some users/i.test(text)) return t('dl.errAge');
  if (/video unavailable|is unavailable|private video|removed|not available in your country|blocked it/i.test(text)) return t('dl.errUnavailable');
  if (/unable to download|getaddrinfo|timed out|network|connection/i.test(text)) return t('dl.errNetwork');
  return text;
}

function jobRow(job) {
  const pct = Math.round((job.progress || 0) * 100);
  const el = h('div.dl-job', { dataset: { id: job.id, status: job.status } });
  const meta = () => {
    if (job.status === 'downloading') {
      const parts = [];
      if (job.count > 1 && job.index) parts.push(t('dl.itemOf', { i: job.index, n: job.count }));
      if (job.speed) parts.push(`${fmtBytes(job.speed)}/s`);
      if (job.eta) parts.push(t('dl.eta', { t: fmtTime(job.eta) }));
      return parts.join(' · ') || t('dl.starting');
    }
    if (job.status === 'processing') return t('dl.processing');
    if (job.status === 'queued') return t('dl.queued');
    if (job.status === 'done') return job.skipped ? t('dl.partial', { n: job.skipped, reason: friendlyError(job.error) }) : t('dl.doneTracks', { n: job.files.length });
    if (job.status === 'canceled') return t('dl.canceled');
    return friendlyError(job.error) || t('dl.failed');
  };
  const blank = () => h('div.dl-thumb.blank', { html: icon(job.kind === 'video' ? 'film' : 'music-2', 20, 1.6) });
  const thumb = job.thumbnail ? h('img.dl-thumb', { src: job.thumbnail, alt: '', referrerpolicy: 'no-referrer' }) : blank();
  thumb.addEventListener('error', () => thumb.replaceWith(blank()));
  const actions = [];
  if (['queued', 'downloading', 'processing'].includes(job.status)) actions.push(h('button.icon-btn', { type: 'button', 'data-tip': t('common.cancel'), html: icon('x', 16, 2), onclick: () => api.cancel(job.id) }));
  if (['error', 'canceled'].includes(job.status)) actions.push(h('button.icon-btn', { type: 'button', 'data-tip': t('dl.retry'), html: icon('refresh-cw', 15, 2), onclick: () => api.retry(job.id) }));
  if (job.status === 'done' && job.trackIds?.length) {
    actions.push(h('button.icon-btn', {
      type: 'button',
      'data-tip': t('common.play'),
      html: icon('play', 15, 0),
      onclick: () => {
        const tracks = job.trackIds.map((id) => lib.byId.get(id)).filter(Boolean);
        if (tracks.length) player.playQueue(tracks, 0, { shuffle: false });
        else toast(t('dl.notYetInLibrary'), 'info');
      },
    }));
    if (job.playlistId) actions.push(h('button.icon-btn', { type: 'button', 'data-tip': t('dl.openPlaylist'), html: icon('list-music', 15, 2), onclick: () => navigate('playlist', job.playlistId) }));
  }
  if (!['queued', 'downloading', 'processing'].includes(job.status)) actions.push(h('button.icon-btn', { type: 'button', 'data-tip': t('common.remove'), html: icon('trash-2', 15, 2), onclick: () => api.remove(job.id) }));

  el.append(thumb, h('div.dl-main', {},
    h('div.dl-title', { text: job.current || job.title }),
    h('div.dl-sub', { text: job.current && job.current !== job.title && job.isPlaylist ? `${job.title} · ${meta()}` : meta() }),
    ['downloading', 'processing', 'queued'].includes(job.status)
      ? h('div.dl-bar', {}, h('span', { style: { width: `${job.status === 'processing' ? 100 : pct}%` } }))
      : null),
  h('div.dl-status', {}, job.status === 'done' ? h('span.ok', { html: icon('circle-check', 18, 2) }) : job.status === 'error' ? h('span.err', { html: icon('circle-alert', 18, 2) }) : job.status === 'downloading' ? h('span.pct', { text: `${pct}%` }) : null),
  h('div.dl-actions', {}, ...actions));
  if (job.status === 'processing') el.classList.add('indeterminate');
  return el;
}

export function downloadsView(root) {
  let jobs = [];
  let info = null;
  let toolStatus = null;
  let installing = null;
  const offs = [];

  const urlInput = h('input.url-input', { type: 'text', placeholder: t('dl.placeholder'), spellcheck: 'false', autocomplete: 'off' });
  const infoBox = h('div.dl-info');
  const toolsBox = h('div.dl-tools');
  const queueBox = h('div.dl-queue');
  const optionsBox = h('div.dl-options');

  // ─── Outils ────────────────────────────────────────────────────────────────
  function renderTools() {
    if (!toolStatus) return toolsBox.replaceChildren();
    const ready = toolStatus.ytdlp.installed && toolStatus.ffmpeg.installed;
    toolsBox.classList.toggle('ready', ready);
    if (installing) {
      const labels = { ytdlp: 'yt-dlp', ffmpeg: 'ffmpeg', deno: 'deno' };
      const pct = installing.total ? Math.round((installing.received / installing.total) * 100) : null;
      toolsBox.replaceChildren(h('div.tools-card.busy', {},
        h('div.tools-icon', { html: icon('cloud-download', 22, 1.7) }),
        h('div.tools-text', {},
          h('strong', { text: t('tools.installing', { tool: labels[installing.tool] || installing.tool }) }),
          h('span', { text: installing.phase === 'extract' ? t('tools.extracting') : pct !== null ? `${fmtBytes(installing.received)} / ${fmtBytes(installing.total)}` : t('tools.preparing') }),
          h('div.dl-bar', {}, h('span', { style: { width: `${pct ?? 100}%` } })))));
      return;
    }
    if (!ready) {
      toolsBox.replaceChildren(h('div.tools-card', {},
        h('div.tools-icon', { html: icon('wrench', 22, 1.7) }),
        h('div.tools-text', {}, h('strong', { text: t('tools.needTitle') }), h('span', { text: t('tools.needText') })),
        button(t('tools.install'), { icon: 'arrow-down-to-line', kind: 'primary', onClick: install })));
      return;
    }
    toolsBox.replaceChildren(h('div.tools-line', {},
      h('span.ok', { html: icon('circle-check', 15, 2.2) }),
      h('span', { text: t('tools.ready', { v: toolStatus.ytdlp.version || '' }) }),
      !toolStatus.deno.installed && toolStatus.deno.supported ? button(t('tools.addDeno'), { small: true, kind: 'plain', onClick: install }) : null,
      button(t('tools.update'), { small: true, kind: 'plain', icon: 'refresh-cw', onClick: async () => {
        toast(t('tools.updating'), 'refresh-cw');
        const r = await tools.update();
        toast(r.error ? t('tools.updateFailed') : r.updated ? t('tools.updated', { v: r.version }) : t('tools.upToDate'), r.error ? 'triangle-alert' : 'check');
      } })));
  }

  async function install() {
    installing = { tool: 'ytdlp', phase: 'download', received: 0, total: 0 };
    renderTools();
    const result = await tools.install();
    installing = null;
    toolStatus = await tools.status();
    renderTools();
    if (!result.ok) toast(result.error || t('tools.failed'), 'triangle-alert');
    else toast(t('tools.installed'), 'circle-check');
    if (urlInput.value.trim()) analyze();
  }

  // ─── Analyse d'un lien ─────────────────────────────────────────────────────
  async function analyze() {
    const url = urlInput.value.trim();
    info = null;
    if (!/^https?:\/\//i.test(url)) {
      renderInfo(url ? { error: t('dl.invalidUrl') } : null);
      return;
    }
    if (!toolStatus?.ytdlp.installed) {
      renderInfo({ error: t('dl.needTools') });
      return;
    }
    renderInfo({ loading: true });
    const res = await api.info(url);
    if (urlInput.value.trim() !== url) return;
    if (!res.ok) {
      renderInfo({ error: friendlyError((res.error || '').split('\n').pop()) || t('dl.analyzeFailed') });
      return;
    }
    info = { ...res.info, url };
    // La musique d'une chaîne « Topic » a déjà ses tags : pas besoin de découper le titre.
    renderInfo(info);
  }

  function renderInfo(state) {
    optionsBox.hidden = !(state && state.type);
    if (!state) return infoBox.replaceChildren();
    if (state.loading) return infoBox.replaceChildren(h('div.dl-hint', {}, h('span.spinner.small', {}, ...Array.from({ length: 12 }, (_, i) => h('i', { style: { '--i': i } }))), h('span', { text: t('dl.analyzing') })));
    if (state.error) return infoBox.replaceChildren(h('div.dl-hint.error', {}, h('span.hint-icon', { html: icon('circle-alert', 15, 2) }), h('span', { text: state.error })));
    const isPl = state.type === 'playlist';
    const blank = () => h('div.info-thumb.blank', { html: icon('music-2', 26, 1.5) });
    const thumb = state.thumbnail ? h('img.info-thumb', { src: state.thumbnail, alt: '', referrerpolicy: 'no-referrer' }) : blank();
    thumb.addEventListener?.('error', () => thumb.replaceWith(blank()));
    const card = h('div.info-card', {},
      thumb,
      h('div.info-text', {},
        h('div.info-kind', { text: isPl ? t('dl.playlistKind', { n: state.count }) : t('dl.videoKind') }),
        h('div.info-title', { text: state.title }),
        h('div.info-sub', { text: [state.uploader, !isPl && state.duration ? fmtTime(state.duration) : null].filter(Boolean).join(' · ') })));
    infoBox.replaceChildren(card);
    optionsBox.hidden = false;
    renderOptions();
  }

  function renderOptions() {
    const isPl = info?.type === 'playlist';
    const audio = settings.dl.kind === 'audio';
    optionsBox.replaceChildren(...[
      segmented(dlBinding('kind'), [{ value: 'audio', label: t('dl.audio'), icon: 'music-2' }, { value: 'video', label: t('dl.video'), icon: 'film' }], { compact: true }),
      audio ? popup(dlBinding('audioFormat'), AUDIO_FORMATS) : null,
      audio && settings.dl.audioFormat === 'mp3' ? popup(dlBinding('audioQuality'), AUDIO_QUALITIES) : null,
      !audio ? popup(dlBinding('videoQuality'), VIDEO_QUALITIES) : null,
      isPl ? h('label.opt-check', {}, toggle(dlBinding('createPlaylist')), h('span', { text: t('dl.createPlaylist') })) : null,
      h('span.spacer'),
      button(isPl ? t('dl.downloadAll', { n: info.count }) : t('dl.download'), { icon: 'arrow-down-to-line', kind: 'primary', onClick: start }),
    ].filter(Boolean));
  }

  async function start() {
    if (!info) return;
    await api.add({
      url: info.url,
      title: info.title,
      thumbnail: info.thumbnail,
      isPlaylist: info.type === 'playlist',
      count: info.count || 1,
      kind: settings.dl.kind,
      format: settings.dl.audioFormat,
      quality: settings.dl.kind === 'audio' ? settings.dl.audioQuality : settings.dl.videoQuality,
      parseTitle: !info.music,
      createPlaylist: info.type === 'playlist' && settings.dl.createPlaylist,
      playlistName: info.title,
    });
    toast(t('dl.added'), 'arrow-down-to-line');
    urlInput.value = '';
    info = null;
    renderInfo(null);
    urlInput.focus();
  }

  // ─── File ──────────────────────────────────────────────────────────────────
  function renderQueue() {
    const active = jobs.filter((j) => ['queued', 'downloading', 'processing'].includes(j.status));
    const finished = jobs.filter((j) => !['queued', 'downloading', 'processing'].includes(j.status)).reverse();
    if (!jobs.length) {
      queueBox.replaceChildren(h('div.dl-empty', {}, h('span', { html: icon('cloud-download', 28, 1.4) }), h('p', { text: t('dl.emptyQueue') })));
      return;
    }
    queueBox.replaceChildren(...[
      active.length ? h('h2.block-title', { text: t('dl.inProgress') }) : null,
      ...active.map(jobRow),
      finished.length ? h('div.block-head', {}, h('h2.block-title', { text: t('dl.finished') }), button(t('dl.clear'), { small: true, kind: 'plain', onClick: () => api.clear() })) : null,
      ...finished.map(jobRow),
    ].filter(Boolean));
  }

  function patchJob(job) {
    const i = jobs.findIndex((j) => j.id === job.id);
    if (i < 0) jobs.push({ ...job });
    else jobs[i] = { ...job };
    const existing = queueBox.querySelector(`.dl-job[data-id="${job.id}"]`);
    const sameGroup = existing && (['queued', 'downloading', 'processing'].includes(existing.dataset.status) === ['queued', 'downloading', 'processing'].includes(job.status));
    if (existing && sameGroup && existing.dataset.status === job.status) existing.replaceWith(jobRow(job));
    else renderQueue();
  }

  urlInput.addEventListener('input', () => {
    clearTimeout(urlInput.timer);
    urlInput.timer = setTimeout(analyze, 450);
  });
  urlInput.addEventListener('paste', () => setTimeout(analyze, 60));
  urlInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      clearTimeout(urlInput.timer);
      if (info) start();
      else analyze();
    }
  });

  root.append(
    pageHead({ title: t('nav.downloads'), subtitle: t('dl.subtitle'), actions: button(t('dl.openFolder'), { icon: 'folder-open', onClick: () => api.openDir() }) }),
    h('div.dl-composer', {}, h('div.url-field', {}, h('span.url-icon', { html: icon('link-2', 18, 1.8) }), urlInput,
      h('button.url-paste', { type: 'button', 'data-tip': t('dl.paste'), html: icon('clipboard-paste', 17, 1.8), onclick: async () => {
        try {
          urlInput.value = (await navigator.clipboard.readText()).trim();
          analyze();
        } catch { urlInput.focus(); }
      } })), infoBox, optionsBox),
    toolsBox,
    queueBox,
    h('div.page-foot'),
  );
  optionsBox.hidden = true;

  api.list().then((list) => { jobs = list; renderQueue(); });
  tools.status().then((s) => { toolStatus = s; renderTools(); });
  offs.push(onSettings((patch) => { if ('dl' in patch && info) renderOptions(); }));
  offs.push(api.onUpdate(patchJob));
  offs.push(api.onReset((list) => { jobs = list; renderQueue(); }));
  offs.push(tools.onStatus((s) => { toolStatus = s; renderTools(); }));
  offs.push(tools.onProgress((p) => {
    if (p.phase === 'done' || p.phase === 'error') return;
    installing = { ...(installing || {}), ...p };
    renderTools();
  }));
  setTimeout(() => urlInput.focus(), 120);
  return { destroy: () => offs.forEach((off) => off()) };
}
