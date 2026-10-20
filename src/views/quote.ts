// Vista de Cotización (fase 4): formato NYKNIU.
// Encabezado con logo y número DDMMAA-NC, tabla Tipo/Componente/Ud./Costo,
// Subtotal + IVA + Total, leyendas de garantía y "Atentamente". Exporta a
// PDF o PNG.
import { api } from '../api';
import { Assembly, ProfitLine, ProfitReport } from '../types';
import { el, typeChip } from '../ui';
import logoUrl from '../assets/nykniu-logo.png';

export interface QuoteViewDeps {
  getSelectedAssembly: () => Assembly | null;
  getParams: () => { percent: string; target: string };
  /** Refresca el shell (el folio vive en el ensamble). */
  onChanged?: () => void;
}

/** Etiquetas de tipo tal como aparecen en la cotización NYKNIU. */
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

const LEYENDAS = [
  'Agradecemos su preferencia y confianza. Esperamos seguir siendo su elección de confianza para sus necesidades futuras.',
  'Nos enorgullece informarte que todos nuestros componentes son completamente nuevos y vienen respaldados por una garantía de un año. Además, queremos asegurarte que tu satisfacción está garantizada, ya que ofrecemos un período de garantía de 730 días naturales para las solicitudes de venta en ensambles. Es importante tener en cuenta que nuestras garantías se aplican exclusivamente a defectos de fabricación y/o servicio, y no cubren daños físicos, problemas causados por software no original, malware, ransomware, entre otros.',
  'Para realizar una reclamación de garantía, es fundamental contar con la nota o factura de compra conservar el empaque original del producto y la integridad de los sellos de garantía. Asimismo, requerimos llevar a cabo una revisión previa antes de iniciar el proceso de garantía, asegurando que se cumplan los términos y condiciones correspondientes.',
  'Tu confianza en nuestros productos y servicios es de suma importancia para nosotros, y nos esforzamos por garantizar tu satisfacción en cada paso del camino. Si tienes alguna pregunta o necesitas asistencia adicional con respecto a nuestras garantías, no dudes en ponerte en contacto con nuestro equipo de atención al cliente. Estaremos encantados de ayudarte en todo lo que necesites.',
];

// El orden de renglones de la cotización sigue el flujo del ensamble.
const LINE_ORDER = [
  'cpu', 'motherboard', 'ram', 'storage', 'psu', 'case', 'cooling',
  'gpu', 'monitor', 'keyboard', 'mouse', 'expansion_card', 'other',
];

