// Diagnostic only: events never authorize a change or manufacture test evidence.
const LIMIT = 200;
const bounded = (value, size = 6000) => {
  const text = String(value ?? '');
  return text.length <= size ? text : text.slice(0, size) + '\n[truncated in log]';
};
export const seconds = ms => `${(Math.max(0, ms) / 1000).toFixed(1)}s`;

export class RunLog {
  constructor({now = Date.now, onChange = () => {}} = {}) {
    this.now = now; this.onChange = onChange; this.rows = []; this.active = new Set();
    this.taskId = null; this.mode = ''; this.phase = ''; this.cursor = 0; this.modelStep = null;
    this.dropped = 0; this.serial = 0;
  }
  emit() { try { this.onChange(); } catch { /* Logging must not break execution. */ } }
  add(role, message, detail = '', at = this.now()) {
    this.rows.push({id: ++this.serial, at, role: bounded(role, 40), message: bounded(message, 1000), detail: bounded(detail)});
    if (this.rows.length > LIMIT) { this.rows.shift(); this.dropped++; }
    this.emit();
  }
  reset(id = null) {
    this.taskId = id; this.rows = []; this.active.clear(); this.modelStep = null;
    this.cursor = 0; this.phase = ''; this.mode = ''; this.dropped = 0; this.emit();
  }
  observe(task, busy = false) {
    if ((task?.id ?? null) !== this.taskId) this.reset(task?.id ?? null);
    if (!task) return;
    this.mode = task.mode;
    const signature = `${task.status}|${task.phase}|${task.attempt || 0}|${busy}|${task.mode}`;
    if (signature !== this.phase) {
      this.phase = signature;
      // Finished/cancelled work must never leave a ticking diagnostic operation.
      if (!busy && task.status !== 'Active') { this.active.clear(); this.modelStep = null; }
      if (task.status !== 'Active' || !busy) this.summary = `${task.status} · ${task.phase}`;
      else this.summary = `${task.phase}${task.attempt ? ` · attempt ${task.attempt}/3` : ''}`;
      this.emit();
    }
    for (const event of (task.events || []).slice(this.cursor)) {
      this.add(event.role, event.text, '', Number.isFinite(Date.parse(event.at)) ? Date.parse(event.at) : this.now());
    }
    this.cursor = (task.events || []).length;
  }
  begin(role, message) {
    const step = {role, message, started: this.now(), last: this.now(), taskId: this.taskId};
    this.active.add(step); this.add(role, message);
    return step;
  }
  stage(step, message) {
    if (!this.active.has(step)) return;
    step.last = this.now();
    if (message !== step.message) { step.message = message; this.add(step.role, message); }
    else this.emit();
  }
  finish(step, message, detail = '') {
    if (!this.active.delete(step) || step.taskId !== this.taskId) return;
    this.add(step.role, `${message} · ${seconds(this.now() - step.started)}`, detail);
  }
  model(event) {
    if (event.type === 'start') {
      this.modelStep = this.begin(event.role || 'Model', event.operation === 'load' ? 'Loading local model' : `Request sent to local model · ${event.inputChars} input characters`);
    } else if (event.type === 'status' && this.modelStep) {
      const prefix = `${this.modelStep.role}: `;
      this.stage(this.modelStep, event.text.startsWith(prefix) ? event.text.slice(prefix.length) : event.text);
    } else if (event.type === 'complete' && this.modelStep) {
      this.finish(this.modelStep, event.operation === 'load' ? 'Local model ready' : `Response received · ${event.output?.length || 0} characters (not yet validated)`, event.output);
      this.modelStep = null;
    } else if (['error', 'stopped'].includes(event.type)) {
      if (this.modelStep) this.finish(this.modelStep, event.type === 'error' ? 'Model failed' : 'Model stopped', event.message);
      else this.add('Model', event.type === 'error' ? 'Model failed' : 'Model stopped', event.message);
      this.modelStep = null;
    }
  }
  async checks(execute, bundle, checks, options) {
    const step = this.begin('Browser', `Running ${checks.length} browser check${checks.length === 1 ? '' : 's'}`);
    try {
      const result = await execute(bundle, checks, options);
      const rows = result.rows || [], errors = result.errors || [];
      this.finish(step, `${rows.filter(row => row.pass).length}/${rows.length} checks passed; ${errors.length} runtime errors`, rows.map(row => `${row.pass ? 'PASS' : 'FAIL'} · ${row.name}`).join('\n'));
      return result; // Preserve the original evidence, including failures.
    } catch (error) {
      this.finish(step, options?.signal?.aborted ? 'Browser checks stopped' : 'Browser checks failed', String(error.message || error));
      throw error;
    }
  }
  status() {
    const step = [...this.active].at(-1);
    if (!step) return {title: this.taskId ? this.summary || 'No active operation' : 'No active task', detail: this.mode === 'recorded' ? 'Recorded run · historical events, not live execution.' : 'Detailed runtime logs last for this page session.', busy: false};
    return {title: `${step.role} · ${step.message}`, detail: `Elapsed ${seconds(this.now() - step.started)} · Last runtime event ${seconds(this.now() - step.last)} ago. Waiting does not indicate percentage complete.`, busy: true};
  }
  exportText() {
    return [`Agent activity · ${this.mode || 'session'}${this.taskId ? ` · ${this.taskId}` : ''}`, this.dropped ? `${this.dropped} older entries omitted.` : '', ...this.rows.map(row => `${new Date(row.at).toISOString()} [${row.role}] ${row.message}${row.detail ? '\n' + row.detail : ''}`)].filter(Boolean).join('\n');
  }
}

