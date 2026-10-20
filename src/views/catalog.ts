// Vista del catálogo: exploración, búsqueda y alta manual de usados.
import { api, NewManualComponent } from '../api';
import { kindLabel, PICKABLE_KINDS } from '../meta';
import { ComponentDto } from '../types';
import { closeModal, el, modal, money, mxn } from '../ui';

interface CatalogState {
  components: ComponentDto[];
  filterKind: string; // '' = todos los armables
  query: string;
}

export function createCatalogView(
  components: ComponentDto[],
  onChanged: (components: ComponentDto[]) => void,
): HTMLElement {
  const state: CatalogState = { components, filterKind: '', query: '' };

  const view = el('section', { class: 'view catalog-view' });

  function specBadges(c: ComponentDto): string[] {
    const s = c.specs ?? {};
    const out: string[] = [];
    if (s.socket) out.push(s.socket);
    if (s.includes_cooler === true) out.push('trae disipador');
    if (s.includes_cooler === false) out.push('sin disipador');
    if (s.wattage) out.push(`${s.wattage}W`);
    if (s.psu_watts) out.push(`PSU ${s.psu_watts}W incluida`);
    if (s.includes_psu === true && !s.psu_watts) out.push('PSU incluida');
    if (s.ddr) out.push(s.ddr);
    if (s.capacity_gb && (c.kind === 'ram')) out.push(`${s.capacity_gb}GB`);
    if (s.capacity_raw) out.push(s.capacity_raw);
    if (s.ram_form && c.kind === 'ram') out.push(s.ram_form);
    if (s.vram) out.push(s.vram);
    if (s.inches) out.push(`${s.inches}"`);
    return out;
  }

  function supplierLabel(supplier: string): string {
    if (supplier === 'cva') return 'CVA';
    if (supplier === 'arroba') return '@';
    if (supplier === 'manual') return 'MANUAL';
    return supplier.toUpperCase();
  }

  /** Desglose de inventario por proveedor, con la regla de cada flujo:
   * ensamble usa la existencia de prioridad; venta individual suma todo. */
  function inventoryBreakdown(c: ComponentDto): HTMLElement {
    const offers = c.offers?.length
      ? c.offers
      : [{ supplier: c.supplier, stock_ensamble: c.stock_ensamble_total, stock_individual: c.stock_individual_total, price: c.price }];
    const rows = offers.map((o) => {
      const label = supplierLabel(o.supplier);
      const detail = o.supplier === 'cva'
        ? `ensamble ${o.stock_ensamble} · venta ${o.stock_individual}`
        : `${o.stock_ensamble} (CEN/AVE-2/CEDIS)`;
      return el('div', { class: 'inv-line' },
        el('span', { class: `chip chip-supplier supplier-${o.supplier}` }, label),
        el('span', { class: 'inv-detail' }, detail),
      );
    });
    return el('div', { class: 'inv-breakdown' },
      ...rows,
      el('div', { class: 'inv-total sub' },
        `Ensamble: ${c.stock_ensamble_total} · Venta: ${c.stock_individual_total}`),
    );
  }

  function filtered(): ComponentDto[] {
    const q = state.query.trim().toUpperCase();
    return state.components.filter((c) => {
      if (state.filterKind === '') {
        if (c.kind === 'other') return false;
      } else if (c.kind !== state.filterKind) return false;
      if (!q) return true;
      return (
        c.sku.toUpperCase().includes(q) ||
        c.description.toUpperCase().includes(q) ||
        c.brand.toUpperCase().includes(q)
      );
    });
  }

  function renderTable() {
    const rows = filtered();
    const table = el('table', { class: 'catalog-table' },
      el('thead', {},
        el('tr', {},
          el('th', {}, 'Tipo'),
          el('th', {}, 'Descripción'),
          el('th', {}, 'Marca'),
          el('th', { class: 'col-cond' }, 'Cond.'),
          el('th', { class: 'num' }, 'Mejor precio'),
          el('th', { class: 'num' }, 'En pesos'),
          el('th', {}, 'Inventario'),
        ),
      ),
      el('tbody', {},
        ...rows.slice(0, 400).map((c) =>
          el('tr', { class: c.condition === 'usado' ? 'row-used' : '' },
            el('td', {}, el('span', { class: `chip chip-${c.kind}` }, kindLabel(c.kind))),
            el('td', {},
              el('div', { class: 'desc' },
                c.description,
                c.on_promotion ? el('span', { class: 'chip chip-promo' }, 'PROMO') : null,
                c.source === 'manual' ? el('span', { class: 'chip chip-manual' }, 'MANUAL') : null,
              ),
              el('div', { class: 'sub' }, [
                c.sku,
                ...specBadges(c),
              ].join(' · ')),
            ),
            el('td', {}, c.brand),
            el('td', { class: 'col-cond' }, c.condition === 'usado' ? 'Usado' : 'Nuevo'),
            el('td', { class: 'num' },
              el('span', { class: 'strong' }, money(c.price)),
              el('span', { class: `chip chip-supplier supplier-${c.supplier}` }, supplierLabel(c.supplier)),
            ),
            el('td', { class: 'num strong' }, mxn(c.price_mxp.amount)),
            el('td', {}, inventoryBreakdown(c)),
          ),
        ),
      ),
    );
    const info = el('p', { class: 'muted' },
      `Mostrando ${Math.min(rows.length, 400)} de ${rows.length} componentes.`);
    const wrap = el('div', { class: 'table-wrap' });
    wrap.replaceChildren(table, info);
    return wrap;
  }

  function renderManualForm() {
    const kindSel = el('select', { class: 'input' },
      ...PICKABLE_KINDS.map((k) =>
        el('option', { value: k }, kindLabel(k)),
      ),
    ) as HTMLSelectElement;

    const desc = el('input', { class: 'input', placeholder: 'Descripción del componente' }) as HTMLInputElement;
    const sku = el('input', { class: 'input', placeholder: 'SKU (opcional)' }) as HTMLInputElement;
    const brand = el('input', { class: 'input', placeholder: 'Marca (opcional)' }) as HTMLInputElement;
    const price = el('input', { class: 'input', placeholder: 'Precio', type: 'number', min: '0', step: '0.01' }) as HTMLInputElement;
    const currency = el('select', { class: 'input' },
      el('option', { value: 'MXP' }, 'Pesos (MXP)'),
      el('option', { value: 'USD' }, 'Dólares (USD)'),
    ) as HTMLSelectElement;
    const condition = el('select', { class: 'input' },
      el('option', { value: 'usado' }, 'Usado'),
      el('option', { value: 'nuevo' }, 'Nuevo'),
    ) as HTMLSelectElement;

    // Especificaciones rápidas según el tipo
    const extra = el('div', { class: 'form-row' });
    let specInputs = renderSpecInputs(kindSel.value);
    extra.replaceChildren(...specInputs.nodes);
    kindSel.addEventListener('change', () => {
      specInputs = renderSpecInputs(kindSel.value);
      extra.replaceChildren(...specInputs.nodes);
    });

    const errorBox = el('p', { class: 'form-error' });
    const submit = el('button', { class: 'btn btn-primary' }, 'Agregar al catálogo');
    submit.addEventListener('click', async () => {
      errorBox.textContent = '';
      if (!desc.value.trim() || !price.value) {
        errorBox.textContent = 'La descripción y el precio son obligatorios.';
        return;
      }
      const input: NewManualComponent = {
        description: desc.value.trim(),
        sku: sku.value.trim() || null,
        brand: brand.value.trim() || null,
        kind: kindSel.value,
        price_amount: price.value,
        price_currency: currency.value as 'USD' | 'MXP',
        condition: condition.value as 'nuevo' | 'usado',
        specs: specInputs.collect(),
      };
      try {
        await api.addManualComponent(input);
        const refreshed = await api.listComponents();
        state.components = refreshed;
        onChanged(refreshed);
        closeModal();
        render();
      } catch (e) {
        errorBox.textContent = String(e);
      }
    });

    return el('form', { class: 'form', onsubmit: (e) => e.preventDefault() },
      el('div', { class: 'form-row' },
        el('label', {}, 'Tipo', kindSel),
        el('label', {}, 'Condición', condition),
      ),
      el('div', { class: 'form-row' }, el('label', {}, 'Descripción', desc)),
      el('div', { class: 'form-row' },
        el('label', {}, 'SKU', sku),
        el('label', {}, 'Marca', brand),
      ),
      el('div', { class: 'form-row' },
        el('label', {}, 'Precio', price),
        el('label', {}, 'Moneda', currency),
      ),
      extra,
      errorBox,
      el('div', { class: 'form-actions' }, submit),
    );
  }

  function renderSpecInputs(kind: string): { nodes: HTMLElement[]; collect: () => Record<string, unknown> } {
    const inputs: { key: string; input: HTMLInputElement | HTMLSelectElement; map?: (v: string) => unknown }[] = [];
    const nodes: HTMLElement[] = [];

    const text = (key: string, label: string, placeholder: string, map?: (v: string) => unknown) => {
      const i = el('input', { class: 'input', placeholder }) as HTMLInputElement;
      inputs.push({ key, input: i, map });
      nodes.push(el('label', {}, label, i));
    };

    switch (kind) {
      case 'cpu':
        text('socket', 'Socket', 'AM4 / AM5 / LGA1700');
        {
          const sel = el('select', { class: 'input' },
            el('option', { value: '' }, '¿Incluye disipador? (desconocido)'),
            el('option', { value: 'true' }, 'Sí incluye disipador'),
            el('option', { value: 'false' }, 'No incluye disipador'),
          ) as HTMLSelectElement;
          inputs.push({ key: 'includes_cooler', input: sel, map: (v) => (v === '' ? null : v === 'true') });
          nodes.push(el('label', {}, 'Disipador', sel));
        }
        break;
      case 'psu':
        text('wattage', 'Watts', '650', (v) => (v ? parseInt(v, 10) : null));
        break;
      case 'case':
        {
          const sel = el('select', { class: 'input' },
            el('option', { value: 'false' }, 'No incluye fuente'),
            el('option', { value: 'true' }, 'Sí incluye fuente'),
          ) as HTMLSelectElement;
          inputs.push({ key: 'includes_psu', input: sel, map: (v) => v === 'true' });
          nodes.push(el('label', {}, 'Fuente incluida', sel));
        }
        text('psu_watts', 'Watts de la fuente incluida', '600', (v) => (v ? parseInt(v, 10) : null));
        break;
      case 'ram':
        text('ddr', 'DDR', 'DDR4 / DDR5');
        text('capacity_gb', 'Capacidad (GB)', '16', (v) => (v ? parseInt(v, 10) : null));
        text('ram_form', 'Formato', 'DIMM / SODIMM');
        break;
      case 'storage':
        {
          const sel = el('select', { class: 'input' },
            el('option', { value: 'ssd' }, 'SSD'),
            el('option', { value: 'hdd' }, 'HDD'),
          ) as HTMLSelectElement;
          inputs.push({ key: 'storage_kind', input: sel });
          nodes.push(el('label', {}, 'Tipo', sel));
        }
        text('capacity_raw', 'Capacidad', '480GB / 2TB');
        break;
      case 'gpu':
        text('vram', 'VRAM', '8GB GDDR6');
        break;
    }
    if (nodes.length) {
      nodes.unshift(el('div', { class: 'form-title' }, 'Especificaciones (opcionales)'));
    }

    return {
      nodes,
      collect: () => {
        const specs: Record<string, unknown> = {};
        for (const { key, input, map } of inputs) {
          const v = input.value.trim();
          if (v === '') continue;
          specs[key] = map ? map(v) : v;
        }
        return specs;
      },
    };
  }

  function render() {
    const search = el('input', {
      class: 'input search',
      placeholder: 'Buscar por descripción, SKU o marca…',
      value: state.query,
      oninput: (e) => {
        state.query = (e.target as HTMLInputElement).value;
        tableZone.replaceChildren(renderTable());
      },
      onchange: (e) => {
        state.query = (e.target as HTMLInputElement).value;
        tableZone.replaceChildren(renderTable());
      },
    }) as HTMLInputElement;

    const kindSel = el('select', { class: 'input' },
      el('option', { value: '' }, 'Todos los armables'),
      ...PICKABLE_KINDS.map((k) => el('option', { value: k }, kindLabel(k))),
    ) as HTMLSelectElement;
    kindSel.value = state.filterKind;
    kindSel.addEventListener('change', () => {
      state.filterKind = kindSel.value;
      tableZone.replaceChildren(renderTable());
    });

    const clearSearch = el('button', { class: 'icon-btn', title: 'Limpiar búsqueda' }, '✕');
    clearSearch.addEventListener('click', () => {
      state.query = '';
      search.value = '';
      tableZone.replaceChildren(renderTable());
      search.focus();
    });

    const manualBtn = el('button', { class: 'btn btn-primary' }, '+ Componente manual / usado');
    manualBtn.addEventListener('click', () => {
      modal('Agregar componente manual', renderManualForm());
    });

    const tableZone = el('div');
    tableZone.append(renderTable());

    view.replaceChildren(
      el('div', { class: 'toolbar' }, search, clearSearch, kindSel, manualBtn),
      tableZone,
    );
  }

  render();
  return view;
}
