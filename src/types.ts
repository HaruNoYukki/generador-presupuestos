// Tipos que espejan los DTOs de Rust (serde, snake_case).

export type Kind =
  | 'cpu'
  | 'motherboard'
  | 'ram'
  | 'storage'
  | 'psu'
  | 'case'
  | 'cooler_air'
  | 'cooler_liquid'
  | 'gpu'
  | 'monitor'
  | 'keyboard'
  | 'mouse'
  | 'keyboard_mouse_kit'
  | 'expansion_card'
  | 'other';

export type SlotId =
  | 'cpu'
  | 'motherboard'
  | 'ram'
  | 'storage'
  | 'psu'
  | 'case'
  | 'cooling'
  | 'gpu'
  | 'monitor'
  | 'keyboard'
  | 'mouse'
  | 'expansion_card';

export interface Money {
  amount: string;
  currency: 'USD' | 'MXP';
}

export interface Stock {
  a_cen: number;
  ave_2: number;
  cedis: number;
  cmal: number;
  gdl: number;
}

export interface Specs {
  socket?: string | null;
  includes_cooler?: boolean | null;
  form_factor?: string | null;
  ddr?: string | null;
  capacity_gb?: number | null;
  ram_form?: string | null;
  wattage?: number | null;
  includes_psu?: boolean | null;
  psu_watts?: number | null;
  storage_kind?: 'ssd' | 'hdd' | null;
  internal?: boolean | null;
  capacity_raw?: string | null;
  vram?: string | null;
  inches?: string | null;
  card_type?: string | null;
}

/** Oferta de un SKU por proveedor. */
export interface Offer {
  supplier: string;
  price: Money;
  on_promotion: boolean;
  /** Existencias por almacén del proveedor. */
  stocks: Record<string, number>;
  /** Desglose legible construido por la base. */
  stock_label: string;
  stock_ensamble: number;
  stock_individual: number;
  cd_only: boolean;
  /** Precio corregido a mano (protegido del import). */
  price_overridden: boolean;
  /** Existencia corregida a mano (misma protección). */
  stock_overridden: boolean;
  /** Precio/existencia de la lista, presentes solo con override activo. */
  list_price: Money | null;
  list_stock: number | null;
}

/** Correcciones manuales protegidas de la importación de listas. */
export interface Overrides {
  kind?: Kind | null;
  /** Claves de `Specs` corregidas a mano (valor tal como se serializa). */
  specs?: Partial<Record<keyof Specs, unknown>>;
}

export interface Component {
  id: string;
  sku: string;
  barcode: string | null;
  description: string;
  brand: string;
  department: string;
  kind: Kind;
  condition: 'nuevo' | 'usado';
  /** Precio de la MEJOR oferta (menor precio disponible). */
  price: Money;
  on_promotion: boolean;
  stock: Stock;
  source: 'lista' | 'manual';
  specs: Specs;
  /** Proveedor de la mejor oferta: 'arroba' | 'cva' | 'manual'. */
  supplier: string;
  /** Desglose por almacén de la oferta ganadora. */
  offer_stocks: Record<string, number>;
  /** Existencia total elegible para ensambles. */
  stock_ensamble_total: number;
  /** Existencia total para venta individual. */
  stock_individual_total: number;
  /** La mejor oferta es solo-CD: no usable en ensambles. */
  cd_only_best: boolean;
  /** Desglose de inventario por proveedor. */
  offers: Offer[];
  /** Correcciones manuales (ya aplicadas en `kind` y `specs`). */
  overrides: Overrides;
  /** Tipo y specs tal como los trajo la lista, sin correcciones. */
  list_kind: Kind;
  list_specs: Specs;
}

export interface ComponentDto extends Component {
  price_mxp: Money;
}

/** Componente de un ensamble vigente afectado por una importación. */
export interface AffectedAssembly {
  id: string;
  name: string;
  quote_type: QuoteType;
  quote_number: string | null;
  quantity: number;
}

export interface OutOfStockHit {
  component_id: string;
  sku: string;
  description: string;
  kind: Kind;
  supplier: string | null;
  stock_after: number;
  /** Antes de importar sí alcanzaba: se acaba de agotar. */
  newly_out: boolean;
  assemblies: AffectedAssembly[];
}

