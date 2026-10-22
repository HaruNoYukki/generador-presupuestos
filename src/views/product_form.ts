// Formulario compartido de alta y edición de productos.
// Lo usan la vista Catálogo (alta y edición por renglón) y el picker de
// ensambles (botón "agregar producto nuevo"). Soporta captura para
// proveedores sin lista de Excel (PCH / manual) con existencia a mano.
//
// En edición, el maestro (descripción, marca, tipo, condición, specs) se
// edita siempre; precio y existencia solo si el producto tiene una oferta
// manual o PCH (las de lista se actualizan importando el Excel). Las specs
// corregidas a mano sobre un SKU de lista quedan protegidas del import y se
// muestran con candado; "usar lista" regresa al valor que trajo el Excel.
import { api, NewManualComponent, UpdateComponentInput } from '../api';
import { ComponentDto, Kind, Offer, Specs } from '../types';
import { closeModal, el, modal, money } from '../ui';

export interface ProductFormOptions {
  /** Si viene, limita el select de tipo a estos kinds (picker de slot). */
  fixedKinds?: Kind[];
  /** Tipo preseleccionado. */
  defaultKind?: Kind;
  /** Proveedor preseleccionado: 'pch' | 'manual'. */
  defaultSupplier?: string;
  /** Título del modal. */
  title?: string;
  /** Si viene, el formulario edita este componente en vez de dar de alta. */
  initial?: ComponentDto;
  onSaved: (dto: ComponentDto) => void;
}

const KIND_LABELS: Record<string, string> = {
  cpu: 'Procesador',
  motherboard: 'Tarjeta madre',
  ram: 'Memoria RAM',
  storage: 'Almacenamiento',
  psu: 'Fuente de poder',
  case: 'Gabinete',
  cooler_air: 'Enfriamiento aire',
  cooler_liquid: 'Enfriamiento líquido',
  gpu: 'Tarjeta de video',
  monitor: 'Monitor',
  keyboard: 'Teclado',
  mouse: 'Mouse',
  keyboard_mouse_kit: 'Kit teclado + mouse',
  expansion_card: 'Tarjeta de expansión',
};

const EDITABLE_SUPPLIERS = ['manual', 'pch'];

function hasListOffer(c: ComponentDto): boolean {
  return c.offers.some((o) => !EDITABLE_SUPPLIERS.includes(o.supplier));
}

/** Valor de spec → texto del input/select. */
function toInputValue(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  return String(v);
}

interface SpecLock {
  overridden: Set<string>;
  listSpecs: Specs;
}

