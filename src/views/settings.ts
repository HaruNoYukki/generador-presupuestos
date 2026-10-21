// Vista de ajustes: tipo de cambio, catálogo (SQLite) e importación.
import { api } from '../api';
import { RateInfo } from '../types';
import { el } from '../ui';

export function createSettingsView(onChanged: () => void): HTMLElement {
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
        onChanged();
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
        xlsxMsg.className = 'ok-note';
        xlsxMsg.textContent =
          `${summary.supplier}: ${summary.updated} productos actualizados por SKU, ` +
          `${summary.inserted} nuevos · total ${summary.total}.`;
        onChanged();
        await render();
      } catch (e) {
        xlsxMsg.textContent = String(e);
      }
    });

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

    view.replaceChildren(
      el('div', { class: 'settings-grid' },
        el('div', { class: 'card' },
          el('h3', {}, 'Tipo de cambio USD → MXP'),
          el('p', { class: 'muted' }, info.exchange_rule),
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
          el('h3', {}, 'Catálogo (SQLite)'),
          el('div', { class: 'rate-rows' },
            row('Archivo fuente', info.source_file),
            row('Fecha de la lista', info.list_date),
          ),
          xlsxBtn,
          xlsxMsg,
          el('p', { class: 'muted' },
            'Actualiza el catálogo directo desde el Excel del proveedor: se detecta ' +
            'el formato por sus encabezados y la importación usa el SKU como clave ' +
            '(los productos existentes se actualizan conservando su identidad; los ' +
            'nuevos se agregan). El inventario que se considera para vender es ' +
            'A-CEN + AVE-2 + CEDIS.'),
          el('div', { class: 'card' },
            el('h3', {}, 'Catálogos del producto'),
            el('p', { class: 'muted' },
              'Valores permitidos en los formularios de alta. Agrega aquí los valores nuevos de cada catálogo; los imports de listas también los registran automáticamente.'),
            el('div', { class: 'form-row' },
              el('label', {}, 'Catálogo', catSel),
            ),
            el('div', { class: 'catalog-values' }, 'Cargando…'),
            el('div', { class: 'form-row' },
              el('label', {}, 'Nuevo valor', newValue),
              catAdd,
            ),
            catMsg,
          ),
          el('div', { class: 'card' },
            el('h3', {}, 'Títulos con IA (Z.ai)'),
            el('p', { class: 'muted' },
              'Los proveedores formatean las descripciones de manera distinta. Con una clave API de Z.ai normalizas los títulos en lotes de 100: cada corrida procesa solo los pendientes — los ya normalizados quedan excluidos y no se reenvían.'),
            el('div', { class: 'form-row' }, el('label', {}, 'Clave API', zaiKey)),
            zaiMsg,
            el('div', { class: 'form-actions' }, zaiSave, zaiImprove),
          ),
          el('div', { class: 'form-title' }, 'Avanzado'),
          importBtn,
          importMsg,
        ),
      ),
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
