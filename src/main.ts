// Punto de entrada de la interfaz gráfica.
import { api } from './api';
import { Assembly } from './types';
import { createAssemblyView } from './views/assembly';
import { createCatalogView } from './views/catalog';
import { createProfitView } from './views/profit';
import { createQuoteView } from './views/quote';
import { createSaleView } from './views/sale';
import { createSaleDirectView } from './views/sale_direct';
import { createSettingsView } from './views/settings';
import { el } from './ui';
import type { CartItem, CatalogOverview, ComponentDto } from './types';

type Tab = 'ensamble' | 'ganancias' | 'cotizacion' | 'venta' | 'directa' | 'catalogo' | 'ajustes';
type TabDef = { id: Tab; label: string };

// Flujo de cotización: pasos en orden, recorridos con la barra de pasos.
const FLOW: TabDef[] = [
  { id: 'ensamble', label: 'Ensamble' },
  { id: 'ganancias', label: 'Ganancias' },
  { id: 'cotizacion', label: 'Cotización' },
  { id: 'venta', label: 'Nota de venta' },
];

// Secciones de la barra lateral. Los dos flujos de venta van separados:
// ensambles (cotización → nota de venta) y venta individual (carrito → nota).
const SECTIONS: { title: string; tabs: TabDef[]; flow?: boolean }[] = [
  { title: 'Flujo de cotización', tabs: FLOW, flow: true },
  { title: 'Ventas', tabs: [{ id: 'directa', label: 'Venta individual' }] },
  {
    title: 'Administración',
    tabs: [
      { id: 'catalogo', label: 'Catálogo' },
      { id: 'ajustes', label: 'Ajustes' },
    ],
  },
];

const state = {
  overview: null as CatalogOverview | null,
  components: [] as ComponentDto[],
  selectedAssembly: null as Assembly | null,
  // Parámetros compartidos entre Ganancias y Cotización.
  quoteParams: { percent: '20', target: '' },
  // Carrito de la venta individual (sobrevive cambios de sección).
  cart: [] as CartItem[],
  tab: 'ensamble' as Tab,
};

const root = document.getElementById('app')!;
// Cotización en curso dentro de la barra de pasos; se repinta en su lugar
// cuando cambia la selección en Ensamble, sin recrear la vista.
let stepContext: HTMLElement | null = null;

function renderShell() {
  root.replaceChildren(
    renderSidebar(),
    el('div', { class: 'app-body' }, renderStepBar(), renderContent()),
  );
}

/** Barra lateral: marca, secciones (flujo de cotización numerado, ventas y
 * administración) y, al pie, el tipo de cambio vigente. */
function renderSidebar(): HTMLElement {
  const overview = state.overview;
  return el('aside', { class: 'sidebar' },
    el('h1', { class: 'side-brand' }, '⚙ Generador de Presupuestos'),
    el('nav', { class: 'side-nav' },
      ...SECTIONS.map((section) =>
        el('div', { class: 'side-section' },
          el('div', { class: 'side-title' }, section.title),
          ...section.tabs.map((t, i) =>
            el('button', {
              class: `side-item${state.tab === t.id ? ' active' : ''}`,
              onclick: () => setTab(t.id),
            },
              section.flow ? el('span', { class: 'step-num' }, String(i + 1)) : null,
              el('span', {}, t.label),
            ),
          ),
        ),
      ),
    ),
    overview
      ? el('div', { class: 'rate-chip side-rate', title: overview.exchange_rule },
          `T.C ${overview.exchange_rate.pesos_per_dollar} → efectivo ${overview.effective_rate.pesos_per_dollar}`,
          el('span', { class: 'side-rate-date' }, `lista ${overview.list_date}`),
        )
      : null,
  );
}

/** Barra de pasos del flujo: ‹ paso anterior · Paso N de 4 · siguiente ›.
 * Solo en los pasos del flujo; cada vista conserva su propio aviso cuando
 * falta confirmar la cotización. */
