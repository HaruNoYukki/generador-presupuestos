// Vista de Venta individual (flujo 2): carrito de cualquier producto,
// ganancia con presets/objetivo, captura de N/S opcional y nota de venta
// exportable. El carrito vive en el estado compartido (main.ts).
import { api } from '../api';
import { CartItem, ComponentDto, ProfitLine, ProfitReport } from '../types';
import { el, mxn } from '../ui';
import logoUrl from '../assets/nykniu-logo.png';

export interface SaleDirectDeps {
  components: () => ComponentDto[];
  cart: () => CartItem[];
  setCart: (items: CartItem[]) => void;
  getParams: () => { percent: string; target: string };
  setParams: (p: { percent: string; target: string }) => void;
}

const TYPE_LABELS: Record<string, string> = {
  cpu: 'Procesador', motherboard: 'Motherboard', ram: 'RAM', storage: 'Almacenamiento',
  psu: 'PSU', case: 'Gabinete', cooling: 'Enfriamiento', gpu: 'GPU',
  monitor: 'Monitor', keyboard: 'Teclado', mouse: 'Mouse',
  keyboard_mouse_kit: 'Kit teclado + mouse', expansion_card: 'Tarjeta de expansión',
  other: 'Producto',
};

const LEYENDAS_NV = [
  'Agradecemos su preferencia y confianza. Esperamos seguir siendo su elección de confianza para sus necesidades futuras.',
  'Nos enorgullece informarte que todos nuestros componentes son completamente nuevos y vienen respaldados por una garantía de un año. Además, queremos asegurarte que tu satisfacción está garantizada, ya que ofrecemos un período de garantía de 365 días naturales para las solicitudes de venta de productos. Es importante tener en cuenta que nuestras garantías se aplican exclusivamente a defectos de fabricación y/o servicio, y no cubren daños físicos, problemas causados por software no original, malware, ransomware, entre otros.',
  'Para realizar una reclamación de garantía, es fundamental contar con la nota o factura de compra y conservar el empaque original del producto. Asimismo, requerimos llevar a cabo una revisión previa antes de iniciar el proceso de garantía, asegurando que se cumplan los términos y condiciones correspondientes.',
  'Tu confianza en nuestros productos y servicios es de suma importancia para nosotros, y nos esforzamos por garantizar tu satisfacción en cada paso del camino. Si tienes alguna pregunta o necesitas asistencia adicional con respecto a nuestras garantías, no dudes en ponerte en contacto con nuestro equipo de atención al cliente. Estaremos encantados de ayudarte en todo lo que necesites.',
];

/** Unidades expandidas del carrito, con su N/S capturable. */
interface UnitRow {
  key: string;
  componentId: string;
  sku: string;
  description: string;
  kind: string;
  price: string;
  serial: string;
}

const round2 = (n: number) => Math.round(n * 100) / 100;