/** Campos de especificación según el tipo seleccionado. */
function specInputsFor(
  kind: string,
  catalogs: { sockets: string[]; ddrs: string[]; formFactors: string[]; ramForms: string[] },
  initial: Specs,
  lock: SpecLock | null,
): { nodes: HTMLElement[]; collect: () => Record<string, unknown> } {
  const { sockets, ddrs, formFactors, ramForms } = catalogs;
  const inputs: { key: string; input: HTMLInputElement | HTMLSelectElement; map?: (v: string) => unknown }[] = [];
  const nodes: HTMLElement[] = [];
  const initialOf = (key: string) => toInputValue((initial as Record<string, unknown>)[key]);

  // Candado: la spec se corrigió a mano y el import no la pisa. El botón
  // regresa al valor de la lista (al guardar, igual a la lista = sin candado).
  const withLock = (key: string, label: string, input: HTMLInputElement | HTMLSelectElement) => {
    if (!lock || !lock.overridden.has(key)) return el('label', {}, label, input);
    const listVal = toInputValue((lock.listSpecs as Record<string, unknown>)[key]);
    const revert = el('button', {
      class: 'btn btn-outline btn-xs',
      type: 'button',
      title: 'Volver al valor que trajo la lista del proveedor',
    }, `↺ usar lista (${listVal || 'vacío'})`);
    revert.addEventListener('click', () => {
      input.value = listVal;
      input.dispatchEvent(new Event('change'));
    });
    return el('label', { class: 'spec-locked' },
      el('span', { class: 'spec-lock-label', title: 'Corregido a mano: protegido del import' }, `🔒 ${label}`),
      input,
      revert,
    );
  };

  const text = (key: string, label: string, placeholder: string, map?: (v: string) => unknown) => {
    const i = el('input', { class: 'input', placeholder, value: initialOf(key) }) as HTMLInputElement;
    inputs.push({ key, input: i, map });
    nodes.push(withLock(key, label, i));
  };

  const select = (key: string, label: string, options: string[], emptyLabel: string) => {
    const current = initialOf(key);
    const opts = current && !options.includes(current) ? [current, ...options] : options;
    const sel = el('select', { class: 'input' },
      el('option', { value: '' }, emptyLabel),
      ...opts.map((o) => el('option', { value: o, selected: o === current }, o)),
    ) as HTMLSelectElement;
    inputs.push({ key, input: sel });
    nodes.push(withLock(key, label, sel));
  };

  const boolSelect = (
    key: string,
    label: string,
    labels: { empty?: string; yes: string; no: string },
    map: (v: string) => unknown,
  ) => {
    const current = initialOf(key);
    const sel = el('select', { class: 'input' },
      ...(labels.empty ? [el('option', { value: '', selected: current === '' }, labels.empty)] : []),
      el('option', { value: 'true', selected: current === 'true' }, labels.yes),
      el('option', { value: 'false', selected: current === 'false' || (!labels.empty && current === '') }, labels.no),
    ) as HTMLSelectElement;
    inputs.push({ key, input: sel, map });
    nodes.push(withLock(key, label, sel));
  };

  switch (kind) {
    case 'cpu':
      select('socket', 'Socket', sockets, 'Desconocido');
      boolSelect('includes_cooler', 'Disipador', {
        empty: '¿Incluye disipador? (desconocido)',
        yes: 'Sí incluye disipador',
        no: 'No incluye disipador',
      }, (v) => (v === '' ? null : v === 'true'));
      break;
    case 'psu':
      text('wattage', 'Watts', '650', (v) => (v ? parseInt(v, 10) : null));
      break;
    case 'case':
      boolSelect('includes_psu', 'Fuente incluida', {
        yes: 'Sí incluye fuente',
        no: 'No incluye fuente',
      }, (v) => v === 'true');
      text('psu_watts', 'Watts de la fuente incluida', '600', (v) => (v ? parseInt(v, 10) : null));
      select('form_factor', 'Factor de forma', formFactors, 'Desconocido');
      break;
    case 'ram':
      select('ddr', 'DDR', ddrs, 'Desconocido');
      text('capacity_gb', 'Capacidad (GB)', '16', (v) => (v ? parseInt(v, 10) : null));
      select('ram_form', 'Formato', ramForms, 'Desconocido');
      break;
    case 'storage':
      {
        const current = initialOf('storage_kind');
        const sel = el('select', { class: 'input' },
          el('option', { value: 'ssd', selected: current !== 'hdd' }, 'SSD'),
          el('option', { value: 'hdd', selected: current === 'hdd' }, 'HDD'),
        ) as HTMLSelectElement;
        inputs.push({ key: 'storage_kind', input: sel });
        nodes.push(withLock('storage_kind', 'Tipo', sel));
      }
      text('capacity_raw', 'Capacidad', '480GB / 2TB');
      break;
    case 'gpu':
      text('vram', 'VRAM', '8GB GDDR6');
      break;
    case 'motherboard':
      select('socket', 'Socket', sockets, 'Desconocido');
      select('ddr', 'DDR', ddrs, 'Desconocido');
      select('form_factor', 'Factor de forma', formFactors, 'Desconocido');
      break;
  }
  return { nodes, collect: () => {
    const specs: Record<string, unknown> = {};
    for (const { key, input, map } of inputs) {
      const v = input.value.trim();
      if (v === '') continue;
      specs[key] = map ? map(v) : v;
    }
    return specs;
  } };
}