function renderStepBar(): HTMLElement | null {
  const i = FLOW.findIndex((s) => s.id === state.tab);
  if (i < 0) {
    stepContext = null;
    return null;
  }
  const prev = FLOW[i - 1];
  const next = FLOW[i + 1];
  stepContext = el('span', { class: 'step-ctx' });
  paintStepContext();
  return el('div', { class: 'step-bar' },
    el('button', {
      class: 'btn btn-outline btn-sm',
      disabled: !prev,
      onclick: () => {
        if (prev) setTab(prev.id);
      },
    }, prev ? `‹ ${prev.label}` : '‹ Anterior'),
    el('div', { class: 'step-info' },
      el('span', { class: 'step-count' }, `Paso ${i + 1} de ${FLOW.length}`),
      el('strong', {}, FLOW[i].label),
      stepContext,
    ),
    el('button', {
      class: 'btn btn-primary btn-sm',
      disabled: !next,
      onclick: () => {
        if (next) setTab(next.id);
      },
    }, next ? `${next.label} ›` : 'Siguiente ›'),
  );
}

function paintStepContext() {
  if (!stepContext) return;
  const asm = state.selectedAssembly;
  stepContext.textContent = asm
    ? `${asm.quote_number ? `#${asm.quote_number} · ` : ''}${asm.name}` +
      (asm.confirmed_at ? '' : ' · sin confirmar')
    : 'Sin cotización seleccionada';
}

function renderContent(): HTMLElement {
  const content = el('main', { class: 'app-main' });
  if (state.tab === 'ensamble') {
    content.append(
      createAssemblyView({
        components: state.components,
        refreshComponents,
        initialAssemblyId: state.selectedAssembly?.id,
        onSelected: (asm) => {
          state.selectedAssembly = asm;
          paintStepContext();
        },
        onConfirmed: () => setTab('ganancias'),
      }),
    );
  } else if (state.tab === 'ganancias') {
    content.append(
      createProfitView({
        getSelectedAssembly: () => state.selectedAssembly,
        getParams: () => state.quoteParams,
        setParams: (p) => {
          state.quoteParams = p;
        },
      }),
    );
  } else if (state.tab === 'cotizacion') {
    content.append(
      createQuoteView({
        getSelectedAssembly: () => state.selectedAssembly,
        getParams: () => state.quoteParams,
        // Al asignar folio se vuelve a leer la cotización: así la hoja, la
        // barra de pasos y la nota de venta usan el folio nuevo al instante.
        onChanged: async () => {
          const id = state.selectedAssembly?.id;
          if (!id) return;
          state.selectedAssembly = await api.getAssembly(id);
          paintStepContext();
        },
      }),
    );
  } else if (state.tab === 'venta') {
    content.append(
      createSaleView({
        getSelectedAssembly: () => state.selectedAssembly,
        getParams: () => state.quoteParams,
      }),
    );
  } else if (state.tab === 'directa') {
    content.append(
      createSaleDirectView({
        components: () => state.components,
        cart: () => state.cart,
        setCart: (items) => {
          state.cart = items;
        },
        getParams: () => state.quoteParams,
        setParams: (p) => {
          state.quoteParams = p;
        },
      }),
    );
  } else if (state.tab === 'catalogo') {
    content.append(createCatalogView(state.components, refreshComponents));
  } else {
    // Tras importar una lista cambian precios y existencias: se recarga la
    // lista compartida (in-place) para que ensambles y picker la vean.
    content.append(createSettingsView(async () => {
      await refreshComponents();
      await refreshOverview();
    }));
  }
  return content;
}

async function refreshComponents(): Promise<ComponentDto[]> {
  // Mutación in-place: las vistas conservan la referencia a la lista.
  // Incluye los tipo «otro»: la venta individual vende cualquier cosa y el
  // picker de ensambles los descarta por kinds del slot.
  const fresh = await api.listComponents(true);
  state.components.length = 0;
  state.components.push(...fresh);
  return state.components;
}

async function refreshOverview() {
  state.overview = await api.catalogOverview();
  renderShell();
}

function setTab(tab: Tab) {
  state.tab = tab;
  renderShell();
}

async function boot() {
  state.overview = await api.catalogOverview();
  state.components = await api.listComponents(true);
  renderShell();
}

boot();