export interface XlsxImportSummary {
  updated: number;
  inserted: number;
  /** SKUs del proveedor que ya no venían y quedaron agotados. */
  zeroed: number;
  total: number;
  supplier: string;
  out_of_stock: OutOfStockHit[];
}

/** Configuración del feed XML de CVA: número de cliente (la auth es por
 *  IP, dada de alta en el portal de CVA). */
export interface CvaFeedConfig {
  cliente: string;
  configured?: boolean;
}

/** Resumen de importación del feed CVA (mismo informe que el XLSX más el
 *  conteo de artículos descargados por grupo). */
export interface CvaImportSummary {
  updated: number;
  inserted: number;
  zeroed: number;
  /** Artículos omitidos por datos inválidos (p. ej. "Sin Descuento"). */
  skipped: number;
  total: number;
  groups: [string, number][];
  out_of_stock: OutOfStockHit[];
}

export interface ExchangeRate {
  pesos_per_dollar: string;
}

export interface CatalogOverview {
  source_file: string;
  list_date: string;
  exchange_rate: ExchangeRate;
  effective_rate: ExchangeRate;
  exchange_rule: string;
  rate_is_overridden: boolean;
  total_components: number;
  counts_by_kind: Record<string, number>;
}

export interface RateInfo {
  rate: ExchangeRate;
  list_rate: ExchangeRate;
  effective_rate: ExchangeRate;
  exchange_rule: string;
  source_file: string;
  list_date: string;
  is_overridden: boolean;
  zai_key: string;
}

export interface AssemblyItem {
  component_id: string;
  quantity: number;
  /** Proveedor elegido para esta partida (null = mejor precio). */
  supplier?: string | null;
}

export type QuoteType = 'preensamble' | 'personalizado';
export type QuoteStatus = 'vigente' | 'en_fabricacion' | 'caducada';

export interface Assembly {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  confirmed_at: string | null;
  quote_type: QuoteType;
  in_fabrication: boolean;
  quote_number: string | null;
  /** Origen de un personalizado creado desde un preensamble. */
  based_on: string | null;
  slots: Partial<Record<SlotId, AssemblyItem[]>>;
}

export interface AssemblySummary {
  id: string;
  name: string;
  updated_at: string;
  is_valid: boolean;
  confirmed: boolean;
  missing_count: number;
  item_count: number;
  quote_type: QuoteType;
  status: QuoteStatus;
  expiry_date: string;
  quote_number: string | null;
  based_on: string | null;
}

export type Severity = 'error' | 'warning' | 'info';

export interface ValidationIssue {
  severity: Severity;
  code: string;
  message: string;
}

export interface PsuCoverage {
  kind: 'dedicated' | 'included_in_case' | 'missing';
  detail?: string;
}

export interface CoolingCoverage {
  kind: 'included_in_cpu' | 'provided' | 'missing';
  detail?: string;
}

export interface ValidationReport {
  is_valid: boolean;
  issues: ValidationIssue[];
  psu: PsuCoverage;
  cooling: CoolingCoverage;
}

/** Desglose de costo del ensamble (fase 2), todo en MXN. */
export interface CostBreakdown {
  subtotal: Money;
  iva: Money;
  total: Money;
}

/** Renglón de la tabla de ganancias (fase 3), montos en MXN. */
export interface ProfitLine {
  component_id: string;
  description: string;
  sku: string;
  quantity: number;
  unit_cost: string;
  cost: string;
  base_profit: string;
  adjustment: string;
  final_profit: string;
  iva: string;
  price: string;
}

export interface ProfitTotals {
  cost: string;
  base_profit: string;
  adjustment: string;
  final_profit: string;
  iva: string;
  price: string;
}

export interface ProfitReport {
  percent: string;
  target: string | null;
  difference: string | null;
  adjustment_per_line: string | null;
  lines: ProfitLine[];
  totals: ProfitTotals;
}

export interface SaleNoteLine {
  component_id: string;
  sku: string;
  description: string;
  kind: string;
  serial: string;
  price: string;
}

export interface SaleNoteRecord {
  number: string;
  quote_number: string | null;
  /** NULL en ventas individuales. */
  assembly_id: string | null;
  created_at: string;
  lines: SaleNoteLine[];
}

export interface CartItem {
  component_id: string;
  quantity: number;
}

export interface CatalogFilters {
  brands: string[];
  departments: string[];
}
