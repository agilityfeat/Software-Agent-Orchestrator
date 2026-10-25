// Presentation only. LocalModel supplies structured runtime events; no fake timers/progress.
const LOADING = new Set(['preparing', 'downloading', 'cached', 'local', 'initializing']);
const bytes = value => value >= 1e9 ? `${(value / 1e9).toFixed(2)} GB` : `${(value / 1e6).toFixed(1)} MB`;

export function loaderPresentation(event = {}) {
  const phase = event.phase || 'idle';
  const titles = {
    idle: 'A little bot. A whole team waiting to wake up.',
    preparing: 'Gathering the pieces…',
    downloading: 'Building your bot’s brain…',
    cached: 'Found its brain in your browser.',
    local: 'Giving your bot its brain…',
    initializing: 'Bringing your bot to life…',
    ready: 'Gemma is awake. Your team is ready.',
    working: `${event.role || 'Your agent'} is thinking locally…`,
    error: 'Your bot needs a hand.',
    stopped: 'Wake-up stopped. Your bot can try again.',
  };
  const loaded = Number.isFinite(event.loaded) && event.loaded >= 0 ? event.loaded : 0;
  const knownTotal = Number.isFinite(event.total) && event.total > 0 && loaded <= event.total;
  const percent = phase === 'downloading' && knownTotal
    ? Math.min(event.complete ? 100 : 99, Math.floor(loaded / event.total * 100)) : null;
  const detail = phase === 'downloading'
    ? `${bytes(loaded)}${knownTotal ? ` / ${bytes(event.total)}` : ' downloaded'}${percent === null ? '' : ` · ${percent}%`}`
    : phase === 'initializing' ? `${event.source === 'cache' ? 'Browser cache found. ' : event.source === 'file' ? 'Local file selected. ' : ''}Preparing the model on your GPU. Almost awake—not ready yet.`
    : phase === 'ready' ? 'Planner, Coder and Tester can now share this local model. No inference server.'
    : phase === 'idle' ? 'Load Gemma once. Its brain runs here, on your device.'
    : event.text || 'Preparing the local runtime…';
  return {
    phase, title: titles[phase] || titles.idle, detail, percent,
    loading: LOADING.has(phase),
    badge: phase === 'ready' ? '✓ READY' : phase === 'working' ? 'WORKING' : phase === 'error' ? 'NEEDS ATTENTION' : phase === 'stopped' ? 'STOPPED' : phase === 'idle' ? 'ASLEEP' : 'WAKING UP',
    button: LOADING.has(phase) ? 'Waking Gemma…' : ['ready', 'working'].includes(phase) ? '✓ Gemma loaded' : ['error', 'stopped'].includes(phase) ? 'Retry loading Gemma' : 'Load Gemma · ~2 GB',
  };
}

