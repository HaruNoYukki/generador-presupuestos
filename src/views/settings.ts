// Vista de ajustes: tipo de cambio, catálogo (SQLite) e importación.
import { api } from '../api';
import { kindLabel } from '../meta';
import { OutOfStockHit, RateInfo, XlsxImportSummary } from '../types';
import { el } from '../ui';

// El último informe de importación vive fuera de la vista: `onChanged`
// vuelve a renderizar el shell (y con él esta vista), y el informe debe
// seguir visible hasta la siguiente importación.
let lastXlsx: { summary: XlsxImportSummary; at: Date } | null = null;
let lastCva: { hits: OutOfStockHit[]; at: Date; resumen: string } | null = null;

/** Informe post-import: componentes de ensambles vigentes sin existencia. */
function renderImportReport(
  hits: OutOfStockHit[],
  meta: { supplier: string; zeroed: number },
): HTMLElement {
  const head = el('div', { class: 'report-head' },
    el('h4', {}, 'Informe de existencias tras importar'),
    el('span', { class: 'muted small' },
      `${meta.supplier} · ${meta.zeroed} SKU agotados por no venir en la lista`),
  );
  if (hits.length === 0) {
    return el('div', { class: 'import-report ok' },
      head,
      el('p', { class: 'ok-note' }, 'Ningún preensamble ni personalizado vigente quedó sin existencias.'),
    );
  }
  const newly = hits.filter((h) => h.newly_out).length;
  const rows = hits.map((h) => {
    const asms = h.assemblies.map((a) =>
      el('li', {},
        el('span', { class: `chip-type ${a.quote_type}` },
          a.quote_type === 'preensamble' ? 'PRE' : 'PER'),
        ' ',
        a.quote_number ? `#${a.quote_number} · ` : '',
        a.name,
        el('span', { class: 'muted' }, ` (pide ${a.quantity})`),
      ));
    return el('div', { class: `report-row ${h.newly_out ? 'newly-out' : ''}` },
      el('div', { class: 'report-comp' },
        h.newly_out
          ? el('span', { class: 'chip chip-danger' }, 'SE ACABA DE AGOTAR')
          : el('span', { class: 'chip chip-warn' }, 'YA ESTABA CORTO'),
        el('span', { class: `chip chip-${h.kind}` }, kindLabel(h.kind)),
        el('span', { class: 'strong' }, h.description),
        el('span', { class: 'sub' },
          `${h.sku} · disponible ahora: ${h.stock_after}` +
          (h.supplier ? ` (proveedor fijado: ${h.supplier})` : '')),
      ),
      el('ul', { class: 'report-asms' }, ...asms),
    );
  });
  return el('div', { class: 'import-report warn' },
    head,
    el('p', { class: 'form-error' },
      `${hits.length} componente(s) sin existencia suficiente en ensambles vigentes` +
      (newly ? ` · ${newly} se acaban de agotar con esta lista` : '') +
      '. Esos ensambles quedan con pendientes hasta cambiar el componente.'),
    ...rows,
  );
}

