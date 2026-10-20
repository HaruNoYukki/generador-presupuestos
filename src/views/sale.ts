// Vista Nota de Venta (fase 5): formato del NV.pdf. Ligada a la cotización
// del ensamble confirmado (mismos parámetros de ganancia), con una línea
// por unidad y su número de serie capturable. Exporta PDF/PNG.
import { api } from '../api';
import { Assembly, ProfitLine, ProfitReport } from '../types';
import { el, mxn, typeChip } from '../ui';
import logoUrl from '../assets/nykniu-logo.png';

export interface SaleViewDeps {
  getSelectedAssembly: () => Assembly | null;
  getParams: () => { percent: string; target: string };
}

const TYPE_LABELS: Record<string, string> = {
  cpu: 'Procesador',
  motherboard: 'Motherboard',
  ram: 'RAM',
  storage: 'Almacenamiento',
  psu: 'PSU',
  case: 'Gabinete',
  cooling: 'Enfriamiento',
  gpu: 'GPU',
  monitor: 'Monitor',
  keyboard: 'Teclado',
  mouse: 'Mouse',
  keyboard_mouse_kit: 'Kit teclado + mouse',
  expansion_card: 'Tarjeta de expansión',
  other: 'Otro',
};

// Leyendas de la NOTA DE VENTA (versión del NV.pdf, textual).
const LEYENDAS_NV = [
  'Agradecemos su preferencia y confianza. Esperamos seguir siendo su elección de confianza para sus necesidades futuras.',
  'Nos enorgullece informarte que todos nuestros componentes son completamente nuevos y vienen respaldados por una garantía de un año. Además, queremos asegurarte que tu satisfacción está garantizada, ya que ofrecemos un período de garantía de 365 días naturales para las solicitudes de venta de productos. Es importante tener en cuenta que nuestras garantías se aplican exclusivamente a defectos de fabricación y/o servicio, y no cubren daños físicos, problemas causados por software no original, malware, ransomware, entre otros.',
  'Para realizar una reclamación de garantía, es fundamental contar con la nota o factura de compra y conservar el empaque original del producto. Asimismo, requerimos llevar a cabo una revisión previa antes de iniciar el proceso de garantía, asegurando que se cumplan los términos y condiciones correspondientes.',
  'Tu confianza en nuestros productos y servicios es de suma importancia para nosotros, y nos esforzamos por garantizar tu satisfacción en cada paso del camino. Si tienes alguna pregunta o necesitas asistencia adicional con respecto a nuestras garantías, no dudes en ponerte en contacto con nuestro equipo de atención al cliente. Estaremos encantados de ayudarte en todo lo que necesites.',
];

interface UnitRow {
  key: string;
  componentId: string;
  sku: string;
  description: string;
  kind: string;
  price: string; // precio unitario con IVA
  serial: string;
}

function round2(n: number): number {
  return Math.round(n * 100) / 100;
}