export function mountModelLoader(root = globalThis.document?.querySelector('.runtime-bar')) {
  // Headless consumers can use LocalModel without a DOM. Existing status callbacks remain supported.
  if (!root) return () => {};
  const doc = root.ownerDocument;
  const element = (tag, className, content = '') => {
    const node = doc.createElement(tag);
    node.className = className;
    node.textContent = content;
    return node;
  };
  if (!doc.querySelector('link[data-model-loader]')) {
    const css = doc.createElement('link');
    css.rel = 'stylesheet';
    css.href = new URL('./model-loader.css', import.meta.url).href;
    css.dataset.modelLoader = '';
    doc.head.append(css);
  }
  root.querySelector('.model-awakening')?.remove();
  root.classList.add('has-model-loader');
  const widget = element('section', 'model-awakening');
  widget.setAttribute('aria-label', 'Local Gemma model readiness');
  const scene = element('div', 'bot-scene');
  scene.setAttribute('aria-hidden', 'true');
  const orbit = element('div', 'bot-orbit');
  for (let i = 0; i < 3; i++) orbit.append(element('i', `bot-spark spark-${i}`));
  const robot = element('div', 'little-bot');
  const antenna = element('span', 'bot-antenna');
  antenna.append(element('i', 'bot-antenna-light'));
  const head = element('div', 'bot-head');
  const face = element('div', 'bot-face');
  face.append(element('i', 'bot-eye eye-left'), element('i', 'bot-eye eye-right'), element('i', 'bot-mouth'));
  head.append(antenna, face);
  const body = element('div', 'bot-body');
  body.append(element('i', 'bot-heart'));
  robot.append(head, body, element('span', 'bot-arm arm-left'), element('span', 'bot-arm arm-right'));
  scene.append(element('div', 'bot-halo'), orbit, robot, element('span', 'bot-sleep', 'z z'), element('div', 'bot-platform'));
  const copy = element('div', 'bot-copy');
  const heading = element('div', 'bot-heading');
  const title = element('h2', 'bot-title');
  title.id = 'model-loader-title';
  title.setAttribute('role', 'status');
  title.setAttribute('aria-atomic', 'true');
  const badge = element('span', 'bot-badge');
  heading.append(title, badge);
  const detail = element('p', 'bot-detail');
  detail.id = 'model-loader-detail';
  const track = element('div', 'bot-progress');
  track.setAttribute('role', 'progressbar');
  track.setAttribute('aria-label', 'Gemma model loading');
  track.setAttribute('aria-describedby', detail.id);
  const fill = element('span', 'bot-progress-fill');
  track.append(fill);
  const steps = element('div', 'bot-steps');
  steps.setAttribute('aria-label', 'Loading stages');
  const stepNodes = ['01 · Download', '02 · Wake up', '03 · Ready'].map(value => element('span', 'bot-step', value));
  steps.append(...stepNodes);
  const controls = element('div', 'bot-controls');
  const pause = element('button', 'quiet bot-motion', 'Pause animation');
  pause.type = 'button';
  pause.setAttribute('aria-pressed', 'false');
  pause.onclick = () => {
    const paused = widget.classList.toggle('motion-paused');
    pause.textContent = paused ? 'Resume animation' : 'Pause animation';
    pause.setAttribute('aria-pressed', String(paused));
  };
  const stop = element('button', 'quiet bot-stop', 'Stop loading');
  stop.type = 'button';
  // Reuse the application's existing cancellation path, including worker termination.
  stop.onclick = () => doc.getElementById('cancel')?.click();
  controls.append(pause, stop);
  copy.append(heading, detail, track, steps);
  widget.append(scene, copy, controls);
  root.prepend(widget);
  let previousPhase = 'idle';
  let source = 'download';
  const update = event => {
    if (event?.phase === 'preparing' && ['idle', 'error', 'stopped', 'ready'].includes(previousPhase)) source = 'download';
    if (event?.source) source = event.source;
    const view = loaderPresentation({...event, source});
    widget.dataset.phase = view.phase;
    widget.classList.toggle('is-loading', view.loading);
    widget.classList.toggle('is-online', ['ready', 'working'].includes(view.phase));
    // Celebrate only the initial engine load, not every completed agent message.
    widget.classList.toggle('just-awoke', view.phase === 'ready' && !['ready', 'working'].includes(previousPhase));
    if (title.textContent !== view.title) title.textContent = view.title;
    badge.textContent = view.badge;
    detail.textContent = view.detail;
    track.hidden = !view.loading;
    stop.hidden = !view.loading;
    pause.hidden = !view.loading && !['ready', 'working'].includes(view.phase);
    track.classList.toggle('indeterminate', view.percent === null);
    if (view.percent === null) {
      track.removeAttribute('aria-valuemin');
      track.removeAttribute('aria-valuemax');
      track.removeAttribute('aria-valuenow');
      track.setAttribute('aria-valuetext', view.detail);
    } else {
      track.setAttribute('aria-valuemin', '0');
      track.setAttribute('aria-valuemax', '100');
      track.setAttribute('aria-valuenow', String(view.percent));
      track.removeAttribute('aria-valuetext');
    }
    fill.style.width = view.percent === null ? '30%' : `${view.percent}%`;
    const stage = ['ready', 'working'].includes(view.phase) ? 2 : ['initializing', 'cached', 'local'].includes(view.phase) ? 1 : 0;
    stepNodes.forEach((step, index) => {
      step.classList.toggle('complete', index < stage || stage === 2);
      step.classList.toggle('current', index === stage && (view.loading || stage === 2));
      if (index === stage && (view.loading || stage === 2)) step.setAttribute('aria-current', 'step');
      else step.removeAttribute('aria-current');
    });
    stepNodes[0].textContent = source === 'cache' || view.phase === 'cached' ? '01 · Cached' : source === 'file' || view.phase === 'local' ? '01 · Local file' : '01 · Download';
    const button = doc.getElementById('load-model');
    if (button) button.textContent = view.button; // The app still owns disabled/permission state.
    previousPhase = view.phase;
  };
  update({phase: 'idle'});
  return update;
}