export function mountRunLog(doc = document) {
  const log = new RunLog();
  const make = (tag, content = '', className = '') => { const node = doc.createElement(tag); node.textContent = content; node.className = className; return node; };
  const css = doc.createElement('link'); css.rel = 'stylesheet'; css.href = new URL('./run-log.css', import.meta.url).href; doc.head.append(css);
  const button = make('button', 'Logs', 'quiet run-log-open'); button.type = 'button'; button.id = 'show-run-log'; button.setAttribute('aria-haspopup', 'dialog'); button.setAttribute('aria-controls', 'run-log-dialog');
  doc.querySelector('.builder .panel-heading').append(button);
  const dialog = make('dialog', '', 'run-log-dialog'); dialog.id = 'run-log-dialog'; dialog.setAttribute('aria-labelledby', 'run-log-title');
  const header = make('header'), title = make('h2', 'Agent activity'); title.id = 'run-log-title';
  const close = make('button', 'Close', 'quiet'); close.type = 'button'; close.autofocus = true; close.onclick = () => dialog.close(); header.append(title, close);
  const live = make('div', '', 'run-log-status'), headline = make('strong'), timer = make('p'); headline.setAttribute('role', 'status'); timer.setAttribute('aria-live', 'off'); live.append(headline, timer);
  const note = make('p', 'Runtime events and completed outputs—not internal reasoning or token-by-token streaming. Closing this view does not stop the agents.', 'run-log-note');
  const list = make('div', '', 'run-log-list'); list.tabIndex = 0; list.setAttribute('role', 'region'); list.setAttribute('aria-label', 'Timestamped agent activity');
  const footer = make('footer'), followLabel = make('label'), follow = make('input'); follow.type = 'checkbox'; follow.checked = true; followLabel.append(follow, doc.createTextNode(' Follow latest'));
  const count = make('span', '', 'run-log-count'), copy = make('button', 'Copy log', 'quiet'); copy.type = 'button';
  const feedback = make('span', '', 'run-log-feedback'); feedback.setAttribute('role', 'status');
  copy.onclick = async () => { try { await doc.defaultView.navigator.clipboard.writeText(log.exportText()); feedback.textContent = 'Copied'; } catch { feedback.textContent = 'Copy unavailable; select and copy the text below.'; const range = doc.createRange(); range.selectNodeContents(list); const selection = doc.getSelection(); selection.removeAllRanges(); selection.addRange(range); } };
  footer.append(followLabel, count, feedback, copy); dialog.append(header, live, note, list, footer); doc.body.append(dialog);
  let interval = null, lastId = 0, lastFirst = null;
  const status = () => { const current = log.status(); if (headline.textContent !== current.title) headline.textContent = current.title; timer.textContent = current.detail; dialog.dataset.busy = String(current.busy); };
  const render = () => {
    button.dataset.busy = String(log.active.size > 0); button.title = log.active.size ? 'Open live agent activity' : 'Open agent activity';
    if (!dialog.open) return;
    status();
    const first = log.rows[0]?.id;
    if (first !== lastFirst) { list.replaceChildren(); lastId = 0; lastFirst = first; }
    const fresh = log.rows.filter(row => row.id > lastId);
    for (const row of fresh) {
      const item = make('article', '', 'run-log-row'), meta = make('div', '', 'run-log-meta'), time = make('time', new Date(row.at).toLocaleTimeString()); time.dateTime = new Date(row.at).toISOString();
      meta.append(time, make('b', row.role)); item.append(meta, make('p', row.message));
      if (row.detail) { const details = make('details'), summary = make('summary', 'View output / details'); details.append(summary, make('pre', row.detail)); item.append(details); }
      list.append(item); lastId = row.id;
    }
    if (!log.rows.length) list.textContent = 'No activity yet. Start a plan or load the local model.';
    count.textContent = `${log.rows.length} events${log.dropped ? ` · ${log.dropped} older omitted` : ''}${log.mode ? ` · ${log.mode.toUpperCase()}` : ''}`;
    if (fresh.length && follow.checked) list.scrollTop = list.scrollHeight;
  };
  log.onChange = render;
  button.onclick = () => { if (dialog.open) return; dialog.showModal(); feedback.textContent = ''; render(); interval = setInterval(status, 500); };
  dialog.addEventListener('close', () => { clearInterval(interval); interval = null; button.focus(); });
  dialog.addEventListener('click', event => { if (event.target === dialog) { const rect = dialog.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) dialog.close(); } });
  follow.onchange = () => { if (follow.checked) list.scrollTop = list.scrollHeight; };
  return log;
}