export function createSaleDirectView(deps: SaleDirectDeps): HTMLElement {
  const view = el('section', { class: 'view sale-direct-view' });
  let report: ProfitReport | null = null;
  let units: UnitRow[] = [];
  let saleNumber: string | null = null;
  let flashMsg = '';

  const components = () => deps.components();
  const byId = () => new Map(components().map((c) => [c.id, c]));

  // ------------------------------------------------------------------
  // Búsqueda y carrito
  // ------------------------------------------------------------------
  // La venta individual vende CUALQUIER producto con existencias — incluidos
  // los tipo «otro» (cámaras, cables, memorias USB, etc.). El texto coincide
  // contra SKU, descripción (ahí viene el modelo) y marca.
  function searchResults(query: string, brand: string): ComponentDto[] {
    const q = query.trim().toUpperCase();
    const list = components().filter((c) => brand === '' || c.brand === brand);
    if (!q && brand === '') return [];
    return list
      .filter(
        (c) =>
          !q ||
          c.sku.toUpperCase().includes(q) ||
          c.description.toUpperCase().includes(q) ||
          c.brand.toUpperCase().includes(q),
      )
      .slice(0, 50);
  }

  function renderSearch() {
    let brandFilter = '';
    const brandSel = el('select', { class: 'input' },
      el('option', { value: '' }, 'Todas las marcas'),
    ) as HTMLSelectElement;
    // Marcas del catálogo (mismos catálogos normalizados que los filtros).
    api.catalogFilters()
      .then((f) => {
        brandSel.replaceChildren(
          el('option', { value: '' }, 'Todas las marcas'),
          ...f.brands.map((b) => el('option', { value: b }, b)),
        );
      })
      .catch(() => {});
    brandSel.addEventListener('change', () => {
      brandFilter = brandSel.value;
      runSearch();
    });

    const input = el('input', {
      class: 'input search',
      placeholder: 'Agregar por SKU, modelo, descripción o marca…',
    }) as HTMLInputElement;
    const results = el('div', { class: 'search-results' });
    function runSearch() {
      const found = searchResults(input.value, brandFilter);
      results.replaceChildren(
        ...found.map((c) => {
          const row = el('div', { class: 'picker-row' },
            el('div', { class: 'picker-desc' },
              el('div', { class: 'desc' }, c.description, procChip(c)),
              el('div', { class: 'sub' },
                `${c.sku} · ${c.brand} · vendible: ${sellableStock(c)}`)),
            el('div', { class: 'picker-side' },
              el('div', { class: 'strong' }, mxn(c.price_mxp.amount)),
              el('span', { class: 'chip chip-ok' }, '+ agregar')),
          );
          row.addEventListener('click', () => {
            addToCart(c.id);
            input.value = '';
            results.replaceChildren();
            input.focus();
          });
          return row;
        }),
        found.length === 0 && (input.value.trim() || brandFilter)
          ? el('p', { class: 'muted' }, 'Sin resultados.')
          : el('span'),
      );
    }
    input.addEventListener('input', runSearch);
    return el('div', {},
      el('div', { class: 'toolbar' }, brandSel, input),
      results,
    );
  }

  /** Venta individual: se usan por igual Disp y Disp CD (CVA) y el stock
   * de CEN/AVE-2/CEDIS de arroba. */
  function sellableStock(c: ComponentDto): number {
    return c.stock_individual_total || c.stock.a_cen + c.stock.ave_2 + c.stock.cedis;
  }

  function addToCart(id: string) {
    const cart = deps.cart();
    const current = cart.find((i) => i.component_id === id)?.quantity ?? 0;
    const c = byId().get(id);
    const max = c ? sellableStock(c) : 0;
    if (current + 1 > max) {
      flashMsg = `Sin inventario suficiente para "${c?.sku}": disponible ${max} (arroba CEN/AVE-2/CEDIS + CVA Disp/CD).`;
      render();
      return;
    }
    const next = cart.filter((i) => i.component_id !== id);
    next.push({ component_id: id, quantity: current + 1 });
    deps.setCart(next);
    load();
  }

  function setQty(id: string, qty: number) {
    const cart = deps.cart().filter((i) => i.component_id !== id);
    if (qty > 0) cart.push({ component_id: id, quantity: qty });
    deps.setCart(cart);
    load();
  }

  function renderCart() {
    const cart = deps.cart();
    if (cart.length === 0) {
      return el('p', { class: 'panel-muted' },
        'Busca un producto por SKU o descripción y agrégalo al carrito.');
    }
    const map = byId();
    const rows = cart.map((item) => {
      const c = map.get(item.component_id);
      if (!c) return el('tr', {}, el('td', { colspan: '6' }, 'Producto no encontrado'));
      const stock = sellableStock(c);
      return el('tr', {},
        el('td', {},
          TYPE_LABELS[c.kind] ?? c.kind,
          procChip(c)),
        el('td', {},
          el('div', { class: 'desc', title: c.description }, c.description),
          el('div', { class: 'sub' }, `${c.sku} · inventario vendible: ${stock}`)),
        el('td', { class: 'num' },
          el('div', { class: 'qty-stepper' },
            el('button', { class: 'icon-btn', title: 'Quitar una' }, '−'),
            el('span', { class: 'qty' }, `×${item.quantity}`),
            el('button', { class: 'icon-btn', title: 'Agregar una' }, '+'),
          ),
        ),
        el('td', { class: 'num strong' }, mxn(c.price_mxp.amount)),
        el('td', {}, el('button', { class: 'icon-btn', title: 'Quitar del carrito' }, '✕')),
      );
    });

    const table = el('table', { class: 'catalog-table cart-table' },
      el('thead', {}, el('tr', {},
        el('th', {}, 'Tipo'), el('th', {}, 'Producto'),
        el('th', { class: 'num' }, 'Cantidad'), el('th', { class: 'num' }, 'Costo c/u (s/IVA)'),
        el('th', {}, ''))),
      el('tbody', {}, ...rows),
    );

    // Cable de eventos (steppers y quitar)
    const buttons = table.querySelectorAll('button');
    let bi = 0;
    for (const item of cart) {
      buttons[bi++]?.addEventListener('click', () => setQty(item.component_id, item.quantity - 1));
      buttons[bi++]?.addEventListener('click', () => setQty(item.component_id, item.quantity + 1));
      buttons[bi++]?.addEventListener('click', () => setQty(item.component_id, 0));
    }
    return table;
  }

  // ------------------------------------------------------------------
  // Ganancia, N/S y hoja
  // ------------------------------------------------------------------
  function expandUnits() {
    const map = byId();
    const rows: UnitRow[] = [];
    for (const item of deps.cart()) {
      const c = map.get(item.component_id);
      if (!c) continue;
      const r = report?.lines.find((l) => l.component_id === item.component_id);
      const price = r ? r.price : c.price_mxp.amount;
      const unit = round2(parseFloat(price) / item.quantity);
      for (let i = 0; i < item.quantity; i++) {
        const p = i === item.quantity - 1
          ? round2(parseFloat(price) - unit * (item.quantity - 1))
          : unit;
        const key = `${item.component_id}#${i}`;
        const prev = units.find((u) => u.key === key);
        rows.push({
          key,
          componentId: item.component_id,
          sku: c.sku,
          description: c.description,
          kind: c.kind,
          price: p.toFixed(2),
          serial: prev?.serial ?? '',
        });
      }
    }
    units = rows;
  }

  async function load() {
    flashMsg = '';
    const cart = deps.cart();
    if (cart.length === 0) {
      report = null;
      units = [];
      render();
      return;
    }
    try {
      const { percent, target } = deps.getParams();
      report = await api.cartProfit(cart, percent, target.trim() === '' ? null : target.trim());
      expandUnits();
    } catch (e) {
      flashMsg = String(e);
      report = null;
    }
    render();
  }

  function renderProfitControls() {
    const { percent } = deps.getParams();
    const presets = ['10', '20', '30'];
    const presetBtns = presets.map((p) =>
      el('button', {
        class: `btn ${percent === p ? 'btn-primary' : 'btn-outline'}`,
        onclick: () => {
          deps.setParams({ ...deps.getParams(), percent: p });
          load();
        },
      }, `${p}%`),
    );
    const custom = el('input', {
      class: 'input', type: 'number', min: '0', step: '0.01',
      placeholder: 'Otro %', value: !presets.includes(percent) ? percent : '',
    }) as HTMLInputElement;
    custom.addEventListener('change', () => {
      const v = custom.value.trim();
      if (v !== '' && !isNaN(parseFloat(v))) {
        deps.setParams({ ...deps.getParams(), percent: v });
        load();
      }
    });
    const target = el('input', {
      class: 'input', type: 'number', min: '0', step: '0.01',
      placeholder: 'Precio objetivo (total con IVA)',
      value: deps.getParams().target,
    }) as HTMLInputElement;
    target.addEventListener('change', () => {
      deps.setParams({ ...deps.getParams(), target: target.value });
      load();
    });
    const apply = el('button', { class: 'btn btn-outline' }, 'Aplicar');
    apply.addEventListener('click', () => {
      deps.setParams({ ...deps.getParams(), target: target.value });
      load();
    });
    const clear = el('button', { class: 'icon-btn', title: 'Quitar precio objetivo' }, '✕');
    clear.addEventListener('click', () => {
      deps.setParams({ ...deps.getParams(), target: '' });
      load();
    });
    return el('div', { class: 'toolbar' },
      el('span', { class: 'muted' }, 'Ganancia:'), ...presetBtns, custom,
      el('span', { class: 'toolbar-sep' }), target, apply, clear,
    );
  }

  function renderLines(r: ProfitReport) {
    const hasTarget = r.difference !== null;
    const rows = r.lines.map((l: ProfitLine) =>
      el('tr', {},
        el('td', {},
          el('div', { class: 'desc', title: l.description }, l.description),
          el('div', { class: 'sub' }, l.sku)),
        el('td', { class: 'num' }, `×${l.quantity}`),
        el('td', { class: 'num' }, mxn(l.cost)),
        el('td', { class: 'num' }, mxn(hasTarget ? l.final_profit : l.base_profit)),
        hasTarget
          ? el('td', { class: `num adj ${parseFloat(l.adjustment) < 0 ? 'adj-neg' : 'adj-pos'}` },
              `${parseFloat(l.adjustment) > 0 ? '+' : ''}${mxn(l.adjustment)}`)
          : null,
        el('td', { class: 'num' }, mxn(l.iva)),
        el('td', { class: 'num strong' }, mxn(l.price)),
      ),
    );
    const t = r.totals;
    return el('div', { class: 'table-wrap' },
      el('table', { class: 'catalog-table' },
        el('thead', {}, el('tr', {},
          el('th', {}, 'Producto'), el('th', { class: 'num' }, 'Cant.'),
          el('th', { class: 'num' }, 'Costo (s/IVA)'),
          el('th', { class: 'num' }, `Ganancia (${r.percent}%)`),
          hasTarget ? el('th', { class: 'num' }, 'Ajuste') : null,
          el('th', { class: 'num' }, 'IVA (16%)'),
          el('th', { class: 'num' }, 'Precio'))),
        el('tbody', {},
          ...rows,
          el('tr', { class: 'totals-row' },
            el('td', { class: 'strong' }, 'Sumas'), el('td'),
            el('td', { class: 'num strong' }, mxn(t.cost)),
            el('td', { class: 'num strong' }, mxn(hasTarget ? t.final_profit : t.base_profit)),
            hasTarget
              ? el('td', { class: `num strong ${parseFloat(t.adjustment) < 0 ? 'adj-neg' : 'adj-pos'}` },
                  `${parseFloat(t.adjustment) > 0 ? '+' : ''}${mxn(t.adjustment)}`)
              : null,
            el('td', { class: 'num strong' }, mxn(t.iva)),
            el('td', { class: 'num strong grand-total' }, mxn(t.price))),
        ),
      ),
    );
  }

  function renderSerialEditor() {
    if (units.length === 0) return el('span');
    const inputs: HTMLInputElement[] = [];
    const table = el('table', { class: 'catalog-table serial-table' },
      el('thead', {}, el('tr', {},
        el('th', {}, 'Producto'), el('th', {}, 'N/S (opcional)'), el('th', { class: 'num' }, 'Costo'))),
      el('tbody', {},
        ...units.map((u) => {
          const input = el('input', {
            class: 'input serial-input', placeholder: 'Número de serie (si aplica)',
            value: u.serial,
          }) as HTMLInputElement;
          input.addEventListener('input', () => {
            u.serial = input.value.trim();
          });
          inputs.push(input);
          return el('tr', {},
            el('td', {},
              el('div', { class: 'desc' }, u.description),
              el('div', { class: 'sub' }, u.sku)),
            el('td', {}, input),
            el('td', { class: 'num strong' }, mxn(u.price)));
        }),
      ),
    );
    return table;
  }

  function saleSheet(): HTMLElement {
    const total = units.reduce((n, u) => n + parseFloat(u.price), 0);
    const totalR = round2(total);
    const subtotal = round2(totalR / 1.16);
    const iva = round2(totalR - subtotal);
    const body = units.map((u) =>
      el('tr', {},
        el('td', { class: 'q-type' }, TYPE_LABELS[u.kind] ?? u.kind),
        el('td', { class: 'q-desc' }, el('span', { class: 'q-desc-text' }, u.description)),
        el('td', { class: 'q-ns' }, u.serial || ' '),
        el('td', { class: 'q-cost num' }, mxn(u.price))),
    );
    return el('div', { class: 'quote-sheet sale-sheet', id: 'direct-sheet' },
      el('div', { class: 'q-topline' }),
      el('div', { class: 'q-header' },
        el('div', { class: 'q-head-left' },
          el('div', { class: 'q-brand' }, 'NYKNIU'),
          el('div', { class: 'q-title' }, 'NOTA DE VENTA'),
          el('div', { class: 'q-number' }, `#${saleNumber ?? '——————'}`),
          el('a', { class: 'q-mail', href: 'mailto:ventas@nykniu.com' }, 'ventas@nykniu.com'),
        ),
        el('img', { class: 'q-logo', src: logoUrl, alt: 'NYKNIU — Just Incredible' }),
      ),
      el('table', { class: 'q-table' },
        el('thead', {}, el('tr', {},
          el('th', { class: 'q-type' }, 'Tipo'), el('th', { class: 'q-desc' }, 'Componente'),
          el('th', { class: 'q-ns' }, 'N/S'), el('th', { class: 'q-cost num' }, 'Costo'))),
        el('tbody', {}, ...body),
      ),
      el('table', { class: 'q-table q-totals' },
        el('tbody', {},
          el('tr', {},
            el('td', { class: 'q-type q-totlabel' }, 'Subtotal'),
            el('td', { class: 'q-totvalue num' }, mxn(String(subtotal)))),
          el('tr', {},
            el('td', { class: 'q-type q-totlabel' }, 'IVA'),
            el('td', { class: 'q-totvalue num' }, mxn(String(iva)))),
          el('tr', {},
            el('td', { class: 'q-type q-totlabel q-total-label' }, 'Total'),
            el('td', { class: 'q-totvalue num q-total-value' }, mxn(String(totalR)))),
        ),
      ),
      el('div', { class: 'q-legal' }, ...LEYENDAS_NV.map((t) => el('p', {}, t))),
      el('div', { class: 'q-signoff' },
        el('p', {}, 'Atentamente'),
        el('div', { class: 'q-signline' }, 'Nykniu')),
    );
  }

  async function captureSheet(): Promise<HTMLCanvasElement> {
    const sheet = document.getElementById('direct-sheet');
    if (!sheet) throw new Error('No hay nota que exportar');
    const html2canvas = (await import('html2canvas-pro')).default;
    return html2canvas(sheet, { scale: 2, backgroundColor: '#ffffff' });
  }

  async function saveBlob(blob: Blob, defaultName: string) {
    const isTauri = '__TAURI_INTERNALS__' in window;
    if (isTauri) {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const path = await save({ defaultPath: defaultName });
      if (!path) return;
      const b64 = await blobToBase64(blob);
      await api.saveExport(path, b64);
      flashMsg = `Guardado en ${path}`;
    } else {
      const url = URL.createObjectURL(blob);
      const a = el('a', { href: url, download: defaultName });
      document.body.append(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      flashMsg = `Descargado ${defaultName}`;
    }
    render();
  }

  function blobToBase64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const dataUrl = String(reader.result);
        resolve(dataUrl.slice(dataUrl.indexOf(',') + 1));
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
  }

  async function exportPdf() {
    try {
      const canvas = await captureSheet();
      const { jsPDF } = await import('jspdf');
      const pdf = new jsPDF({ orientation: 'p', unit: 'pt', format: 'letter' });
      const pw = pdf.internal.pageSize.getWidth();
      const ph = pdf.internal.pageSize.getHeight();
      const margin = 36;
      const w = pw - margin * 2;
      const h = (canvas.height / canvas.width) * w;
      if (h <= ph - margin * 2) {
        pdf.addImage(canvas.toDataURL('image/png'), 'PNG', margin, margin, w, h);
      } else {
        const pageHpx = Math.floor(((ph - margin * 2) * canvas.width) / w);
        let y = 0;
        let first = true;
        while (y < canvas.height) {
          const sliceH = Math.min(pageHpx, canvas.height - y);
          const slice = document.createElement('canvas');
          slice.width = canvas.width;
          slice.height = sliceH;
          slice.getContext('2d')!.drawImage(canvas, 0, y, canvas.width, sliceH, 0, 0, canvas.width, sliceH);
          if (!first) pdf.addPage();
          pdf.addImage(slice.toDataURL('image/png'), 'PNG', margin, margin, w, (sliceH / canvas.width) * w);
          first = false;
          y += sliceH;
        }
      }
      await saveBlob(pdf.output('blob'), `NotaVenta-${saleNumber ?? 'borrador'}.pdf`);
    } catch (e) {
      flashMsg = `Error al exportar PDF: ${e}`;
      render();
    }
  }

  async function exportPng() {
    try {
      const canvas = await captureSheet();
      const blob: Blob | null = await new Promise((res) => canvas.toBlob(res, 'image/png'));
      if (!blob) throw new Error('No se pudo generar la imagen');
      await saveBlob(blob, `NotaVenta-${saleNumber ?? 'borrador'}.png`);
    } catch (e) {
      flashMsg = `Error al exportar PNG: ${e}`;
      render();
    }
  }

  async function issue() {
    if (units.length === 0) {
      flashMsg = 'El carrito está vacío.';
      render();
      return;
    }
    if (!saleNumber) {
      flashMsg = 'Asigna el número de nota de venta antes de emitirla.';
      render();
      return;
    }
    try {
      await api.recordSaleNote({
        number: saleNumber,
        quote_number: null,
        assembly_id: null,
        lines: units.map((u) => ({
          component_id: u.componentId,
          sku: u.sku,
          description: u.description,
          kind: TYPE_LABELS[u.kind] ?? u.kind,
          serial: u.serial,
          price: u.price,
        })),
      });
      flashMsg = `Nota de venta individual ${saleNumber} emitida y guardada.`;
      render();
    } catch (e) {
      flashMsg = String(e);
      render();
    }
  }

  /** Chip de procedencia del precio (+ SOLO CD si aplica). */
  function procChip(c: ComponentDto): HTMLElement {
    const label = c.supplier === 'cva' ? 'CVA' : c.supplier === 'arroba' ? '@' : 'MANUAL';
    return el('span', { class: `chip chip-supplier supplier-${c.supplier}` },
      label,
      c.cd_only_best ? el('span', { class: 'chip chip-cd-only' }, 'SOLO CD') : null);
  }

  function render() {
    const children: HTMLElement[] = [];
    if (flashMsg) children.push(el('p', { class: 'q-flash' }, flashMsg));
    children.push(
      el('h2', { class: 'view-title' }, 'Venta individual'),
      renderSearch(),
      renderCart(),
    );
    if (deps.cart().length > 0 && report) {
      children.push(
        renderProfitControls(),
        renderLines(report),
        el('h2', { class: 'view-title' }, 'Números de serie (opcional)'),
        renderSerialEditor(),
      );
      const numberBtn = el('button', { class: 'btn btn-outline' },
        saleNumber ? `Número: ${saleNumber} (asignar otro)` : '# Asignar número de nota de venta');
      numberBtn.addEventListener('click', async () => {
        try {
          saleNumber = await api.nextSaleNoteNumber();
          render();
        } catch (e) {
          flashMsg = String(e);
          render();
        }
      });
      const issueBtn = el('button', { class: 'btn btn-primary' }, '✓ Emitir nota de venta');
      issueBtn.addEventListener('click', issue);
      const pdfBtn = el('button', { class: 'btn btn-primary' }, '⤓ Exportar PDF');
      pdfBtn.addEventListener('click', exportPdf);
      const pngBtn = el('button', { class: 'btn btn-outline' }, '⤓ Exportar PNG');
      pngBtn.addEventListener('click', exportPng);
      children.push(
        el('div', { class: 'toolbar' }, numberBtn, issueBtn, pdfBtn, pngBtn,
          el('span', { class: 'muted' }, 'Sin cotización: venta directa.')),
        el('h2', { class: 'view-title' }, 'Vista previa de la nota'),
        saleSheet(),
      );
    } else if (deps.cart().length > 0 && !report) {
      children.push(el('p', { class: 'form-error' }, flashMsg || 'Calculando…'));
    }
    view.replaceChildren(...children);
  }

  render();
  return view;
}
