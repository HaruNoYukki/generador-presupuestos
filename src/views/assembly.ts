// Vista de ensamble: constructor de equipos con validación en vivo.
import { api } from '../api';
import { SLOTS, SlotMeta, kindLabel } from '../meta';
import { Assembly, AssemblySummary, ComponentDto, CostBreakdown, ValidationReport } from '../types';
import { closeModal, el, modal, mxn } from '../ui';
import { openProductForm } from './product_form';

export interface AssemblyViewDeps {
  components: ComponentDto[];
  refreshComponents: () => Promise<ComponentDto[]>;
  /** Notifica el ensamble seleccionado (para la vista de ganancias). */
  onSelected?: (asm: Assembly | null) => void;
  /** Se llama tras confirmar un ensamble (para saltar a Ganancias). */
  onConfirmed?: () => void;
}

export function createAssemblyView(deps: AssemblyViewDeps): HTMLElement {
  const view = el('section', { class: 'view assembly-view' });
  let assemblies: AssemblySummary[] = [];
  // Filtros de cotizaciones: preensambles vigentes y búsqueda por folio.
  let onlyVigentePreensambles = false;
  let searchNumber = '';
  let current: Assembly | null = null;
  let report: ValidationReport | null = null;
  let cost: CostBreakdown | null = null;

  const byId = () => {
    const map = new Map<string, ComponentDto>();
    for (const c of deps.components) map.set(c.id, c);
    return map;
  };

  async function loadAssemblies(selectId?: string) {
    assemblies = await api.listAssemblies();
    const target =
      (selectId && assemblies.find((a) => a.id === selectId)) || assemblies[0];
    if (target) {
      await select(target.id);
    } else {
      current = null;
      report = null;
      cost = null;
      render();
    }
  }

  async function select(id: string) {
    current = await api.getAssembly(id);
    [report, cost] = await Promise.all([
      api.validateAssembly(id),
      api.assemblyCost(id),
    ]);
    deps.onSelected?.(current);
    render();
  }

  async function mutate(p: Promise<unknown>, assemblyId?: string) {
    try {
      await p;
    } catch (e) {
      // P. ej. "Sin existencia para ensamble": se muestra y se recarga el
      // ensamble igual, por si el catálogo cambió debajo.
      const banner = el('p', { class: 'form-error confirm-msg' }, String(e));
      view.prepend(banner);
      window.setTimeout(() => banner.remove(), 6000);
    }
    await loadAssemblies(assemblyId ?? current?.id);
  }

  /** Existencia para ensamble del renglón: la oferta fijada o la suma. */
  function availableFor(c: ComponentDto | undefined, supplier?: string | null): number {
    if (!c) return 0;
    if (supplier) return c.offers?.find((o) => o.supplier === supplier)?.stock_ensamble ?? 0;
    return c.stock_ensamble_total;
  }

  // ------------------------------------------------------------------
  // Selector de componentes (modal) para un slot
  // ------------------------------------------------------------------
  async function openPicker(slot: SlotMeta) {
    const search = el('input', {
      class: 'input search',
      placeholder: `Buscar en ${slot.label.toLowerCase()}…`,
    }) as HTMLInputElement;

    // Filtro por marca (catálogo de marcas de la base)
    const filters = await api.catalogFilters().catch(() => ({ brands: [] as string[], departments: [] as string[] }));
    const brandSel = el('select', { class: 'input' },
      el('option', { value: '' }, 'Todas las marcas'),
      ...filters.brands.map((b) => el('option', { value: b }, b)),
    ) as HTMLSelectElement;
    let brandFilter = '';
    brandSel.addEventListener('change', () => {
      brandFilter = brandSel.value;
      drawList();
    });

    const list = el('div', { class: 'picker-list' });
    // Orden por defecto: precio de menor a mayor (en MXN para comparar
    // entre monedas). Las SODIMM son para laptop: se excluyen.
    const comps = deps.components
      .filter((c) => slot.kinds.includes(c.kind as never))
      .filter((c) => !(c.kind === 'ram' && c.specs?.ram_form === 'SODIMM'))
      .sort((a, b) => parseFloat(a.price_mxp.amount) - parseFloat(b.price_mxp.amount));

    function drawList() {
      const q = search.value.trim().toUpperCase();
      const rows = comps.filter(
        (c) =>
          (!q ||
            c.description.toUpperCase().includes(q) ||
            c.sku.toUpperCase().includes(q) ||
            c.brand.toUpperCase().includes(q)) &&
          (brandFilter === '' || c.brand === brandFilter),
      );
      list.replaceChildren(
        ...rows.slice(0, 60).map((c) => {
          const item = existingQty(c.id);
          // En ensambles solo pueden usarse productos con existencia de
          // prioridad (arroba CEN/AVE-2/CEDIS, CVA Disp, PCH/manual a mano).
          // Los SOLO-CD de CVA y los AGOTADOS se muestran bloqueados.
          const cdOnly = c.cd_only_best === true;
          const sinStock = c.stock_ensamble_total <= 0;
          const noEnsamble = cdOnly || sinStock;
          const supplierChip = el('span', { class: `chip chip-supplier supplier-${c.supplier || 'arroba'}` },
            c.supplier === 'cva' ? 'CVA' : c.supplier === 'arroba' ? '@' : (c.supplier || 'manual').toUpperCase());
          const blockChip = cdOnly
            ? el('span', { class: 'chip chip-cd-only' }, 'SOLO CD · no ensamble')
            : sinStock
              ? el('span', { class: 'chip chip-danger' }, 'SIN EXISTENCIA')
              : null;
          const others = (c.offers ?? []).filter((o) => o.supplier !== c.supplier);
          const chosenSupplier = item?.supplier ?? null;

          const offerLines = (c.offers ?? []).map((o) => {
            const oLabel = o.supplier === 'cva' ? 'CVA' : o.supplier === 'arroba' ? '@' : o.supplier.toUpperCase();
            const elegido = chosenSupplier ? chosenSupplier === o.supplier : c.supplier === o.supplier;
            const usable = !(o.cd_only && o.stock_ensamble === 0) && o.stock_ensamble > 0;
            const useBtn = usable
              ? el('button', { class: 'btn btn-sm btn-outline offer-use' }, elegido ? '✓ Usando' : 'Usar este')
              : el('span', { class: 'chip chip-danger' }, o.stock_ensamble === 0 ? 'sin existencia' : 'solo CD');
            if (usable && current) {
              const currentId = current.id;
              useBtn.addEventListener('click', async (e) => {
                e.stopPropagation();
                await mutate(api.setAssemblyItem(currentId, slot.id, c.id, 1, o.supplier));
                closeModal();
              });
            }
            return el('div', { class: 'offer-line' },
              el('span', { class: `chip chip-supplier supplier-${o.supplier}` }, oLabel),
              el('span', { class: 'strong' }, mxn(o.price.amount)),
              el('span', { class: 'sub' }, o.stock_label),
              useBtn);
          });

          const row = el('div', { class: `picker-row${item ? ' picked' : ''}${noEnsamble ? ' disabled' : ''}` },
            el('div', { class: 'picker-desc' },
              el('div', { class: 'desc' }, c.description),
              el('div', { class: 'sub' },
                [c.sku, c.brand, kindLabel(c.kind), c.condition === 'usado' ? 'USADO' : null,
                 `disponible ensamble: ${c.stock_ensamble_total}`]
                  .filter(Boolean)
                  .join(' · ')),
              // Opción de ver/cambiar proveedores aunque otro sea más barato
              others.length > 0
                ? el('button', { class: 'btn btn-sm btn-outline offer-toggle' },
                    `Ver otros proveedores (${others.length})`)
                : null,
              el('div', { class: 'offers-box hidden' }, ...offerLines),
            ),
            el('div', { class: 'picker-side' },
              el('div', { class: 'strong' }, mxn(c.price_mxp.amount), supplierChip),
              el('div', { class: 'sub' }, money0(c)),
              blockChip,
              item
                ? el('button', { class: 'btn btn-ghost btn-sm' }, `En ensamble (${item.qty})`)
                : null,
            ),
          );
          // Toggle de ofertas de otros proveedores
          const toggle = row.querySelector('.offer-toggle');
          if (toggle) {
            toggle.addEventListener('click', (e) => {
              e.stopPropagation();
              row.querySelector('.offers-box')?.classList.toggle('hidden');
            });
          }
          if (noEnsamble && !item) {
            row.addEventListener('click', (e) => {
              e.stopPropagation();
            });
          } else {
            // Un componente ya en el ensamble siempre se puede quitar,
            // aunque se haya agotado después de elegirlo.
            row.addEventListener('click', async () => {
              if (!current) return;
              const currentQty = existingQty(c.id);
              const next = slot.multiple ? (currentQty ? 0 : 1) : currentQty ? 0 : 1;
              await mutate(
                api.setAssemblyItem(current.id, slot.id, c.id, next),
              );
              closeModal();
            });
          }
          return row;
        }),
      );
      if (!rows.length) {
        list.replaceChildren(el('p', { class: 'muted' }, 'Sin resultados.'));
      }
    }

    function existingQty(id: string): { qty: number; supplier?: string | null } | null {
      if (!current) return null;
      const items = current.slots[slot.id] ?? [];
      const it = items.find((i) => i.component_id === id);
      return it ? { qty: it.quantity, supplier: it.supplier ?? null } : null;
    }

    function money0(c: ComponentDto): string {
      return c.price.currency === 'USD' ? `US$${c.price.amount}` : '';
    }

    // Botón para limpiar la búsqueda: algunos entornos no disparan 'input'
    // al vaciar el campo, y el usuario necesita volver a la lista completa.
    const clearBtn = el('button', { class: 'icon-btn', title: 'Limpiar búsqueda' }, '✕');
    clearBtn.addEventListener('click', () => {
      search.value = '';
      drawList();
      search.focus();
    });
    search.addEventListener('input', drawList);
    search.addEventListener('change', drawList);

    // Alta de productos desde el propio picker (p. ej. captura manual de
    // proveedores sin Excel como PCH). Al guardar se auto-selecciona.
    const addNewBtn = el('button', { class: 'btn btn-outline' },
      `＋ Agregar ${slot.label.toLowerCase()} nuevo`);
    addNewBtn.addEventListener('click', () => {
      openProductForm({
        title: `Agregar ${slot.label.toLowerCase()} nuevo`,
        fixedKinds: slot.kinds,
        defaultKind: slot.kinds[0],
        defaultSupplier: 'pch',
        onSaved: async (dto) => {
          await deps.refreshComponents();
          if (current) {
            await mutate(api.setAssemblyItem(current.id, slot.id, dto.id, 1, dto.supplier));
          }
        },
      });
    });

    drawList();
    modal(
      `Elegir ${slot.label.toLowerCase()}`,
      el('div', {},
        el('div', { class: 'picker-search' }, search, clearBtn),
        el('div', { class: 'toolbar' }, el('span', { class: 'muted' }, 'Marca:'), brandSel),
        el('div', { class: 'toolbar' }, addNewBtn),
        list),
    );
  }

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------
  function renderSlot(slot: SlotMeta) {
    const items = current?.slots[slot.id] ?? [];
    const map = byId();
    const filled = items.length > 0;

    const body = el('div', { class: 'slot-items' });
    for (const item of items) {
      const c = map.get(item.component_id);
      const available = availableFor(c, item.supplier);
      const outOfStock = item.quantity > available;
      const qtyControls = slot.multiple
        ? [
            el('button', { class: 'icon-btn', title: 'Quitar una unidad' }, '−'),
            el('button', {
              class: 'icon-btn',
              title: item.quantity >= available ? `Sin más existencia (disponible ${available})` : 'Agregar una unidad',
              disabled: item.quantity >= available,
            }, '+'),
          ]
        : [];
      if (slot.multiple) {
        qtyControls[0].addEventListener('click', () =>
          current && mutate(api.setAssemblyItem(current.id, slot.id, item.component_id, item.quantity - 1, item.supplier)),
        );
        qtyControls[1].addEventListener('click', () =>
          current && mutate(api.setAssemblyItem(current.id, slot.id, item.component_id, item.quantity + 1, item.supplier)),
        );
      }
      const removeBtn = el('button', { class: 'icon-btn', title: 'Quitar' }, '✕');
      removeBtn.addEventListener('click', () =>
        current && mutate(api.setAssemblyItem(current.id, slot.id, item.component_id, 0, item.supplier)),
      );

      body.append(
        el('div', { class: `slot-item${outOfStock ? ' out-of-stock' : ''}` },
          el('div', { class: 'slot-item-info' },
            el('div', { class: 'desc', title: c?.description },
              c ? c.description : `Componente eliminado (${item.component_id.slice(0, 8)}…)`,
              outOfStock
                ? el('span', { class: 'chip chip-danger' },
                    available === 0 ? 'SIN EXISTENCIA' : `SOLO ${available} DISPONIBLE${available === 1 ? '' : 'S'}`)
                : null,
            ),
            c
              ? el('div', { class: 'sub' },
                  [c.sku, c.brand, mxn(c.price_mxp.amount),
                   item.supplier ? `proveedor ${item.supplier}` : null,
                   `disponible ${available}`]
                    .filter(Boolean).join(' · '))
              : null,
          ),
          el('div', { class: 'slot-item-actions' },
            // En slots unitarios la cantidad siempre es 1: el badge sobra.
            slot.multiple || item.quantity !== 1
              ? el('span', { class: 'qty' }, `×${item.quantity}`)
              : null,
            ...qtyControls,
            removeBtn,
          ),
        ),
      );
    }
    if (!filled) {
      body.append(
        el('div', { class: 'slot-empty' },
          slot.required ? 'Obligatorio' : 'Opcional',
          slot.note ? el('span', { class: 'sub' }, ` — ${slot.note}`) : null,
        ),
      );
    }

    // Estado especial del slot PSU cuando el gabinete cubre la fuente
    let psuNote: HTMLElement | null = null;
    if (slot.id === 'psu' && report && report.psu.kind === 'included_in_case') {
      psuNote = el('div', { class: 'slot-note ok' },
        `Cubierta por la fuente del gabinete: ${report.psu.detail}`);
    } else if (slot.id === 'psu' && report && report.psu.kind === 'missing') {
      psuNote = el('div', { class: 'slot-note bad' },
        'Falta fuente: agrega una dedicada o elige un gabinete que incluya una.');
    }

    const addBtn = el('button', { class: 'btn btn-sm btn-outline' }, '+ Agregar');
    addBtn.addEventListener('click', () => current && openPicker(slot));

    let clearBtn: HTMLElement | null = null;
    if (filled) {
      clearBtn = el('button', { class: 'icon-btn', title: 'Vaciar slot' }, '🗑');
      clearBtn.addEventListener('click', () =>
        current && mutate(api.clearAssemblySlot(current.id, slot.id)),
      );
    }

    // El slot PSU está "cubierto" también cuando la fuente viene del gabinete.
    const covered =
      slot.id === 'psu'
        ? report
          ? report.psu.kind !== 'missing'
          : filled
        : filled;

    return el('div', { class: `slot${covered ? ' filled' : ''}` },
      el('div', { class: 'slot-head' },
        el('h4', {},
          slot.label,
          slot.required
            ? (covered
                ? el('span', { class: 'chip chip-ok' }, '✓ cubierto')
                : el('span', { class: 'chip chip-req' }, 'obligatorio'))
            : el('span', { class: 'chip chip-opt' }, 'opcional'),
        ),
        clearBtn,
        addBtn,
      ),
      psuNote,
      body,
    );
  }

  function renderValidation() {
    if (!report) {
      return el('div', { class: 'panel-muted' }, 'Crea un ensamble para validar.');
    }
    const badge = el('span', {
      class: `chip ${report.is_valid ? 'chip-ok' : 'chip-bad'}`,
    }, report.is_valid ? '✓ Equipo completo' : '✗ Incompleto');

    // El aviso "fuente cubierta por gabinete" ya se muestra como nota verde
    // en el slot y en la fila de cobertura: no repetirlo una tercera vez.
    const issueNodes = report.issues
      .filter((i) => i.code !== 'psu_from_case')
      .map((i) =>
      el('div', { class: `issue issue-${i.severity}` },
        el('span', { class: 'issue-dot' }),
        i.message,
      ),
    );

    const coverage = (title: string, text: string) =>
      el('div', { class: 'coverage' },
        el('span', { class: 'muted' }, title),
        el('span', {}, text),
      );

    const psuText =
      report.psu.kind === 'dedicated'
        ? 'Fuente dedicada'
        : report.psu.detail ?? 'Sin fuente';
    const coolText = report.cooling.detail ?? 'Sin enfriamiento';

    return el('div', { class: 'validation-panel' },
      el('h3', {}, 'Validación ', badge),
      coverage('Fuente de poder', psuText),
      coverage('Enfriamiento', coolText),
      issueNodes.length
        ? el('div', { class: 'issues' }, ...issueNodes)
        : el('p', { class: 'muted' }, 'Sin observaciones: el equipo cumple los mínimos.'),
    );
  }

  const STATUS_LABELS: Record<string, string> = {
    vigente: 'Vigente',
    en_fabricacion: 'En fabricación',
    caducada: 'Caducada',
  };

  function applyFilters(list: AssemblySummary[]): AssemblySummary[] {
    return list.filter((a) => {
      if (onlyVigentePreensambles && !(a.quote_type === 'preensamble' && a.status === 'vigente')) {
        return false;
      }
      if (searchNumber.trim() !== '') {
        const q = searchNumber.trim().replace('#', '');
        if (!(a.quote_number ?? '').includes(q)) return false;
      }
      return true;
    });
  }

  /** Advertencia de productos PCH cuando el ensamble está en fabricación:
   * la existencia de PCH no se valida por sistema (captura a mano, sin
   * Excel), así que hay que verificarla manualmente. */
  function renderPchWarning() {
    if (!current) return null;
    const map = byId();
    const pchItems: { description: string; sku: string }[] = [];
    for (const items of Object.values(current.slots)) {
      for (const item of items ?? []) {
        const c = map.get(item.component_id);
        if (c && c.supplier === 'pch') {
          pchItems.push({ description: c.description, sku: c.sku });
        }
      }
    }
    if (pchItems.length === 0) return null;
    const lineas = pchItems.map((p) =>
      el('div', { class: 'pch-line' },
        el('strong', {}, p.description),
        el('span', {}, ` — Con SKU: ${p.sku} — Por favor verifica existencia`)),
    );
    return el('div', { class: 'pch-banner', role: 'alert' },
      el('strong', {}, '⚠ Advertencia'),
      el('div', {},
        el('span', {},
          'Este ensamble tiene el siguiente producto de PCH: '),
        ...lineas),
    );
  }

  function renderTypeBanner() {
    if (!current) return null;
    const isPre = current.quote_type === 'preensamble';
    const chips: (Node | string)[] = [];
    const summary = assemblies.find((a) => a.id === current!.id);
    if (summary) {
      chips.push(el('span', { class: `status-chip status-${summary.status}` },
        STATUS_LABELS[summary.status]));
      chips.push(el('span', { class: 'banner-meta' }, `vence ${summary.expiry_date}`));
      if (summary.quote_number) {
        chips.push(el('span', { class: 'banner-meta' }, `folio #${summary.quote_number}`));
      }
    }
    if (current.based_on) {
      chips.push(el('span', { class: 'banner-meta based-on' },
        `⤴ basado en el preensamble: ${current.based_on}`));
    }
    return el('div', { class: `type-banner type-${current.quote_type}` },
      el('span', { class: 'type-chip' }, isPre ? 'PREENSAMBLE' : 'PERSONALIZADO'),
      el('span', { class: 'banner-name' }, current.name),
      ...chips,
      el('span', { class: 'banner-hint' },
        isPre
          ? 'plantilla para uno o varios clientes'
          : current.based_on
            ? 'cotización única generada desde un preensamble'
            : 'cotización única'),
    );
  }

  function renderHeader() {
    const filtered = applyFilters(assemblies);

    // Filtros: preensambles vigentes + búsqueda por número de cotización
    const vigenteCheck = el('input', { type: 'checkbox' }) as HTMLInputElement;
    vigenteCheck.checked = onlyVigentePreensambles;
    vigenteCheck.addEventListener('change', () => {
      onlyVigentePreensambles = vigenteCheck.checked;
      render();
    });
    const numberSearch = el('input', {
      class: 'input',
      placeholder: 'Buscar por # de cotización…',
      value: searchNumber,
    }) as HTMLInputElement;
    numberSearch.addEventListener('input', () => {
      searchNumber = numberSearch.value;
      // Re-render solo del selector para no perder el foco del buscador
      refreshSelector(filtered);
    });
    numberSearch.addEventListener('change', () => {
      searchNumber = numberSearch.value;
      refreshSelector(filtered);
    });
    const clearSearch = el('button', { class: 'icon-btn', title: 'Limpiar búsqueda' }, '✕');
    clearSearch.addEventListener('click', () => {
      searchNumber = '';
      numberSearch.value = '';
      refreshSelector(filtered);
      numberSearch.focus();
    });

    const sel = el('select', { class: 'input' }) as HTMLSelectElement;
    function fillSelector(list: AssemblySummary[]) {
      sel.replaceChildren();
      for (const a of list) {
        const tipo = a.quote_type === 'preensamble' ? 'Preensamble' : 'Personalizado';
        const opt = el('option', { value: a.id },
          `${tipo} · ${a.quote_number ? `#${a.quote_number} · ` : ''}${a.name} ` +
          `[${STATUS_LABELS[a.status]}]` +
          (a.confirmed ? ' ✓' : a.is_valid ? '' : ` (${a.missing_count} pendientes)`) +
          ` · vence ${a.expiry_date}`);
        sel.append(opt);
      }
      // Si el ensamble en edición quedó fuera del filtro, se muestra al
      // final marcado, para que las acciones no operen a escondidas.
      if (current && !list.some((a) => a.id === current!.id)) {
        const hidden = assemblies.find((a) => a.id === current!.id);
        if (hidden) {
          sel.append(el('option', { value: hidden.id },
            `— (fuera del filtro) ${hidden.name} [${STATUS_LABELS[hidden.status]}]`));
        }
      }
      if (current) sel.value = current.id;
    }
    function refreshSelector(_prev?: AssemblySummary[]) {
      fillSelector(applyFilters(assemblies));
    }
    fillSelector(filtered);
    sel.addEventListener('change', () => select(sel.value));

    // Confirmar: guarda el ensamble como cotización (fase 3)
    const confirmBtn = el('button', { class: 'btn btn-primary' }, '✓ Confirmar');
    confirmBtn.addEventListener('click', async () => {
      if (!current) return;
      try {
        await api.confirmAssembly(current.id);
        await loadAssemblies(current.id);
        deps.onConfirmed?.();
      } catch (e) {
        const banner = el('p', { class: 'form-error confirm-msg' }, String(e));
        view.prepend(banner);
        window.setTimeout(() => banner.remove(), 5000);
      }
    });

    const newBtn = el('button', { class: 'btn btn-primary' }, '+ Nueva cotización');
    newBtn.addEventListener('click', async () => {
      const name = el('input', {
        class: 'input',
        placeholder: 'Nombre del equipo o cliente (p. ej. Oficina Básica)',
      }) as HTMLInputElement;
      let chosen: 'preensamble' | 'personalizado' | null = null;
      const cardPre = el('div', { class: 'quote-card card-pre' },
        el('div', { class: 'quote-card-title' }, 'PREENSAMBLE'),
        el('p', { class: 'muted' },
          'Plantilla para uno o varios clientes. Caduca el día 1 del mes siguiente.'),
      );
      const cardPer = el('div', { class: 'quote-card card-per' },
        el('div', { class: 'quote-card-title' }, 'PERSONALIZADO'),
        el('p', { class: 'muted' },
          'Cotización única para un cliente. Caduca 15 días naturales después de crearse.'),
      );
      const select = (which: 'preensamble' | 'personalizado') => {
        chosen = which;
        cardPre.classList.toggle('selected', which === 'preensamble');
        cardPer.classList.toggle('selected', which === 'personalizado');
        btn.disabled = false;
      };
      cardPre.addEventListener('click', () => select('preensamble'));
      cardPer.addEventListener('click', () => select('personalizado'));

      const create = async () => {
        if (!chosen) return;
        const asm = await api.createAssembly(
          name.value.trim() || 'Equipo sin nombre',
          chosen,
        );
        closeModal();
        await loadAssemblies(asm.id);
      };
      name.addEventListener('keydown', (e) => e.key === 'Enter' && chosen && create());
      const btn = el('button', { class: 'btn btn-primary', disabled: true },
        'Selecciona un tipo…') as HTMLButtonElement;
      btn.addEventListener('click', create);
      modal('Nueva cotización', el('div', { class: 'form' },
        el('div', { class: 'quote-cards' }, cardPre, cardPer),
        el('div', { class: 'form-row' }, el('label', {}, 'Nombre', name)),
        el('div', { class: 'form-actions' }, btn),
      ));
    });

    // Generar cotización personalizada desde este ensamble
    const dupBtn = el('button', {
      class: 'btn btn-outline',
      title: 'Crea una cotización PERSONALIZADA nueva usando este ensamble como base (nace sin folio y sin confirmar)',
    },
      current?.quote_type === 'preensamble'
        ? '⧉ Generar personalizado desde este preensamble'
        : '⧉ Duplicar como personalizado');
    dupBtn.addEventListener('click', async () => {
      if (!current) return;
      const copy = await api.duplicateAssembly(current.id);
      await loadAssemblies(copy.id);
    });

    // En fabricación: bloquea la caducidad y permite vender aunque pase la fecha
    const fabBtn = el('button', { class: 'btn btn-outline' },
      current?.in_fabrication ? '⏵ Quitar fabricación' : '⚙ En fabricación');
    fabBtn.addEventListener('click', async () => {
      if (!current) return;
      await api.setFabrication(current.id, !current.in_fabrication);
      await loadAssemblies(current.id);
    });

    const renameBtn = el('button', { class: 'btn btn-outline' }, 'Renombrar');
    renameBtn.addEventListener('click', () => {
      if (!current) return;
      const name = el('input', { class: 'input', value: current.name }) as HTMLInputElement;
      const doRename = async () => {
        await api.renameAssembly(current!.id, name.value.trim() || current!.name);
        closeModal();
        await loadAssemblies(current!.id);
      };
      const btn = el('button', { class: 'btn btn-primary' }, 'Guardar');
      btn.addEventListener('click', doRename);
      modal('Renombrar ensamble', el('div', { class: 'form' },
        el('div', { class: 'form-row' }, el('label', {}, 'Nombre', name)),
        el('div', { class: 'form-actions' }, btn),
      ));
    });

    const delBtn = el('button', { class: 'btn btn-danger' }, 'Eliminar');
    delBtn.addEventListener('click', async () => {
      if (!current) return;
      await api.deleteAssembly(current.id);
      await loadAssemblies();
    });

    return el('div', { class: 'filters-wrap' },
      current ? renderTypeBanner() : null,
      current?.in_fabrication ? renderPchWarning() : null,
      el('div', { class: 'toolbar filter-bar' },
        el('label', { class: 'filter-check' }, vigenteCheck, ' Solo preensambles vigentes'),
        numberSearch,
        clearSearch,
      ),
      el('div', { class: 'toolbar' },
        sel, confirmBtn, fabBtn, dupBtn, newBtn, renameBtn, delBtn,
      ),
    );
  }

  function renderCost() {
    if (!cost) {
      return el('div', { class: 'panel-muted' }, 'Sin costo: crea un ensamble.');
    }
    return el('div', { class: 'cost-panel' },
      el('h3', {}, 'Costo del ensamble'),
      el('div', { class: 'cost-row' },
        el('span', { class: 'muted' }, 'Subtotal'),
        el('span', {}, mxn(cost.subtotal.amount)),
      ),
      el('div', { class: 'cost-row' },
        el('span', { class: 'muted' }, 'IVA (16%)'),
        el('span', {}, mxn(cost.iva.amount)),
      ),
      el('div', { class: 'cost-row cost-total-row' },
        el('span', {}, 'Total'),
        el('span', { class: 'cost-total' }, mxn(cost.total.amount)),
      ),
      el('p', { class: 'sub' },
        'Precios convertidos a MXN con el T.C efectivo.',
      ),
    );
  }

  function render() {
    if (!current) {
      view.replaceChildren(
        renderHeader(),
        el('div', { class: 'panel-muted' },
          'Aún no hay ensambles. Crea uno con “+ Nuevo ensamble”.'),
      );
      return;
    }
    view.replaceChildren(
      renderHeader(),
      el('div', { class: 'assembly-grid' },
        el('div', { class: 'slots-col' }, ...SLOTS.map(renderSlot)),
        el('aside', { class: 'report-col' }, renderValidation(), renderCost()),
      ),
    );
  }

  // El botón de vaciar slot necesita su listener; se cablea en renderSlot
  async function boot() {
    await loadAssemblies();
  }
  boot();

  return view;
}
