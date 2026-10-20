// Vista de ganancias (fase 3): tabla por componente con costo sin IVA,
// ganancia e IVA; presets 10/20/30% (+ personalizado) y precio objetivo
// que reparte la diferencia equitativamente ajustando la ganancia.
import { api } from '../api';
import { Assembly, ProfitReport } from '../types';
import { el, mxn, typeChip } from '../ui';

export interface ProfitViewDeps {
  getSelectedAssembly: () => Assembly | null;
  /** Parámetros compartidos con la vista Cotización. */
  getParams: () => { percent: string; target: string };
  setParams: (p: { percent: string; target: string }) => void;
}

const PRESETS = ['10', '20', '30'];

export function createProfitView(deps: ProfitViewDeps): HTMLElement {
  const view = el('section', { class: 'view profit-view' });
  const percent = () => deps.getParams().percent;
  let report: ProfitReport | null = null;
  let error = '';

  function setPercent(v: string) {
    deps.setParams({ ...deps.getParams(), percent: v });
  }

  function setTarget(v: string) {
    deps.setParams({ ...deps.getParams(), target: v });
  }

  const selected = () => {
    const a = deps.getSelectedAssembly();
    return a && a.confirmed_at ? a : null;
  };

  async function load() {
    error = '';
    const asm = selected();
    if (!asm) {
      report = null;
      render();
      return;
    }
    try {
      const { percent, target } = deps.getParams();
      report = await api.assemblyProfit(
        asm.id,
        percent,
        target.trim() === '' ? null : target.trim(),
      );
    } catch (e) {
      error = String(e);
      report = null;
    }
    render();
  }

  function fmt(n: string): string {
    return mxn(n);
  }

  function signed(n: string): string {
    const v = parseFloat(n);
    return `${v > 0 ? '+' : ''}${mxn(n)}`;
  }

  function renderControls() {
    const presetBtns = PRESETS.map((p) =>
      el('button', {
        class: `btn ${percent() === p ? 'btn-primary' : 'btn-outline'}`,
        onclick: () => {
          setPercent(p);
          load();
        },
      }, `${p}%`),
    );

    const customInput = el('input', {
      class: 'input',
      type: 'number',
      min: '0',
      step: '0.01',
      placeholder: 'Otro %',
      value: percent() && !PRESETS.includes(percent()) ? percent() : '',
    }) as HTMLInputElement;
    customInput.addEventListener('change', () => {
      const v = customInput.value.trim();
      if (v === '') return;
      const num = parseFloat(v);
      if (!isNaN(num) && num >= 0) {
        setPercent(v);
        load();
      }
    });

    const target = el('input', {
      class: 'input',
      type: 'number',
      min: '0',
      step: '0.01',
      placeholder: 'Precio objetivo (total con IVA)',
      value: deps.getParams().target,
    }) as HTMLInputElement;
    target.addEventListener('change', () => {
      setTarget(target.value);
      load();
    });
    const applyTarget = el('button', { class: 'btn btn-outline' }, 'Aplicar');
    applyTarget.addEventListener('click', () => {
      setTarget(target.value);
      load();
    });
    const clearTarget = el('button', {
      class: 'icon-btn',
      title: 'Quitar precio objetivo',
    }, '✕');
    clearTarget.addEventListener('click', () => {
      setTarget('');
      target.value = '';
      load();
    });

    return el('div', { class: 'toolbar profit-toolbar' },
      el('span', { class: 'muted' }, 'Ganancia:'),
      ...presetBtns,
      customInput,
      el('span', { class: 'toolbar-sep' }),
      target,
      applyTarget,
      clearTarget,
    );
  }

  function renderTable(r: ProfitReport) {
    const hasTarget = r.difference !== null;
    const header = el('tr', {},
      el('th', {}, 'Componente'),
      el('th', { class: 'num' }, 'Cant.'),
      el('th', { class: 'num' }, 'Costo (s/IVA)'),
      el('th', { class: 'num' }, `Ganancia (${r.percent}%)`),
      hasTarget ? el('th', { class: 'num' }, 'Ajuste') : null,
      el('th', { class: 'num' }, 'IVA (16%)'),
      el('th', { class: 'num' }, 'Precio'),
    );

    const rows = r.lines.map((l) =>
      el('tr', {},
        el('td', {},
          el('div', { class: 'desc', title: l.description }, l.description),
          el('div', { class: 'sub' }, l.sku),
        ),
        el('td', { class: 'num' }, `×${l.quantity}`),
        el('td', { class: 'num' }, fmt(l.cost)),
        el('td', { class: 'num' }, fmt(hasTarget ? l.final_profit : l.base_profit)),
        hasTarget
          ? el('td', {
              class: `num adj ${parseFloat(l.adjustment) < 0 ? 'adj-neg' : 'adj-pos'}`,
            }, signed(l.adjustment))
          : null,
        el('td', { class: 'num' }, fmt(l.iva)),
        el('td', { class: 'num strong' }, fmt(l.price)),
      ),
    );

    const t = r.totals;
    const totals = el('tr', { class: 'totals-row' },
      el('td', { class: 'strong' }, 'Sumas'),
      el('td', {}, ''),
      el('td', { class: 'num strong' }, fmt(t.cost)),
      el('td', { class: 'num strong' }, fmt(hasTarget ? t.final_profit : t.base_profit)),
      hasTarget
        ? el('td', {
            class: `num strong ${parseFloat(t.adjustment) < 0 ? 'adj-neg' : 'adj-pos'}`,
          }, signed(t.adjustment))
        : null,
      el('td', { class: 'num strong' }, fmt(t.iva)),
      el('td', { class: 'num strong grand-total' }, fmt(t.price)),
    );

    return el('div', { class: 'table-wrap' },
      el('table', { class: 'catalog-table profit-table' },
        el('thead', {}, header),
        el('tbody', {}, ...rows, totals),
      ),
    );
  }

  function renderTargetSummary(r: ProfitReport) {
    if (r.difference === null) return null;
    const diff = parseFloat(r.difference);
    return el('div', {
      class: `target-banner ${diff < 0 ? 'target-neg' : 'target-pos'}`,
    },
      el('div', {},
        `Objetivo: ${mxn(r.target ?? '0')} · Diferencia: ${signed(r.difference)} ` +
        `(${diff < 0 ? 'restando' : 'añadiendo'} ${mxn(String(Math.abs(parseFloat(r.adjustment_per_line ?? '0'))))} por componente)`),
      diff < 0
        ? el('div', { class: 'sub' }, 'La ganancia se redujo; con este objetivo puede ser negativa.')
        : null,
    );
  }

  function render() {
    const asm = selected();
    if (!asm) {
      view.replaceChildren(
        el('div', { class: 'panel-muted' },
          'Confirma un ensamble completo en la pestaña Ensamble para calcular ganancias.'),
      );
      return;
    }
    if (error) {
      view.replaceChildren(el('p', { class: 'form-error' }, error));
      return;
    }
    if (!report) {
      view.replaceChildren(el('p', { class: 'muted' }, 'Calculando…'));
      return;
    }
    view.replaceChildren(
      el('h2', { class: 'view-title' },
        `Ganancias — ${asm.name}`,
        typeChip(asm.quote_type)),
      renderControls(),
      renderTargetSummary(report) ?? el('span'),
      renderTable(report),
      el('p', { class: 'muted small' },
        'El IVA se aplica una sola vez sobre (costo + ganancia base). El precio objetivo ajusta solo la ganancia.'),
    );
  }

  load();
  return view;
}