export async function openProductForm(opts: ProductFormOptions): Promise<void> {
  const initial = opts.initial ?? null;
  const editing = initial !== null;

  // Cargar catálogos normalizados (marcas y atributos) antes de armar el form
  const [filters, sockets, ddrs, formFactors, ramForms] = await Promise.all([
    api.catalogFilters().catch(() => ({ brands: [], departments: [] })),
    api.listSpecOptions('socket').catch(() => [] as string[]),
    api.listSpecOptions('ddr').catch(() => [] as string[]),
    api.listSpecOptions('form_factor').catch(() => [] as string[]),
    api.listSpecOptions('ram_form').catch(() => [] as string[]),
  ]);
  const brands: string[] = filters.brands;

  // En edición de un SKU de lista, lo corregido a mano lleva candado.
  const listLocked = editing && hasListOffer(initial);
  const lock: SpecLock | null = listLocked
    ? { overridden: new Set(Object.keys(initial.overrides?.specs ?? {})), listSpecs: initial.list_specs ?? {} }
    : null;

  const kinds: string[] = opts.fixedKinds ?? Object.keys(KIND_LABELS);
  const selectedKind = initial?.kind ?? opts.defaultKind;
  if (selectedKind && !kinds.includes(selectedKind)) kinds.unshift(selectedKind);
  const kindSel = el('select', { class: 'input' },
    ...kinds.map((k) => el('option', { value: k, selected: k === selectedKind }, KIND_LABELS[k] ?? k)),
  ) as HTMLSelectElement;
  const kindLocked = listLocked && initial.overrides?.kind != null;
  const kindLabel: (Node | string)[] = kindLocked
    ? [el('span', { class: 'spec-lock-label', title: 'Tipo corregido a mano: protegido del import' },
        `🔒 Tipo (lista: ${KIND_LABELS[initial.list_kind] ?? initial.list_kind})`)]
    : ['Tipo'];

  // Oferta a editar: en modo edición se elige entre las del producto
  // (manual/PCH se escriben directo; las de lista quedan como corrección
  // protegida del import). En alta no aplica.
  let selectedOffer: Offer | null = editing
    ? initial.offers.find((o) => o.supplier === initial.supplier)
      ?? initial.offers[0]
      ?? null
    : null;
  const priceEditable = !editing || selectedOffer !== null;

  const supplierSel = el('select', { class: 'input' },
    el('option', { value: 'pch', selected: opts.defaultSupplier === 'pch' },
      'PCH (captura a mano, sin lista de Excel)'),
    el('option', { value: 'manual', selected: opts.defaultSupplier !== 'pch' }, 'Manual (sin proveedor)'),
  ) as HTMLSelectElement;
  const supplierLabel = (s: string) =>
    s === 'cva' ? 'CVA' : s === 'arroba' ? '@ arroba' : s.toUpperCase();
  const supplierInfo = editing
    ? initial.offers.length > 1
      ? el('select', { class: 'input' },
          ...initial.offers.map((o) => el('option', {
            value: o.supplier,
            selected: o.supplier === selectedOffer?.supplier,
          }, `${supplierLabel(o.supplier)} · ${money(o.price)}`)),
        ) as HTMLSelectElement
      : el('span', { class: 'input input-readonly' },
          initial.offers.map((o) => supplierLabel(o.supplier)).join(', ') || 'sin oferta')
    : supplierSel;
  if (editing && initial.offers.length > 1) {
    (supplierInfo as HTMLSelectElement).addEventListener('change', () => {
      selectedOffer = initial.offers.find((o) => o.supplier === (supplierInfo as HTMLSelectElement).value) ?? null;
      loadOfferFields();
    });
  }

  const condition = el('select', { class: 'input' },
    el('option', { value: 'nuevo', selected: (initial?.condition ?? 'nuevo') === 'nuevo' }, 'Nuevo'),
    el('option', { value: 'usado', selected: initial?.condition === 'usado' }, 'Usado'),
  ) as HTMLSelectElement;

  const desc = el('input', {
    class: 'input',
    placeholder: 'Descripción del producto',
    value: initial?.description ?? '',
  }) as HTMLInputElement;
  const sku = el('input', {
    class: 'input',
    placeholder: 'SKU (vacío = autogenerado)',
    value: initial?.sku ?? '',
  }) as HTMLInputElement;
  if (editing) sku.readOnly = true;

  // Marca: select desde el catálogo (normalizado)
  const brandOptions = initial?.brand && initial.brand !== 'SIN MARCA' && !brands.includes(initial.brand)
    ? [initial.brand, ...brands]
    : brands;
  const brand = el('select', { class: 'input' },
    el('option', { value: '' }, 'SIN MARCA'),
    ...brandOptions.map((b) => el('option', { value: b, selected: b === initial?.brand }, b)),
  ) as HTMLSelectElement;

  const price = el('input', {
    class: 'input', type: 'number', min: '0', step: '0.01', placeholder: 'Precio',
    value: selectedOffer?.price.amount ?? initial?.price.amount ?? '',
  }) as HTMLInputElement;
  const currency = el('select', { class: 'input' },
    el('option', { value: 'MXP', selected: (selectedOffer?.price.currency ?? 'MXP') === 'MXP' }, 'Pesos (MXP)'),
    el('option', { value: 'USD', selected: selectedOffer?.price.currency === 'USD' }, 'Dólares (USD)'),
  ) as HTMLSelectElement;
  const stock = el('input', {
    class: 'input', type: 'number', min: '0', step: '1',
    value: String(selectedOffer?.stock_ensamble ?? 0),
  }) as HTMLInputElement;

  // Nota según la oferta elegida + botón "usar valores de la lista" cuando
  // la oferta tiene correcciones manuales activas.
  const priceNote = el('p', { class: 'muted small' });
  const revertBtn = el('button', { class: 'btn btn-outline btn-xs', type: 'button' }) as HTMLButtonElement;
  function refreshOfferNote() {
    const isManual = selectedOffer != null && EDITABLE_SUPPLIERS.includes(selectedOffer.supplier);
    if (selectedOffer == null) {
      priceNote.textContent = '';
    } else if (isManual) {
      priceNote.textContent = 'Oferta capturada a mano: los cambios se escriben directo.';
    } else if (selectedOffer.price_overridden || selectedOffer.stock_overridden) {
      priceNote.textContent = '🔒 Corregido a mano: protegido del import.';
    } else {
      priceNote.textContent = 'Los cambios quedan protegidos del import (corrige datos que la lista trae incompletos).';
    }
    const hasOverride = selectedOffer?.price_overridden || selectedOffer?.stock_overridden;
    revertBtn.style.display = hasOverride ? '' : 'none';
    if (hasOverride && selectedOffer) {
      const parts: string[] = [];
      if (selectedOffer.price_overridden && selectedOffer.list_price) {
        parts.push(`precio ${selectedOffer.list_price.amount} ${selectedOffer.list_price.currency}`);
      }
      if (selectedOffer.stock_overridden && selectedOffer.list_stock != null) {
        parts.push(`existencia ${selectedOffer.list_stock}`);
      }
      revertBtn.textContent = `↺ usar lista (${parts.join(' · ')})`;
    }
  }
  revertBtn.addEventListener('click', async () => {
    if (!selectedOffer || !editing) return;
    errorBox.textContent = '';
    try {
      const dto = await api.clearOfferOverride(initial.id, selectedOffer.supplier);
      closeModal();
      opts.onSaved(dto);
    } catch (e) {
      errorBox.textContent = String(e);
    }
  });
  function loadOfferFields() {
    price.value = selectedOffer?.price.amount ?? '';
    currency.value = selectedOffer?.price.currency ?? 'MXP';
    stock.value = String(selectedOffer?.stock_ensamble ?? 0);
    refreshOfferNote();
  }
  refreshOfferNote();

  const catalog = { sockets, ddrs, formFactors, ramForms };
  const specZone = el('div', { class: 'form-row' });
  let specCollect = () => ({}) as Record<string, unknown>;
  function refreshSpecs() {
    const sameKind = String(kindSel.value) === initial?.kind;
    const { nodes, collect } = specInputsFor(
      String(kindSel.value),
      catalog,
      sameKind ? initial?.specs ?? {} : {},
      sameKind ? lock : null,
    );
    specZone.replaceChildren(...nodes);
    specCollect = collect;
  }
  kindSel.addEventListener('change', refreshSpecs);
  refreshSpecs();

  const errorBox = el('p', { class: 'form-error' });
  const submit = el('button', { class: 'btn btn-primary' },
    editing ? 'Guardar cambios' : 'Agregar al catálogo');
  submit.addEventListener('click', async () => {
    errorBox.textContent = '';
    if (!desc.value.trim() || (priceEditable && !price.value)) {
      errorBox.textContent = 'La descripción y el precio son obligatorios.';
      return;
    }
    try {
      let dto: ComponentDto;
      if (editing) {
        const input: UpdateComponentInput = {
          description: desc.value.trim(),
          brand: brand.value.trim() || null,
          kind: String(kindSel.value),
          condition: condition.value as 'nuevo' | 'usado',
          specs: specCollect(),
          supplier: selectedOffer?.supplier ?? null,
          price_amount: selectedOffer ? price.value : null,
          price_currency: selectedOffer ? (currency.value as 'USD' | 'MXP') : null,
          stock: selectedOffer ? parseInt(stock.value, 10) || 0 : null,
        };
        dto = await api.updateComponent(initial.id, input);
      } else {
        const input: NewManualComponent = {
          description: desc.value.trim(),
          sku: sku.value.trim() || null,
          brand: brand.value.trim() || null,
          kind: String(kindSel.value),
          price_amount: price.value,
          price_currency: currency.value as 'USD' | 'MXP',
          condition: condition.value as 'nuevo' | 'usado',
          specs: specCollect(),
          supplier: String(supplierSel.value),
          stock: parseInt(stock.value, 10) || 0,
        };
        dto = await api.addManualComponent(input);
      }
      closeModal();
      opts.onSaved(dto);
    } catch (e) {
      errorBox.textContent = String(e);
    }
  });

  const box = el('div', { class: 'form' },
    el('div', { class: 'form-row' },
      el('label', {}, ...kindLabel, kindSel),
      el('label', {}, 'Proveedor', supplierInfo),
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
      el('label', {}, selectedOffer ? `Existencia (${selectedOffer.supplier})` : 'Existencia', stock),
    ),
    el('div', {}, priceNote, revertBtn),
    specZone,
    errorBox,
    el('div', { class: 'form-actions' }, submit),
  );

  modal(opts.title ?? (editing ? `Editar ${initial.sku}` : 'Agregar producto'), box);
}
