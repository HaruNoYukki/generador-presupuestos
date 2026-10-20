// Capa fina de invocación de comandos Tauri.
// En navegador (vite dev sin Tauri) se enruta al mock con las mismas reglas.
import { invoke } from '@tauri-apps/api/core';
import type {
  Assembly,
  CartItem,
  AssemblySummary,
  CatalogOverview,
  ComponentDto,
  CostBreakdown,
  ProfitReport,
  RateInfo,
  ValidationReport,
} from './types';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) return invoke<T>(cmd, args);
  const { mockInvoke } = await import('./mock');
  return mockInvoke(cmd, args) as Promise<T>;
}

export interface NewManualComponent {
  description: string;
  sku?: string | null;
  brand?: string | null;
  kind: string;
  price_amount: string;
  price_currency: 'USD' | 'MXP';
  condition: 'nuevo' | 'usado';
  specs: Record<string, unknown>;
}

export const api = {
  catalogOverview: () => call<CatalogOverview>('catalog_overview'),

  catalogFilters: () =>
    call<{ brands: string[]; departments: string[] }>('catalog_filters'),

  getZaiKey: () => call<string>('get_zai_key'),

  setZaiKey: (key: string) => call<void>('set_zai_key', { key }),

  aiNormalizeTitles: (limit: number) =>
    call<{ improved: number; remaining: number }>('ai_normalize_titles', { limit }),

  listComponents: () =>
    call<ComponentDto[]>('list_components', { kind: null, search: null }),

  addManualComponent: (input: NewManualComponent) =>
    call<ComponentDto>('add_manual_component', { input }),

  removeManualComponent: (id: string) =>
    call<boolean>('remove_manual_component', { id }),

  getRate: () => call<RateInfo>('get_rate'),

  setRate: (rate: string) => call<RateInfo>('set_rate', { rate }),

  createAssembly: (name: string, quoteType?: string) =>
    call<Assembly>('create_assembly', { name, quoteType: quoteType ?? null }),

  duplicateAssembly: (id: string) => call<Assembly>('duplicate_assembly', { id }),

  setFabrication: (id: string, inFabrication: boolean) =>
    call<Assembly>('set_fabrication', { id, inFabrication }),

  assignQuoteNumber: (id: string) => call<string>('assign_quote_number', { id }),

  listAssemblies: () => call<AssemblySummary[]>('list_assemblies'),

  getAssembly: (id: string) => call<Assembly>('get_assembly', { id }),

  renameAssembly: (id: string, name: string) =>
    call<Assembly>('rename_assembly', { id, name }),

  deleteAssembly: (id: string) => call<boolean>('delete_assembly', { id }),

  setAssemblyItem: (
    assemblyId: string,
    slot: string,
    componentId: string,
    quantity: number,
    supplier?: string | null,
  ) =>
    call<Assembly>('set_assembly_item', {
      assemblyId,
      slot,
      componentId,
      quantity,
      supplier: supplier ?? null,
    }),

  clearAssemblySlot: (assemblyId: string, slot: string) =>
    call<Assembly>('clear_assembly_slot', { assemblyId, slot }),

  validateAssembly: (assemblyId: string) =>
    call<ValidationReport>('validate_assembly', { assemblyId }),

  assemblyCost: (assemblyId: string) =>
    call<CostBreakdown>('assembly_cost', { assemblyId }),

  confirmAssembly: (assemblyId: string) =>
    call<Assembly>('confirm_assembly', { assemblyId }),

  assemblyProfit: (assemblyId: string, percent: string, target?: string | null) =>
    call<ProfitReport>('assembly_profit', {
      assemblyId,
      percent,
      target: target ?? null,
    }),

  nextQuoteNumber: () => call<string>('next_quote_number'),

  /** Importa un catálogo JSON con UPSERT por SKU (solo app de escritorio). */
  importCatalog: (path: string) =>
    call<{ updated: number; inserted: number; total: number }>('import_catalog', { path }),

  /** Importa un Excel de lista de precios (cualquier proveedor registrado). */
  importCatalogXlsx: (path: string) =>
    call<{ updated: number; inserted: number; total: number; supplier: string }>(
      'import_catalog_xlsx',
      { path },
    ),

  getComponentBySku: (sku: string) =>
    call<ComponentDto | null>('get_component_by_sku', { sku }),

  nextSaleNoteNumber: () => call<string>('next_sale_note_number'),

  cartProfit: (items: CartItem[], percent: string, target?: string | null) =>
    call<ProfitReport>('cart_profit', {
      items,
      percent,
      target: target ?? null,
    }),

  recordSaleNote: (note: {
    number: string;
    quote_number: string | null;
    assembly_id: string | null;
    lines: {
      component_id: string;
      sku: string;
      description: string;
      kind: string;
      serial: string;
      price: string;
    }[];
  }) => call<unknown>('record_sale_note', { ...note }),

  listSaleNotes: () => call<unknown[]>('list_sale_notes'),

  saveExport: (path: string, bytesBase64: string) =>
    call<string>('save_export', { path, bytesBase64 }),
};