export function createSettingsView(onChanged: () => void | Promise<void>): HTMLElement {
  const view = el('section', { class: 'view settings-view' });

  async function render() {
    const info: RateInfo = await api.getRate();

    const input = el('input', {
      class: 'input',
      type: 'number',
      step: '0.01',
      min: '0.01',
      value: info.rate.pesos_per_dollar,
    }) as HTMLInputElement;

    const msg = el('p', { class: 'form-error' });
    const save = el('button', { class: 'btn btn-primary' }, 'Guardar T.C.');
    save.addEventListener('click', async () => {
      msg.textContent = '';
      try {
        await api.setRate(input.value);
        onChanged();
        await render();
      } catch (e) {
        msg.textContent = String(e);
      }
    });

    // Importar catálogo: JSON (mismo formato que catalog_seed.json).
    // La base hace UPSERT por SKU: actualiza existentes e inserta nuevos.
    const importMsg = el('p', { class: 'form-error' });
    const importBtn = el('button', { class: 'btn btn-primary' },
      '⤒ Importar catálogo (JSON)');
    importBtn.addEventListener('click', async () => {
      importMsg.textContent = '';
      importMsg.className = 'form-error';
      try {
        const isTauri = '__TAURI_INTERNALS__' in window;
        if (!isTauri) {
          throw new Error('Importar catálogo solo está disponible en la aplicación de escritorio.');
        }
        const { open } = await import('@tauri-apps/plugin-dialog');
        const path = await open({
          multiple: false,
          filters: [{ name: 'Catálogo JSON', extensions: ['json'] }],
        });
        if (!path || Array.isArray(path)) return;
        const summary = await api.importCatalog(String(path));
        importMsg.className = 'ok-note';
        importMsg.textContent =
          `Catálogo importado: ${summary.updated} actualizados por SKU, ` +
          `${summary.inserted} nuevos · total ${summary.total} productos.`;
        await onChanged();
        await render();
      } catch (e) {
        importMsg.textContent = String(e);
      }
    });

    // Importar Excel: abre el libro, detecta el proveedor por sus
    // encabezados y hace UPSERT por SKU.
    const xlsxMsg = el('p', { class: 'form-error' });
    const xlsxBtn = el('button', { class: 'btn btn-primary' },
      '⤒ Importar Excel del proveedor (.xlsx)');
    xlsxBtn.addEventListener('click', async () => {
      xlsxMsg.textContent = '';
      xlsxMsg.className = 'form-error';
      try {
        const isTauri = '__TAURI_INTERNALS__' in window;
        if (!isTauri) {
          throw new Error('Importar solo está disponible en la aplicación de escritorio.');
        }
        const { open } = await import('@tauri-apps/plugin-dialog');
        const path = await open({
          multiple: false,
          filters: [{ name: 'Lista de precios (CVA .xls/.xlsx, arroba .xlsx)', extensions: ['xlsx', 'xlsm', 'xls'] }],
        });
        if (!path || Array.isArray(path)) return;
        xlsxMsg.className = '';
        xlsxMsg.textContent = 'Importando lista…';
        const summary = await api.importCatalogXlsx(String(path));
        lastXlsx = { summary, at: new Date() };
        xlsxMsg.className = 'ok-note';
        xlsxMsg.textContent =
          `${summary.supplier}: ${summary.updated} productos actualizados por SKU, ` +
          `${summary.inserted} nuevos, ${summary.zeroed} agotados por no venir en la lista · total ${summary.total}.`;
        await onChanged();
        await render();
      } catch (e) {
        xlsxMsg.textContent = String(e);
      }
    });

    const xlsxReport = lastXlsx
      ? el('div', {},
          el('p', { class: 'ok-note' },
            `Última importación ${lastXlsx.at.toLocaleString('es-MX')}: ${lastXlsx.summary.supplier} · ` +
            `${lastXlsx.summary.updated} actualizados, ${lastXlsx.summary.inserted} nuevos, ` +
            `${lastXlsx.summary.zeroed} agotados.`),
          renderImportReport(lastXlsx.summary.out_of_stock, lastXlsx.summary),
        )
      : null;

    // Clave API de Z.ai + formateo asistido de títulos
    const zaiKey = el('input', {
      class: 'input',
      type: 'password',
      placeholder: 'Clave API de Z.ai',
      value: info.zai_key ?? '',
    }) as HTMLInputElement;
    const zaiMsg = el('p', { class: 'form-error' });
    const zaiSave = el('button', { class: 'btn btn-outline' }, 'Guardar clave');
    zaiSave.addEventListener('click', async () => {
      zaiMsg.textContent = '';
      try {
        await api.setZaiKey(zaiKey.value.trim());
        zaiMsg.className = 'ok-note';
        zaiMsg.textContent = 'Clave guardada.';
      } catch (e) {
        zaiMsg.textContent = String(e);
      }
    });
    const zaiImprove = el('button', { class: 'btn btn-primary' },
      '✨ Normalizar títulos con IA (lote de 100)');
    zaiImprove.addEventListener('click', async () => {
      zaiMsg.textContent = '';
      try {
        const { improved, remaining } = await api.aiNormalizeTitles(100);
        zaiMsg.className = 'ok-note';
        zaiMsg.textContent = improved > 0
          ? `${improved} títulos normalizados · quedan ${remaining} pendientes. Vuelve a ejecutar para el siguiente lote.`
          : remaining > 0
            ? `Sin cambios en este lote · quedan ${remaining} pendientes.`
            : 'Todos los títulos del catálogo ya están normalizados.';
        onChanged();
      } catch (e) {
        zaiMsg.textContent = String(e);
      }
    });

    // Feed XML de CVA: número de cliente (la auth es por IP) + importación
    // de los grupos de ensamble. Horario del servicio: 19:01–08:59.
    const ASSEMBLY_GROUPS = [
      'gabinetes', 'monitores', 'teclado y mouse', 'mouse', 'procesadores',
      'tarjeta madre', 'teclados', 'video', 'memorias', 'discos duros', 'energia',
    ];
    const feedCfg = await api.getCvaFeedConfig().catch(() => null);
    const feedCliente = el('input', {
      class: 'input',
      inputmode: 'numeric',
      placeholder: 'Número de cliente (p. ej. 74247)',
      value: feedCfg?.cliente ?? '',
    }) as HTMLInputElement;
    const feedMsg = el('p', { class: 'form-error' });
    const feedSave = el('button', { class: 'btn btn-outline' }, 'Guardar número de cliente');
    feedSave.addEventListener('click', async () => {
      feedMsg.textContent = '';
      try {
        await api.setCvaFeedConfig({ cliente: feedCliente.value.trim() });
        feedMsg.className = 'ok-note';
        feedMsg.textContent = 'Número de cliente guardado.';
      } catch (e) {
        feedMsg.textContent = String(e);
      }
    });

    // Importación por grupos de ensamble
    const groupChecks: { name: string; box: HTMLInputElement }[] = ASSEMBLY_GROUPS.map((g) => {
      const box = el('input', { type: 'checkbox', checked: true }) as HTMLInputElement;
      return { name: g, box };
    });
    const marcaInput = el('input', {
      class: 'input',
      placeholder: 'Marca (opcional, p. ej. HP)',
    }) as HTMLInputElement;
    const cvaImportMsg = el('p', { class: 'form-error' });
    const cvaImportBtn = el('button', { class: 'btn btn-primary' },
      '⤓ Importar de CVA (grupos seleccionados)');
    cvaImportBtn.addEventListener('click', async () => {
      cvaImportMsg.textContent = '';
      const grupos = groupChecks.filter((g) => g.box.checked).map((g) => g.name);
      if (grupos.length === 0) {
        cvaImportMsg.textContent = 'Selecciona al menos un grupo.';
        return;
      }
      const isTauri = '__TAURI_INTERNALS__' in window;
      if (!isTauri) {
        cvaImportMsg.textContent = 'Importar del feed CVA solo está disponible en la aplicación de escritorio.';
        return;
      }
      cvaImportBtn.disabled = true;
      cvaImportMsg.className = '';
      cvaImportMsg.textContent = 'Consultando el feed de CVA… (el servicio solo responde de 19:01 a 08:59)';
      try {
        const summary = await api.importCatalogCva(grupos, marcaInput.value.trim() || null);
        const gruposTxt = summary.groups
          .map(([g, n]) => `${g}: ${n}`)
          .join(' · ');
        const resumen =
          `CVA: ${summary.updated} actualizados por SKU, ${summary.inserted} nuevos, ` +
          `${summary.zeroed} agotados` +
          (summary.skipped > 0 ? `, ${summary.skipped} omitidos por precio inválido` : '') +
          ` · total ${summary.total}.` +
          (gruposTxt ? ` Descargados por grupo — ${gruposTxt}.` : '');
        lastCva = { hits: summary.out_of_stock, at: new Date(), resumen };
        await onChanged();
        await render();
      } catch (e) {
        cvaImportMsg.textContent = String(e);
      } finally {
        cvaImportBtn.disabled = false;
      }
    });
    const cvaReport = lastCva
      ? el('div', {},
          el('p', { class: 'ok-note small' }, `${lastCva.at.toLocaleString('es-MX')} — ${lastCva.resumen}`),
          renderImportReport(lastCva.hits, { supplier: 'CVA', zeroed: 0 }),
        )
      : null;

    // Catálogos de atributos normalizados: agregar valores por separado
    const CATALOGS: { key: string; label: string }[] = [
      { key: 'marca', label: 'Marca' },
      { key: 'socket', label: 'Socket' },
      { key: 'ddr', label: 'DDR' },
      { key: 'form_factor', label: 'Factor de forma' },
      { key: 'ram_form', label: 'Formato de memoria RAM' },
    ];
    let catalogoSel = 'marca';
    let valores: string[] = [];

    const catSel = el('select', { class: 'input' },
      ...CATALOGS.map((c) => el('option', { value: c.key }, c.label)),
    ) as HTMLSelectElement;
    const valoresBox = el('div', { class: 'catalog-values' });
    const newValue = el('input', { class: 'input', placeholder: 'Nuevo valor' }) as HTMLInputElement;
    const catMsg = el('p', { class: 'form-error' });

    async function refreshCatalogList() {
      valores = catalogoSel === 'marca'
        ? (await api.catalogFilters().catch(() => ({ brands: [] }))).brands
        : await api.listSpecOptions(catalogoSel).catch(() => []);
      valoresBox.replaceChildren(
        ...valores.map((v) => el('span', { class: 'chip chip-supplier' }, v)),
        valores.length === 0 ? el('span', { class: 'muted' }, 'Sin valores') : el('span'),
      );
    }
    catSel.addEventListener('change', async () => {
      catalogoSel = catSel.value;
      await refreshCatalogList();
    });

    const catAdd = el('button', { class: 'btn btn-outline' }, 'Agregar valor');
    catAdd.addEventListener('click', async () => {
      catMsg.textContent = '';
      const v = newValue.value.trim();
      if (!v) return;
      try {
        if (catalogoSel === 'marca') {
          await api.addBrand(v);
        } else {
          await api.addSpecOption(catalogoSel, v);
        }
        newValue.value = '';
        await refreshCatalogList();
        catMsg.className = 'ok-note';
        catMsg.textContent = `"${v}" agregado al catálogo ${CATALOGS.find(c => c.key === catalogoSel)?.label}.`;
        onChanged();
      } catch (e) {
        catMsg.textContent = String(e);
      }
    });

    await refreshCatalogList();

    // Dos columnas balanceadas: inventario/imports a la izquierda (uso
    // diario), configuración a la derecha.
    const colInventario = el('div', { class: 'settings-col' },
      el('div', { class: 'card' },
        el('h3', {}, 'Catálogo e inventario'),
        el('div', { class: 'rate-rows' },
          row('Archivo fuente', info.source_file),
          row('Fecha de la lista', info.list_date),
        ),
        xlsxBtn,
        xlsxMsg,
        xlsxReport,
        el('p', { class: 'muted small' },
          'UPSERT por SKU: los existentes se actualizan conservando su identidad; los nuevos se agregan. Con arroba y CVA, los SKU ausentes quedan agotados (último precio visible). Las correcciones manuales no se pisan. Vendible = A-CEN + AVE-2 + CEDIS.'),
      ),
      el('div', { class: 'card' },
        el('h3', {}, 'Feed XML de CVA'),
        el('p', { class: 'muted small' },
          'Importación directa del catálogo de CVA. Coloca tu número de cliente y recuerda dar de alta la IP de este equipo en el portal de CVA. El servicio solo responde de 19:01 a 08:59 h.'),
        el('div', { class: 'form-row' }, el('label', {}, 'Número de cliente', feedCliente)),
        feedMsg,
        el('div', { class: 'form-actions' }, feedSave),
        el('div', { class: 'form-title' }, 'Importar grupos de ensamble'),
        el('p', { class: 'muted small' },
          'Dentro de memorias y energía, lo que no es de ensamble (USB, flash, UPS…) se importa como tipo «otro»: vendible en venta individual, nunca en ensambles. Las promociones aplican como precio efectivo y quedan registradas.'),
        el('div', { class: 'group-checks' },
          ...groupChecks.map(({ name, box }) =>
            el('label', { class: 'filter-check' }, box, ` ${name}`))),
        el('div', { class: 'form-row' }, el('label', {}, 'Marca (opcional)', marcaInput)),
        cvaImportMsg,
        el('div', { class: 'form-actions' }, cvaImportBtn),
        cvaReport,
      ),
      el('div', { class: 'card' },
        el('h3', {}, 'Avanzado'),
        el('p', { class: 'muted small' },
          'Importar un catálogo JSON (mismo formato que catalog_seed.json). La base hace UPSERT por SKU.'),
        importBtn,
        importMsg,
      ),
    );

    const colConfig = el('div', { class: 'settings-col' },
      el('div', { class: 'card' },
        el('h3', {}, 'Tipo de cambio USD → MXP'),
        el('p', { class: 'muted small' }, info.exchange_rule),
        el('div', { class: 'rate-rows' },
          row('T.C de la lista', `${info.list_rate.pesos_per_dollar} (lista ${info.list_date})`),
          row('T.C capturado', info.rate.pesos_per_dollar),
          row('T.C efectivo aplicado', `${info.effective_rate.pesos_per_dollar} por dólar`),
          row('Origen', info.is_overridden ? 'Capturado por el usuario' : 'Lista de precios'),
        ),
        el('div', { class: 'form-row' },
          el('label', {}, 'Capturar otro T.C', input),
        ),
        msg,
        el('div', { class: 'form-actions' }, save),
      ),
      el('div', { class: 'card' },
        el('h3', {}, 'Catálogos del producto'),
        el('p', { class: 'muted small' },
          'Valores permitidos en los formularios de alta. Los imports de listas también registran automáticamente cualquier valor nuevo.'),
        el('div', { class: 'form-row' },
          el('label', {}, 'Catálogo', catSel),
        ),
        valoresBox,
        el('div', { class: 'form-row' },
          el('label', {}, 'Nuevo valor', newValue),
          catAdd,
        ),
        catMsg,
      ),
      el('div', { class: 'card' },
        el('h3', {}, 'Títulos con IA (Z.ai)'),
        el('p', { class: 'muted small' },
          'Normaliza los títulos del catálogo en lotes de 100: cada corrida procesa solo los pendientes; los ya normalizados y los editados a mano quedan excluidos.'),
        el('div', { class: 'form-row' }, el('label', {}, 'Clave API', zaiKey)),
        zaiMsg,
        el('div', { class: 'form-actions' }, zaiSave, zaiImprove),
      ),
    );

    view.replaceChildren(
      el('div', { class: 'settings-grid' }, colInventario, colConfig),
    );
  }

  function row(label: string, value: string) {
    return el('div', { class: 'rate-row' },
      el('span', { class: 'muted' }, label),
      el('span', { class: 'strong' }, value),
    );
  }

  render();
  return view;
}
