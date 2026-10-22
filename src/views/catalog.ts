// Vista del catálogo: exploración, búsqueda, alta manual y edición.
import { openProductForm } from './product_form';
import { kindLabel, PICKABLE_KINDS } from '../meta';
import { ComponentDto } from '../types';
import { el, money, mxn } from '../ui';

interface CatalogState {
  components: ComponentDto[];
  filterKind: string; // '' = todos los armables
  query: string;
}

export function createCatalogView(
  components: ComponentDto[],
  /** Recarga la lista compartida IN-PLACE (misma referencia). */
  refresh: () => Promise<ComponentDto[]>,
): HTMLElement {
  const state: CatalogState = { components, filterKind: '', query: '' };

  const view = el('section', { class: 'view catalog-view' });

  function specBadges(c: ComponentDto): string[] {
    const s = c.specs ?? {};
    // Las specs corregidas a mano (protegidas del import) llevan candado.
    const locked = new Set(Object.keys(c.overrides?.specs ?? {}));
    const lk = (key: string, text: string) => (locked.has(key) ? `🔒 ${text}` : text);
    const out: string[] = [];
    if (s.socket) out.push(lk('socket', s.socket));
    if (s.includes_cooler === true) out.push(lk('includes_cooler', 'trae disipador'));
    if (s.includes_cooler === false) out.push(lk('includes_cooler', 'sin disipador'));
    if (s.wattage) out.push(lk('wattage', `${s.wattage}W`));
    if (s.psu_watts && s.includes_psu !== false) out.push(lk('psu_watts', `PSU ${s.psu_watts}W incluida`));
    if (s.includes_psu === true && !s.psu_watts) out.push(lk('includes_psu', 'PSU incluida'));
    if (s.includes_psu === false && c.kind === 'case') out.push(lk('includes_psu', 'sin PSU'));
    if (s.ddr) out.push(lk('ddr', s.ddr));
    if (s.capacity_gb && (c.kind === 'ram')) out.push(lk('capacity_gb', `${s.capacity_gb}GB`));
    if (s.capacity_raw) out.push(lk('capacity_raw', s.capacity_raw));
    if (s.ram_form && c.kind === 'ram') out.push(lk('ram_form', s.ram_form));
    if (s.vram) out.push(lk('vram', s.vram));
    if (s.inches) out.push(lk('inches', `${s.inches}"`));
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

  async function afterSaved() {
    await refresh();
    tableZone.replaceChildren(renderTable());
  }

  function editButton(c: ComponentDto): HTMLElement {
    const btn = el('button', { class: 'icon-btn', title: `Editar ${c.sku}` }, '✎');
    btn.addEventListener('click', () => {
      openProductForm({ initial: c, onSaved: afterSaved });
    });
    return btn;
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
          el('th', { class: 'col-actions' }, ''),
        ),
      ),
      el('tbody', {},
        ...rows.slice(0, 400).map((c) =>
          el('tr', {
            class: [
              c.condition === 'usado' ? 'row-used' : '',
              c.stock_ensamble_total === 0 ? 'row-out' : '',
            ].join(' ').trim(),
          },
            el('td', {}, el('span', { class: `chip chip-${c.kind}` }, kindLabel(c.kind))),
            el('td', {},
              el('div', { class: 'desc' },
                c.description,
                c.on_promotion ? el('span', { class: 'chip chip-promo' }, 'PROMO') : null,
                c.source === 'manual' ? el('span', { class: 'chip chip-manual' }, 'MANUAL') : null,
                c.stock_ensamble_total === 0 ? el('span', { class: 'chip chip-danger' }, 'AGOTADO') : null,
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
            el('td', { class: 'col-actions' }, editButton(c)),
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

    const manualBtn = el('button', { class: 'btn btn-primary' }, '+ Agregar producto (manual / PCH)');
    manualBtn.addEventListener('click', () => {
      openProductForm({
        title: 'Agregar producto',
        defaultSupplier: 'manual',
        onSaved: afterSaved,
      });
    });

    tableZone.replaceChildren(renderTable());

    view.replaceChildren(
      el('div', { class: 'toolbar' }, search, clearSearch, kindSel, manualBtn),
      tableZone,
    );
  }

  const tableZone = el('div');
  render();
  return view;
}
