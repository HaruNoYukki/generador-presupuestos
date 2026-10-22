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

const state = {
  overview: null as CatalogOverview | null,
  components: [] as ComponentDto[],
  selectedAssembly: null as Assembly | null,
  // Parámetros compartidos entre Ganancias y Cotización.
  quoteParams: { percent: '20', target: '' },
  // Carrito de la venta individual (sobrevive cambios de pestaña).
  cart: [] as CartItem[],
  tab: 'ensamble' as Tab,
};

const root = document.getElementById('app')!;

function renderShell() {
  const overview = state.overview;
  // Dos flujos: ensambles (cotización→venta) y ventas individuales.
  const tabs: { id: Tab; label: string; group: 'ensambles' | 'individuales' | 'app' }[] = [
    { id: 'ensamble', label: 'Ensamble', group: 'ensambles' },
    { id: 'ganancias', label: 'Ganancias', group: 'ensambles' },
    { id: 'cotizacion', label: 'Cotización', group: 'ensambles' },
    { id: 'venta', label: 'Nota de venta', group: 'ensambles' },
    { id: 'directa', label: 'Venta individual', group: 'individuales' },
    { id: 'catalogo', label: 'Catálogo', group: 'app' },
    { id: 'ajustes', label: 'Ajustes', group: 'app' },
  ];

  const header = el('header', { class: 'app-header' },
    el('div', { class: 'brand' },
      el('h1', {}, '⚙ Generador de Presupuestos'),
      overview
        ? el('span', { class: 'rate-chip', title: overview.exchange_rule },
            `T.C ${overview.exchange_rate.pesos_per_dollar} → efectivo ${overview.effective_rate.pesos_per_dollar} · lista ${overview.list_date}`)
        : null,
    ),
    el('nav', { class: 'tabs' },
      ...tabs.flatMap((t) => {
        const btn = el('button', {
          class: `tab ${state.tab === t.id ? 'active' : ''}`,
          onclick: () => setTab(t.id),
        }, t.label);
        const prev = tabs[tabs.indexOf(t) - 1];
        if (prev && prev.group !== t.group) {
          return [el('span', { class: 'tab-sep' }), btn];
        }
        return [btn];
      }),
    ),
  );

  const content = el('main', { class: 'app-main' });
  if (state.tab === 'ensamble') {
    content.append(
      createAssemblyView({
        components: state.components,
        refreshComponents,
        onSelected: (asm) => {
          state.selectedAssembly = asm;
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

  root.replaceChildren(header, content);
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