export function createQuoteView(deps: QuoteViewDeps): HTMLElement {
  const view = el('section', { class: 'view quote-view' });
  let report: ProfitReport | null = null;
  let error = '';
  const quoteNumber = () => selected()?.quote_number ?? null;

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
    const num = parseFloat(n);
    return num.toLocaleString('es-MX', { style: 'currency', currency: 'MXN' });
  }

  // ------------------------------------------------------------------
  // Hoja de cotización (lo que se exporta)
  // ------------------------------------------------------------------
  function quoteSheet(r: ProfitReport, asmName: string): HTMLElement {
    const byKind = new Map<string, ProfitLine[]>();
    // El reporte trae líneas por slot; rescata el tipo desde el catálogo
    // de la vista de ensamble: aquí usamos el orden del reporte + SKU.
    // Para las etiquetas, agrupamos por la descripción del tipo en meta.
    for (const l of r.lines) {
      const k = kindOfLine(l);
      if (!byKind.has(k)) byKind.set(k, []);
      byKind.get(k)!.push(l);
    }

    const tbody: HTMLElement[] = [];
    for (const k of LINE_ORDER) {
      const lines = byKind.get(k);
      if (!lines) continue;
      for (const l of lines) {
        tbody.push(
          el('tr', {},
            el('td', { class: 'q-type' }, TYPE_LABELS[k] ?? k),
            el('td', { class: 'q-desc' },
              el('span', { class: 'q-desc-text' }, l.description),
              asmName ? el('span', { class: 'q-asm' }, ` / ${asmName}`) : null),
            el('td', { class: 'q-ud num' }, String(l.quantity)),
            el('td', { class: 'q-cost num' }, fmt(l.price)),
          ),
        );
      }
    }

    // Formato NYKNIU: la columna Costo incluye el IVA de cada renglón
    // (las líneas suman el Total); el Subtotal y el IVA se desglosan del
    // total final (Total/1.16 y el complemento), así Subtotal + IVA =
    // Total coincide siempre — con o sin precio objetivo.
    const total = r.totals.price;
    const subtotal = Math.round((parseFloat(total) / 1.16) * 100) / 100;
    const iva = Math.round((parseFloat(total) - subtotal) * 100) / 100;
    const subtotalFmt = fmt(String(subtotal));

    return el('div', { class: 'quote-sheet', id: 'quote-sheet' },
      el('div', { class: 'q-topline' }),
      el('div', { class: 'q-header' },
        el('div', { class: 'q-head-left' },
          el('div', { class: 'q-brand' }, 'NYKNIU'),
          el('div', { class: 'q-title' }, 'Cotización'),
          el('div', { class: 'q-number' }, `#${quoteNumber() ?? '——————'}`),
          el('a', { class: 'q-mail', href: 'mailto:ventas@nykniu.com' }, 'ventas@nykniu.com'),
          el('div', { class: 'q-seller' }, 'Ventas'),
        ),
        el('img', { class: 'q-logo', src: logoUrl, alt: 'NYKNIU — Just Incredible' }),
      ),
      el('table', { class: 'q-table' },
        el('thead', {},
          el('tr', {},
            el('th', { class: 'q-type' }, 'Tipo'),
            el('th', { class: 'q-desc' }, 'Componente'),
            el('th', { class: 'q-ud num' }, 'Ud.'),
            el('th', { class: 'q-cost num' }, 'Costo'),
          ),
        ),
        el('tbody', {}, ...tbody),
      ),
      el('table', { class: 'q-table q-totals' },
        el('tbody', {},
          el('tr', {},
            el('td', { class: 'q-type q-totlabel' }, 'Subtotal'),
            el('td', { class: 'q-totvalue num' }, subtotalFmt),
          ),
          el('tr', {},
            el('td', { class: 'q-type q-totlabel' }, 'IVA'),
            el('td', { class: 'q-totvalue num' }, fmt(String(iva))),
          ),
          el('tr', {},
            el('td', { class: 'q-type q-totlabel q-total-label' }, 'Total'),
            el('td', { class: 'q-totvalue num q-total-value' }, fmt(r.totals.price)),
          ),
        ),
      ),
      el('div', { class: 'q-legal' },
        ...LEYENDAS.map((t) => el('p', {}, t)),
      ),
      el('div', { class: 'q-signoff' },
        el('p', {}, 'Atentamente'),
        el('div', { class: 'q-signline' }, 'Nykniu'),
      ),
    );
  }

  /** Tipo del renglón: lo deduce del catálogo vía el ensamble seleccionado. */
  function kindOfLine(_l: ProfitLine): string {
    // El backend no envía el kind en ProfitLine; el ensamble seleccionado
    // mapea component_id → kind desde la vista.
    const asm = selected();
    if (asm) {
      for (const [slot, items] of Object.entries(asm.slots)) {
        if (items?.some((i) => i.component_id === _l.component_id)) {
          return slot;
        }
      }
    }
    return 'other';
  }

  // ------------------------------------------------------------------
  // Exportación
  // ------------------------------------------------------------------
  async function captureSheet(): Promise<HTMLCanvasElement> {
    const sheet = document.getElementById('quote-sheet');
    if (!sheet) throw new Error('No hay cotización que exportar');
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
      flash(`Guardado en ${path}`);
    } else {
      const url = URL.createObjectURL(blob);
      const a = el('a', { href: url, download: defaultName });
      document.body.append(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      flash(`Descargado ${defaultName}`);
    }
  }

  function flash(msg: string) {
    const n = el('p', { class: 'q-flash' }, msg);
    view.prepend(n);
    window.setTimeout(() => n.remove(), 4000);
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
        // Multipágina: recorta el canvas en franjas del alto de página.
        const pageHpx = Math.floor(((ph - margin * 2) * canvas.width) / w);
        let y = 0;
        let first = true;
        while (y < canvas.height) {
          const sliceH = Math.min(pageHpx, canvas.height - y);
          const slice = document.createElement('canvas');
          slice.width = canvas.width;
          slice.height = sliceH;
          slice.getContext('2d')!.drawImage(
            canvas, 0, y, canvas.width, sliceH, 0, 0, canvas.width, sliceH,
          );
          if (!first) pdf.addPage();
          pdf.addImage(slice.toDataURL('image/png'), 'PNG', margin, margin, w, (sliceH / canvas.width) * w);
          first = false;
          y += sliceH;
        }
      }
      const blob = pdf.output('blob');
      await saveBlob(blob, `Cotizacion-${quoteNumber() ?? 'borrador'}.pdf`);
    } catch (e) {
      flash(`Error al exportar PDF: ${e}`);
    }
  }

  async function exportPng() {
    try {
      const canvas = await captureSheet();
      const blob: Blob | null = await new Promise((res) => canvas.toBlob(res, 'image/png'));
      if (!blob) throw new Error('No se pudo generar la imagen');
      await saveBlob(blob, `Cotizacion-${quoteNumber() ?? 'borrador'}.png`);
    } catch (e) {
      flash(`Error al exportar PNG: ${e}`);
    }
  }

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------
  function render() {
    const asm = selected();
    if (!asm) {
      view.replaceChildren(
        el('div', { class: 'panel-muted' },
          'Confirma un ensamble en la pestaña Ensamble para generar su cotización.'),
      );
      return;
    }
    if (error) {
      view.replaceChildren(el('p', { class: 'form-error' }, error));
      return;
    }
    if (!report) {
      view.replaceChildren(el('p', { class: 'muted' }, 'Generando cotización…'));
      return;
    }

    const numberBtn = el('button', { class: 'btn btn-outline' },
      quoteNumber()
        ? `Número: ${quoteNumber()} (asignar otro)`
        : '# Asignar número de cotización');
    numberBtn.addEventListener('click', async () => {
      const asm = selected();
      if (!asm) return;
      try {
        await api.assignQuoteNumber(asm.id);
        await load();
        deps.onChanged?.();
      } catch (e) {
        flash(String(e));
      }
    });

    const pdfBtn = el('button', { class: 'btn btn-primary' }, '⤓ Exportar PDF');
    pdfBtn.addEventListener('click', exportPdf);
    const pngBtn = el('button', { class: 'btn btn-outline' }, '⤓ Exportar PNG');
    pngBtn.addEventListener('click', exportPng);

    view.replaceChildren(
      el('div', { class: 'toolbar' },
        numberBtn, pdfBtn, pngBtn,
        typeChip(selected()?.quote_type),
        el('span', { class: 'muted' },
          `Ganancia ${deps.getParams().percent}% · se ajusta en la pestaña Ganancias`),
      ),
      quoteSheet(report, asm.name),
    );
  }

  load();
  return view;
}
