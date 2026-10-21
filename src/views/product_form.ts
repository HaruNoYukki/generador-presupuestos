// Formulario compartido de alta manual de productos.
// Lo usan la vista Catálogo y el picker de ensambles (botón "agregar
// producto nuevo" dentro de la lista desplegable). Soporta captura para
// proveedores sin lista de Excel (PCH) con existencia a mano.
import { api, NewManualComponent } from '../api';
import { ComponentDto, Kind } from '../types';
import { closeModal, el, modal } from '../ui';

export interface ProductFormOptions {
  /** Si viene, limita el select de tipo a estos kinds (picker de slot). */
  fixedKinds?: Kind[];
  /** Tipo preseleccionado. */
  defaultKind?: Kind;
  /** Proveedor preseleccionado: 'pch' | 'manual'. */
  defaultSupplier?: string;
  /** Título del modal. */
  title?: string;
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

/** Campos de especificación según el tipo seleccionado. */
function specInputsFor(kind: string): { nodes: HTMLElement[]; collect: () => Record<string, unknown> } {
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
        text('psu_watts', 'Watts de la fuente incluida', '600', (v) => (v ? parseInt(v, 10) : null));
        text('form_factor', 'Factor de forma', 'ATX / MICRO ATX / MINI ITX');
      }
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
    case 'motherboard':
      text('socket', 'Socket', 'AM4 / AM5 / LGA1700');
      text('ddr', 'DDR', 'DDR4 / DDR5');
      text('form_factor', 'Factor de forma', 'ATX / MICRO ATX / MINI ITX');
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

export function openProductForm(opts: ProductFormOptions): void {
  const kinds: string[] = opts.fixedKinds ?? Object.keys(KIND_LABELS);
  const kindSel = el('select', { class: 'input' },
    ...kinds.map((k) => el('option', { value: k, selected: k === opts.defaultKind }, KIND_LABELS[k] ?? k)),
  ) as HTMLSelectElement;

  const supplierSel = el('select', { class: 'input' },
    el('option', { value: 'pch', selected: opts.defaultSupplier === 'pch' },
      'PCH (captura a mano, sin lista de Excel)'),
    el('option', { value: 'manual', selected: opts.defaultSupplier !== 'pch' }, 'Manual (sin proveedor)'),
  ) as HTMLSelectElement;

  const condition = el('select', { class: 'input' },
    el('option', { value: 'nuevo', selected: true }, 'Nuevo'),
    el('option', { value: 'usado' }, 'Usado'),
  ) as HTMLSelectElement;

  const desc = el('input', { class: 'input', placeholder: 'Descripción del producto' }) as HTMLInputElement;
  const sku = el('input', { class: 'input', placeholder: 'SKU (vacío = autogenerado)' }) as HTMLInputElement;
  const brand = el('input', { class: 'input', placeholder: 'Marca' }) as HTMLInputElement;
  const price = el('input', { class: 'input', type: 'number', min: '0', step: '0.01', placeholder: 'Precio' }) as HTMLInputElement;
  const currency = el('select', { class: 'input' },
    el('option', { value: 'MXP' }, 'Pesos (MXP)'),
    el('option', { value: 'USD' }, 'Dólares (USD)'),
  ) as HTMLSelectElement;
  const stock = el('input', { class: 'input', type: 'number', min: '0', step: '1', value: '0' }) as HTMLInputElement;

  const specZone = el('div', { class: 'form-row' });
  let specCollect = () => ({}) as Record<string, unknown>;
  function refreshSpecs() {
    const { nodes, collect } = specInputsFor(String(kindSel.value));
    specZone.replaceChildren(...nodes);
    specCollect = collect;
  }
  kindSel.addEventListener('change', refreshSpecs);
  refreshSpecs();

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
      kind: String(kindSel.value),
      price_amount: price.value,
      price_currency: currency.value as 'USD' | 'MXP',
      condition: condition.value as 'nuevo' | 'usado',
      specs: specCollect(),
      supplier: String(supplierSel.value),
      stock: parseInt(stock.value, 10) || 0,
    };
    try {
      const dto = await api.addManualComponent(input);
      closeModal();
      opts.onSaved(dto);
    } catch (e) {
      errorBox.textContent = String(e);
    }
  });

  const box = el('div', { class: 'form' },
    el('div', { class: 'form-row' },
      el('label', {}, 'Tipo', kindSel),
      el('label', {}, 'Proveedor', supplierSel),
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
      el('label', {}, 'Existencia (PCH)', stock),
    ),
    specZone,
    errorBox,
    el('div', { class: 'form-actions' }, submit),
  );

  modal(opts.title ?? 'Agregar producto', box);
}
