// Vista de ensamble: constructor de equipos con validación en vivo.
import { api } from '../api';
import { SLOTS, SlotMeta, kindLabel } from '../meta';
import { Assembly, AssemblyRefreshReport, AssemblySummary, ComponentDto, CostBreakdown, ValidationReport } from '../types';
import { closeModal, el, modal, mxn } from '../ui';
import {
  ArrowUpRight, ChartColumn, Check, ChevronLeft, ChevronRight, Cog, Copy, Globe,
  Minus, PenLine, Play, Plus, RefreshCw, Repeat, Trash, TriangleExclamationPoint, X, icon,
} from '../icons';
import { openProductForm } from './product_form';

export interface AssemblyViewDeps {
  components: ComponentDto[];
  refreshComponents: () => Promise<ComponentDto[]>;
  /** Notifica el ensamble seleccionado (para la vista de ganancias). */
  onSelected?: (asm: Assembly | null) => void;
  /** Se llama tras confirmar un ensamble (para saltar a Ganancias). */
  onConfirmed?: () => void;
  /** Cotización a mostrar al montar la vista. La vista se recrea en cada
   * cambio de sección: sin esto se volvería a elegir la primera. */
  initialAssemblyId?: string | null;
}

export function createAssemblyView(deps: AssemblyViewDeps): HTMLElement {
  const view = el('section', { class: 'view assembly-view' });
  let assemblies: AssemblySummary[] = [];
  // Búsqueda de cotizaciones por folio (el selector ya agrupa por tipo y
  // vigencia, así que no hace falta otro filtro).
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
      // Sin cotizaciones (p. ej. se borró la última): las vistas del flujo
      // no deben seguir mostrando la eliminada.
      deps.onSelected?.(null);
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
      page = 1;
      drawList();
    });

    const list = el('div', { class: 'picker-list' });
    // Orden por defecto: precio de menor a mayor (en MXN para comparar
    // entre monedas). Las SODIMM son para laptop: se excluyen.
    const comps = deps.components
      .filter((c) => slot.kinds.includes(c.kind as never))
      .filter((c) => !(c.kind === 'ram' && c.specs?.ram_form === 'SODIMM'))
      .sort((a, b) => parseFloat(a.price_mxp.amount) - parseFloat(b.price_mxp.amount));

    // Paginación del picker: 20 renglones por página.
    const PAGE_SIZE = 20;
    let page = 1; // página actual; cualquier cambio de filtro regresa a la 1

    // Un componente no puede usarse en ensamble si su mejor oferta es
    // SOLO-CD o si se quedó sin existencia para ensamble.
    function blockedForAssembly(c: ComponentDto): 'cd' | 'stock' | null {
      if (c.cd_only_best === true) return 'cd';
      if (c.stock_ensamble_total <= 0) return 'stock';
      return null;
    }

    // Modo reemplazo: el slot ya contiene un componente que dejó de poder
    // usarse (p. ej. una carga de inventario o la renovación lo agotó o lo
    // volvió SOLO-CD). Se fija arriba como referencia "a reemplazar" y los
    // candidatos se ordenan por cercanía de precio para facilitar elegir
    // el sustituto.
    let toReplace: ComponentDto | null = null;
    let replaceQty = 1;
    let replaceSupplier: string | null = null;
    if (current) {
      for (const it of current.slots[slot.id] ?? []) {
        const c = deps.components.find((x) => x.id === it.component_id);
        if (c && blockedForAssembly(c)) {
          toReplace = c;
          replaceQty = it.quantity;
          replaceSupplier = it.supplier ?? null;
          break;
        }
      }
    }
    const replaceBanner = toReplace
      ? el('p', { class: 'replace-banner' },
          icon(Repeat),
          ` “${toReplace.description}” ya no puede usarse en ensamble. ` +
          'La lista muestra solo componentes utilizables, ordenados por cercanía de precio.')
      : null;

    /** Pone `c` en el slot. En modo reemplazo es un intercambio: conserva
     * la cantidad del componente a reemplazar (limitada a la existencia del
     * sustituto) y retira el viejo solo después de agregar el nuevo, así un
     * fallo al agregar (p. ej. sin existencia) deja el ensamble intacto. */
    async function choose(c: ComponentDto, supplier?: string) {
      if (!current) return;
      const asmId = current.id;
      const old = toReplace;
      const qty = old ? Math.max(1, Math.min(replaceQty, availableFor(c, supplier))) : 1;
      await mutate((async () => {
        await api.setAssemblyItem(asmId, slot.id, c.id, qty, supplier);
        if (old) await api.setAssemblyItem(asmId, slot.id, old.id, 0, replaceSupplier);
      })());
      closeModal();
    }

    function drawList() {
      const q = search.value.trim().toUpperCase();
      const rows = comps.filter(
        (c) =>
          (!q ||
            c.description.toUpperCase().includes(q) ||
            c.sku.toUpperCase().includes(q) ||
            c.brand.toUpperCase().includes(q)) &&
          (brandFilter === '' || c.brand === brandFilter) &&
          // Se ocultan los no seleccionables (sin existencia o SOLO-CD):
          // la lista solo ofrece componentes utilizables en ensamble.
          !blockedForAssembly(c),
      );
      // En modo reemplazo los candidatos se ordenan por cercanía de precio
      // al componente a reemplazar (en MXN para comparar entre monedas).
      if (toReplace) {
        const base = parseFloat(toReplace.price_mxp.amount);
        rows.sort(
          (a, b) =>
            Math.abs(parseFloat(a.price_mxp.amount) - base) -
            Math.abs(parseFloat(b.price_mxp.amount) - base),
        );
      }
      // Paginación: 20 renglones por página (antes la lista se cortaba en 60).
      const pages = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
      if (page > pages) page = pages;
      const pageRows = rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
      const children: HTMLElement[] = [];
      if (toReplace) children.push(replaceSourceRow(toReplace));
      children.push(...pageRows.map(rowFor));
      if (!rows.length) {
        children.push(el('p', { class: 'muted' }, 'Sin resultados.'));
      } else if (pages > 1) {
        children.push(pagerEl(rows.length, pages));
      }
      list.replaceChildren(...children);
    }

    /** Renglón del componente a reemplazar: solo informativo, no clicable. */
    function replaceSourceRow(c: ComponentDto): HTMLElement {
      const motivo = blockedForAssembly(c) === 'cd'
        ? 'sus ofertas quedaron SOLO-CD (no aptas para ensamble)'
        : 'se quedó sin existencia para ensamble';
      const row = el('div', { class: 'picker-row picker-replace-src' },
        el('div', { class: 'picker-desc' },
          el('div', { class: 'desc' }, c.description),
          el('div', { class: 'sub' },
            [c.sku, c.brand, kindLabel(c.kind), c.condition === 'usado' ? 'USADO' : null]
              .filter(Boolean)
              .join(' · ')),
          el('div', { class: 'replace-note' },
            icon(TriangleExclamationPoint, 12), ` Componente a reemplazar: ${motivo}.`),
        ),
        el('div', { class: 'picker-side' },
          el('div', { class: 'strong' }, mxn(c.price_mxp.amount)),
          el('span', { class: 'chip chip-danger' }, 'A REEMPLAZAR'),
        ),
      );
      row.addEventListener('click', (e) => e.stopPropagation());
      return row;
    }

    /** Controles de paginación (solo cuando hay más de una página). */
    function pagerEl(total: number, pages: number): HTMLElement {
      const prev = el('button', {
        class: 'icon-btn', title: 'Página anterior', disabled: page <= 1,
      }, icon(ChevronLeft));
      const next = el('button', {
        class: 'icon-btn', title: 'Página siguiente', disabled: page >= pages,
      }, icon(ChevronRight));
      prev.addEventListener('click', () => {
        if (page > 1) { page--; drawList(); list.scrollTop = 0; }
      });
      next.addEventListener('click', () => {
        if (page < pages) { page++; drawList(); list.scrollTop = 0; }
      });
      return el('div', { class: 'picker-pager' },
        prev,
        el('span', { class: 'muted' },
          `Página ${page} de ${pages} · ${total} resultado${total === 1 ? '' : 's'}`),
        next);
    }

    /** Renglón seleccionable de un componente del catálogo. */
    function rowFor(c: ComponentDto) {
      const item = existingQty(c.id);
      const supplierChip = el('span', { class: `chip chip-supplier supplier-${c.supplier || 'arroba'}` },
        c.supplier === 'cva' ? 'CVA' : c.supplier === 'arroba' ? '@' : (c.supplier || 'manual').toUpperCase());
      const others = (c.offers ?? []).filter((o) => o.supplier !== c.supplier);
      const chosenSupplier = item?.supplier ?? null;

      const offerLines = (c.offers ?? []).map((o) => {
        const oLabel = o.supplier === 'cva' ? 'CVA' : o.supplier === 'arroba' ? '@' : o.supplier.toUpperCase();
        const elegido = chosenSupplier ? chosenSupplier === o.supplier : c.supplier === o.supplier;
        const usable = !(o.cd_only && o.stock_ensamble === 0) && o.stock_ensamble > 0;
        const useBtn = usable
          ? el('button', { class: 'btn btn-sm btn-outline offer-use' },
              elegido ? icon(Check) : null, elegido ? 'Usando' : 'Usar este')
          : el('span', { class: 'chip chip-danger' }, o.stock_ensamble === 0 ? 'sin existencia' : 'solo CD');
        if (usable && current) {
          useBtn.addEventListener('click', async (e) => {
            e.stopPropagation();
            await choose(c, o.supplier);
          });
        }
        return el('div', { class: 'offer-line' },
          el('span', { class: `chip chip-supplier supplier-${o.supplier}` }, oLabel),
          el('span', { class: 'strong' }, mxn(o.price.amount)),
          el('span', { class: 'sub' }, o.stock_label),
          useBtn);
      });

      const row = el('div', { class: `picker-row${item ? ' picked' : ''}` },
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
      // Clic: agrega el componente (o lo intercambia en modo reemplazo);
      // si ya está en el ensamble, lo quita.
      row.addEventListener('click', async () => {
        if (!current) return;
        if (existingQty(c.id)) {
          await mutate(api.setAssemblyItem(current.id, slot.id, c.id, 0));
          closeModal();
        } else {
          await choose(c);
        }
      });
      return row;
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

    // Cualquier cambio de filtro regresa a la página 1.
    const resetPageAndDraw = () => {
      page = 1;
      drawList();
    };
    // Botón para limpiar la búsqueda: algunos entornos no disparan 'input'
    // al vaciar el campo, y el usuario necesita volver a la lista completa.
    const clearBtn = el('button', { class: 'icon-btn', title: 'Limpiar búsqueda' }, icon(X));
    clearBtn.addEventListener('click', () => {
      search.value = '';
      resetPageAndDraw();
      search.focus();
    });
    search.addEventListener('input', resetPageAndDraw);
    search.addEventListener('change', resetPageAndDraw);

    // Alta de productos desde el propio picker (p. ej. captura manual de
    // proveedores sin Excel como PCH). Al guardar se auto-selecciona.
    const addNewBtn = el('button', { class: 'btn btn-outline' },
      icon(Plus), `Agregar ${slot.label.toLowerCase()} nuevo`);
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
        replaceBanner,
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
            el('button', { class: 'icon-btn', title: 'Quitar una unidad' }, icon(Minus)),
            el('button', {
              class: 'icon-btn',
              title: item.quantity >= available ? `Sin más existencia (disponible ${available})` : 'Agregar una unidad',
              disabled: item.quantity >= available,
            }, icon(Plus)),
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
      const removeBtn = el('button', { class: 'icon-btn', title: 'Quitar' }, icon(X));
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

    const addBtn = el('button', { class: 'btn btn-sm btn-outline' }, icon(Plus), 'Agregar');
    addBtn.addEventListener('click', () => current && openPicker(slot));

    let clearBtn: HTMLElement | null = null;
    if (filled) {
      clearBtn = el('button', { class: 'icon-btn', title: 'Vaciar slot' }, icon(Trash));
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
                ? el('span', { class: 'chip chip-ok' }, icon(Check, 12), ' cubierto')
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
    }, icon(report.is_valid ? Check : X, 12), report.is_valid ? ' Equipo completo' : ' Incompleto');

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

  // Grupos del selector: tipo × vigencia. "Vigente" = no caducada (incluye
  // en fabricación, que no caduca), igual que el backend al emitir notas.
  const SELECTOR_GROUPS: { label: string; match: (a: AssemblySummary) => boolean }[] = [
    { label: 'Preensambles vigentes', match: (a) => a.quote_type === 'preensamble' && a.status !== 'caducada' },
    { label: 'Preensambles no vigentes', match: (a) => a.quote_type === 'preensamble' && a.status === 'caducada' },
    { label: 'Personalizados vigentes', match: (a) => a.quote_type === 'personalizado' && a.status !== 'caducada' },
    { label: 'Personalizados no vigentes', match: (a) => a.quote_type === 'personalizado' && a.status === 'caducada' },
  ];

  function applyFilters(list: AssemblySummary[]): AssemblySummary[] {
    const q = searchNumber.trim().replace('#', '');
    if (q === '') return list;
    return list.filter((a) => (a.quote_number ?? '').includes(q));
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
      el('strong', {}, icon(TriangleExclamationPoint), ' Advertencia'),
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
        icon(ArrowUpRight, 12), ` basado en el preensamble: ${current.based_on}`));
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

  // ------------------------------------------------------------------
  // Actualizar preensamble: renovación + refresh CVA/arroba/PCH
  // ------------------------------------------------------------------
  function openRefreshWizard() {
    if (!current) return;
    const map = byId();
    const buckets: { cva: string[]; arroba: string[]; pch: { desc: string; sku: string }[] } = {
      cva: [], arroba: [], pch: [],
    };
    for (const slot of SLOTS) {
      for (const item of current.slots[slot.id] ?? []) {
        const c = map.get(item.component_id);
        if (!c) continue;
        const supplier = item.supplier ?? c.supplier;
        const label = `${c.description} (${c.sku})`;
        if (supplier === 'cva') buckets.cva.push(label);
        else if (supplier === 'arroba') buckets.arroba.push(label);
        else buckets.pch.push({ desc: c.description, sku: c.sku });
      }
    }
    const summary = assemblies.find((a) => a.id === current!.id);
    const caducada = summary?.status === 'caducada';
    const id = current.id;

    const statusLine = el('p', { class: 'muted' }, 'Listo para actualizar.');
    const notice = el('p', { class: 'muted small' });
    const errorBox = el('p', { class: 'form-error' });
    const reportZone = el('div');

    const pchBlock: HTMLElement[] = [];
    let pchApproved = false;
    if (buckets.pch.length > 0) {
      const approval = el('input', { type: 'checkbox' }) as HTMLInputElement;
      approval.addEventListener('change', () => {
        pchApproved = approval.checked;
        runBtn.disabled = !pchApproved;
      });
      pchBlock.push(
        el('div', { class: 'pch-banner' },
          el('strong', {}, icon(TriangleExclamationPoint), ' Componentes de PCH/captura manual: '),
          el('p', { class: 'pch-line' }, 'No se pueden actualizar automáticamente. Verifica su existencia a mano y aprueba para continuar.'),
          ...buckets.pch.map((p) => el('div', { class: 'pch-line' }, `${p.desc} — SKU: ${p.sku}`)),
          el('label', { class: 'filter-check' },
            approval, ' Verifiqué la existencia de estos componentes'),
        ),
      );
    }

    const runBtn = el('button', {
      class: 'btn btn-primary',
      disabled: buckets.pch.length > 0,
    }, icon(RefreshCw), 'Actualizar ahora') as HTMLButtonElement;

    async function run() {
      runBtn.disabled = true;
      errorBox.textContent = '';
      const avisos: string[] = [];
      try {
        if (caducada) {
          statusLine.textContent = 'Renovando vigencia…';
          await api.renewAssembly(id);
        }
        if (buckets.arroba.length > 0) {
          statusLine.textContent = 'Pide el XLSX de arroba…';
          try {
            const isTauri = '__TAURI_INTERNALS__' in window;
            if (!isTauri) throw new Error('solo escritorio');
            const { open } = await import('@tauri-apps/plugin-dialog');
            const path = await open({
              multiple: false,
              filters: [{ name: 'Lista de precios arroba (.xlsx)', extensions: ['xlsx', 'xlsm', 'xls'] }],
            });
            if (!path || Array.isArray(path)) {
              avisos.push('arroba SIN actualizar (cancelaste el archivo).');
            } else {
              statusLine.textContent = 'Importando lista de arroba…';
              await api.importCatalogXlsx(String(path));
            }
          } catch (e) {
            avisos.push(`arroba SIN actualizar (${String(e)}).`);
          }
        }
        if (buckets.cva.length > 0) {
          statusLine.textContent = `Consultando el feed de CVA (${buckets.cva.length} SKU)…`;
        }
        const report = await api.refreshAssemblyFeed(id);
        await loadAssemblies(id);
        renderReport(report, avisos);
      } catch (e) {
        errorBox.textContent = String(e);
        runBtn.disabled = false;
        if (avisos.length) notice.textContent = avisos.join(' ');
      }
    }
    runBtn.addEventListener('click', run);

    function renderReport(report: AssemblyRefreshReport, avisos: string[]) {
      statusLine.textContent = '';
      const rows = report.items.map((r) =>
        el('tr', {},
          el('td', {}, r.sku),
          el('td', {},
            r.found
              ? el('span', { class: 'chip chip-ok' }, 'actualizado')
              : el('span', { class: 'chip chip-danger' }, 'no encontrado')),
          el('td', { class: 'sub' }, r.description.slice(0, 48)),
          el('td', { class: 'num strong' }, r.found ? `${r.price} ${r.currency}` : '—'),
          el('td', { class: 'num' }, r.found ? `Ens. ${r.stock_ensamble} · Venta ${r.stock_individual}` : '—'),
          el('td', { class: 'sub' }, r.promo_description ?? '')));
      reportZone.replaceChildren(
        el('h4', {}, 'Resultado'),
        el('table', { class: 'catalog-table refresh-table' },
          el('thead', {},
            el('tr', {},
              el('th', {}, 'SKU'), el('th', {}, 'Estado'), el('th', {}, 'Descripción'),
              el('th', { class: 'num' }, 'Precio'), el('th', { class: 'num' }, 'Existencias'),
              el('th', {}, 'Promo'))),
          el('tbody', {}, ...rows)),
        report.out_of_stock.length > 0
          ? el('p', { class: 'form-error' },
              `${report.out_of_stock.length} componente(s) quedaron sin existencia suficiente en ensambles vigentes.`)
          : el('p', { class: 'ok-note' }, 'Ningún ensamble vigente quedó sin existencias.'),
        avisos.length ? el('p', { class: 'muted small' }, avisos.join(' ')) : el('span'),
        el('div', { class: 'form-actions' },
          el('button', {
            class: 'btn btn-primary',
            onclick: async () => {
              closeModal();
              await loadAssemblies(id);
            },
          }, 'Listo')),
      );
      // El botón Listo queda al fondo: desplazar para verlo.
      reportZone.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }

    const body = el('div', { class: 'form' },
      el('p', { class: 'muted small' },
        caducada
          ? 'El preensamble está CADUCADO: se renovará la vigencia (nueva fecha de expiración) y deberás re-confirmarlo.'
          : 'El preensamble está vigente: solo se refrescan precios y existencias.'),
      el('div', { class: 'rate-rows' },
        el('div', { class: 'rate-row' },
          el('span', { class: 'muted' }, icon(Globe), ' CVA (feed XML: precios y existencias)'),
          el('span', { class: 'strong' }, `${buckets.cva.length}`)),
        el('div', { class: 'rate-row' },
          el('span', { class: 'muted' }, icon(ChartColumn), ' Arroba (se pedirá el XLSX)'),
          el('span', { class: 'strong' }, `${buckets.arroba.length}`)),
        el('div', { class: 'rate-row' },
          el('span', { class: 'muted' }, icon(PenLine), ' PCH / manual (aprobación manual)'),
          el('span', { class: 'strong' }, `${buckets.pch.length}`)),
      ),
      ...pchBlock,
      statusLine,
      notice,
      errorBox,
      el('div', { class: 'form-actions' }, runBtn),
      reportZone,
    );
    modal('Actualizar preensamble', body);
  }

  function renderHeader() {
    const filtered = applyFilters(assemblies);

    // Búsqueda por número de cotización
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
    const clearSearch = el('button', { class: 'icon-btn', title: 'Limpiar búsqueda' }, icon(X));
    clearSearch.addEventListener('click', () => {
      searchNumber = '';
      numberSearch.value = '';
      refreshSelector(filtered);
      numberSearch.focus();
    });

    const sel = el('select', { class: 'input' }) as HTMLSelectElement;
    // El grupo ya dice el tipo y la vigencia: la opción no los repite.
    const optionFor = (a: AssemblySummary) =>
      el('option', { value: a.id },
        `${a.quote_number ? `#${a.quote_number} · ` : ''}${a.name} ` +
        `[${STATUS_LABELS[a.status]}]` +
        (a.confirmed ? ' ✓' : a.is_valid ? '' : ` (${a.missing_count} pendientes)`) +
        ` · vence ${a.expiry_date}`);
    function fillSelector(list: AssemblySummary[]) {
      sel.replaceChildren();
      for (const g of SELECTOR_GROUPS) {
        const items = list.filter(g.match);
        if (items.length === 0) continue;
        sel.append(el('optgroup', { label: g.label }, ...items.map(optionFor)));
      }
      // Si la cotización en edición quedó fuera de la búsqueda, se muestra
      // aparte, para que las acciones no operen a escondidas.
      if (current && !list.some((a) => a.id === current!.id)) {
        const hidden = assemblies.find((a) => a.id === current!.id);
        if (hidden) {
          const tipo = hidden.quote_type === 'preensamble' ? 'Preensamble' : 'Personalizado';
          sel.append(el('optgroup', { label: `Fuera de la búsqueda · ${tipo}` }, optionFor(hidden)));
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
    const confirmBtn = el('button', { class: 'btn btn-primary' }, icon(Check), 'Confirmar');
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

    const newBtn = el('button', { class: 'btn btn-primary' }, icon(Plus), 'Nueva cotización');
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
      icon(Copy),
      current?.quote_type === 'preensamble'
        ? 'Generar personalizado desde este preensamble'
        : 'Duplicar como personalizado');
    dupBtn.addEventListener('click', async () => {
      if (!current) return;
      const copy = await api.duplicateAssembly(current.id);
      await loadAssemblies(copy.id);
    });

    // En fabricación: bloquea la caducidad y permite vender aunque pase la fecha
    const fabBtn = el('button', { class: 'btn btn-outline' },
      icon(current?.in_fabrication ? Play : Cog),
      current?.in_fabrication ? 'Quitar fabricación' : 'En fabricación');
    fabBtn.addEventListener('click', async () => {
      if (!current) return;
      await api.setFabrication(current.id, !current.in_fabrication);
      await loadAssemblies(current.id);
    });

    const refreshBtn = current?.quote_type === 'preensamble'
      ? el('button', { class: 'btn btn-outline', title: 'Renueva la vigencia si está caducado y actualiza precios/existencias (CVA vía feed XML, arroba con el XLSX, PCH con tu aprobación)' }, icon(RefreshCw), 'Actualizar')
      : null;
    if (refreshBtn) refreshBtn.addEventListener('click', openRefreshWizard);

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
        numberSearch,
        clearSearch,
      ),
      el('div', { class: 'toolbar' },
        sel, confirmBtn, fabBtn, dupBtn, refreshBtn, newBtn, renameBtn, delBtn,
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
          'Aún no hay cotizaciones. Crea una con «Nueva cotización».'),
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
    await loadAssemblies(deps.initialAssemblyId ?? undefined);
  }
  boot();

  return view;
}