export function createSaleView(deps: SaleViewDeps): HTMLElement {
  const view = el('section', { class: 'view sale-view' });
  let report: ProfitReport | null = null;
  let unitRows: UnitRow[] = [];
  let saleNumber: string | null = null;
  let error = '';
  let flashMsg = '';

  const selected = () => {
    const a = deps.getSelectedAssembly();
    return a && a.confirmed_at ? a : null;
  };

  /** Expande cada renglón de la cotización en una fila por unidad; el
   * precio unitario reparte el precio del renglón y la última unidad
   * absorbe el residuo de centavos (la suma cierra igual que la cotización). */
  function buildUnitRows(lines: ProfitLine[], kindOf: (id: string) => string) {
    const rows: UnitRow[] = [];
    for (const l of lines) {
      const q = l.quantity;
      const unit = round2(parseFloat(l.price) / q);
      for (let i = 0; i < q; i++) {
        const price = i === q - 1
          ? round2(parseFloat(l.price) - unit * (q - 1))
          : unit;
        rows.push({
          key: `${l.component_id}#${i}`,
          componentId: l.component_id,
          sku: l.sku,
          description: l.description,
          kind: kindOf(l.component_id),
          price: price.toFixed(2),
          serial: '',
        });
      }
    }
    return rows;
  }

  async function load() {
    error = '';
    const asm = selected();
    if (!asm) {
      report = null;
      unitRows = [];
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
      // Conserva los N/S ya capturados si la vista se re-renderiza.
      const previous = new Map(unitRows.map((r) => [r.key, r.serial]));
      const kindOf = (id: string) => {
        for (const [slot, items] of Object.entries(asm.slots)) {
          if (items?.some((i) => i.component_id === id)) return slot;
        }
        return 'other';
      };
      unitRows = buildUnitRows(report.lines, kindOf);
      for (const r of unitRows) r.serial = previous.get(r.key) ?? '';
    } catch (e) {
      error = String(e);
      report = null;
      unitRows = [];
    }
    render();
  }

  function fmt(n: string): string {
    return parseFloat(n).toLocaleString('es-MX', { style: 'currency', currency: 'MXN' });
  }

  // ------------------------------------------------------------------
  // Hoja de la nota de venta (formato NV.pdf)
  // ------------------------------------------------------------------
  function saleSheet(
    rows: { kind: string; description: string; sku: string; serial: string; price: string }[],
  ): HTMLElement {
    const body = rows.map((r) =>
      el('tr', {},
        el('td', { class: 'q-type' }, TYPE_LABELS[r.kind] ?? r.kind),
        el('td', { class: 'q-desc' }, el('span', { class: 'q-desc-text' }, r.description)),
        el('td', { class: 'q-ns' }, r.serial || ' '),
        el('td', { class: 'q-cost num' }, fmt(r.price)),
      ),
    );

    const total = rows.reduce((n, r) => n + parseFloat(r.price), 0);
    const totalR = round2(total);
    const subtotal = round2(totalR / 1.16);
    const iva = round2(totalR - subtotal);

    return el('div', { class: 'quote-sheet sale-sheet', id: 'sale-sheet' },
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
        el('thead', {},
          el('tr', {},
            el('th', { class: 'q-type' }, 'Tipo'),
            el('th', { class: 'q-desc' }, 'Componente'),
            el('th', { class: 'q-ns' }, 'N/S'),
            el('th', { class: 'q-cost num' }, 'Costo'),
          ),
        ),
        el('tbody', {}, ...body),
      ),
      el('table', { class: 'q-table q-totals' },
        el('tbody', {},
          el('tr', {},
            el('td', { class: 'q-type q-totlabel' }, 'Subtotal'),
            el('td', { class: 'q-totvalue num' }, fmt(String(subtotal))),
          ),
          el('tr', {},
            el('td', { class: 'q-type q-totlabel' }, 'IVA'),
            el('td', { class: 'q-totvalue num' }, fmt(String(iva))),
          ),
          el('tr', {},
            el('td', { class: 'q-type q-totlabel q-total-label' }, 'Total'),
            el('td', { class: 'q-totvalue num q-total-value' }, fmt(String(totalR))),
          ),
        ),
      ),
      el('div', { class: 'q-legal' },
        ...LEYENDAS_NV.map((t) => el('p', {}, t)),
      ),
      el('div', { class: 'q-signoff' },
        el('p', {}, 'Atentamente'),
        el('div', { class: 'q-signline' }, 'Nykniu'),
      ),
    );
  }

  // ------------------------------------------------------------------
  // Captura de números de serie
  // ------------------------------------------------------------------
  function serialEditor(): HTMLElement {
    const pending = unitRows.filter((r) => r.serial.trim() === '').length;
    const hint = pending === 0
      ? el('span', { class: 'ok-note' }, '✓ Todos los números de serie capturados')
      : el('span', { class: 'muted' }, `Faltan ${pending} números de serie por capturar`);

    const table = el('table', { class: 'catalog-table serial-table' },
      el('thead', {},
        el('tr', {},
          el('th', {}, 'Tipo'),
          el('th', {}, 'Componente'),
          el('th', {}, 'N/S (capturar)'),
          el('th', { class: 'num' }, 'Costo unitario'),
        ),
      ),
      el('tbody', {},
        ...unitRows.map((r) => {
          const input = el('input', {
            class: 'input serial-input',
            placeholder: 'Número de serie',
            value: r.serial,
          }) as HTMLInputElement;
          input.addEventListener('input', () => {
            r.serial = input.value.trim();
          });
          return el('tr', {},
            el('td', {}, TYPE_LABELS[r.kind] ?? r.kind),
            el('td', {},
              el('div', { class: 'desc', title: r.description }, r.description),
              el('div', { class: 'sub' }, r.sku),
            ),
            el('td', {}, input),
            el('td', { class: 'num strong' }, fmt(r.price)),
          );
        }),
      ),
    );
    return el('div', {}, el('p', { class: 'serial-hint' }, hint), table);
  }

  // ------------------------------------------------------------------
  // Exportación (misma mecánica que la cotización)
  // ------------------------------------------------------------------
  async function captureSheet(): Promise<HTMLCanvasElement> {
    const sheet = document.getElementById('sale-sheet');
    if (!sheet) throw new Error('No hay nota de venta que exportar');
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
      const img = canvas.toDataURL('image/png');
      if (h <= ph - margin * 2) {
        pdf.addImage(img, 'PNG', margin, margin, w, h);
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

  /** Emite la nota: exige número y todos los N/S; la registra ligada a la
   * cotización y los precios coinciden con ella por construcción. */
  async function issue() {
    const asm = selected();
    if (!asm || !report) return;
    if (!saleNumber) {
      flashMsg = 'Asigna el número de la nota de venta antes de emitirla.';
      render();
      return;
    }
    const missing = unitRows.filter((r) => r.serial.trim() === '').length;
    if (missing > 0) {
      flashMsg = `Faltan ${missing} números de serie por capturar.`;
      render();
      return;
    }
    try {
      await api.recordSaleNote({
        number: saleNumber,
        quote_number: selected()?.quote_number ?? null,
        assembly_id: asm.id,
        lines: unitRows.map((r) => ({
          component_id: r.componentId,
          sku: r.sku,
          description: r.description,
          kind: TYPE_LABELS[r.kind] ?? r.kind,
          serial: r.serial,
          price: r.price,
        })),
      });
      flashMsg = `Nota de venta ${saleNumber} emitida y guardada` +
        ` (cotización: ${selected()?.quote_number ?? 'sin número'}).`;
      render();
    } catch (e) {
      flashMsg = String(e);
      render();
    }
  }

  function flash(text: string): HTMLElement {
    return el('p', { class: 'q-flash' }, text);
  }

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------
  function render() {
    const asm = selected();
    if (!asm) {
      view.replaceChildren(
        el('div', { class: 'panel-muted' },
          'Confirma un ensamble en la pestaña Ensamble para generar su nota de venta.'),
      );
      return;
    }
    if (error) {
      view.replaceChildren(el('p', { class: 'form-error' }, error));
      return;
    }
    if (!report) {
      view.replaceChildren(el('p', { class: 'muted' }, 'Generando nota de venta…'));
      return;
    }

    const numberBtn = el('button', { class: 'btn btn-outline' },
      saleNumber
        ? `Número: ${saleNumber} (asignar otro)`
        : '# Asignar número de nota de venta');
    numberBtn.addEventListener('click', async () => {
      try {
        saleNumber = await api.nextSaleNoteNumber();
        render();
      } catch (e) {
        flashMsg = String(e);
        render();
      }
    });

    const pdfBtn = el('button', { class: 'btn btn-primary' }, '⤓ Exportar PDF');
    pdfBtn.addEventListener('click', exportPdf);
    const pngBtn = el('button', { class: 'btn btn-outline' }, '⤓ Exportar PNG');
    pngBtn.addEventListener('click', exportPng);
    const issueBtn = el('button', { class: 'btn btn-primary' }, '✓ Emitir nota de venta');
    issueBtn.addEventListener('click', issue);

    const quoteNum = selected()?.quote_number ?? null;
    const totalSale = unitRows.reduce((n, r) => n + parseFloat(r.price), 0);

    view.replaceChildren(
      el('div', { class: 'toolbar' },
        numberBtn, issueBtn, pdfBtn, pngBtn,
        typeChip(asm.quote_type),
        el('span', { class: 'muted' },
          `Ligada a la cotización ${quoteNum ? `#${quoteNum}` : '(sin número asignado)'} · ` +
          `Suma de unidades: ${mxn(String(round2(totalSale)))}`),
      ),
      flashMsg ? flash(flashMsg) : el('span'),
      el('h2', { class: 'view-title' }, 'Números de serie por unidad'),
      serialEditor(),
      el('h2', { class: 'view-title' }, 'Vista previa de la nota'),
      saleSheet(unitRows),
    );
  }

  load();
  return view;
}
