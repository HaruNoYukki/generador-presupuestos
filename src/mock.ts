// Mock del backend para desarrollo en navegador (sin Tauri).
// Reproduce las reglas del dominio en TypeScript usando la MISMA semilla
// data/catalog_seed.json. En la app real (Tauri) este archivo nunca se usa:
// api.ts enruta a los comandos Rust cuando window.__TAURI_INTERNALS__ existe.
import type {
  Assembly,
  CartItem,
  AssemblySummary,
  CatalogOverview,
  ComponentDto,
  CostBreakdown,
  ProfitLine,
  ProfitReport,
  ProfitTotals,
  RateInfo,
  ValidationIssue,
  ValidationReport,
} from './types';

interface SeedComponent {
  id: string;
  sku: string;
  barcode: string | null;
  description: string;
  brand: string;
  department: string;
  kind: string;
  condition: 'nuevo' | 'usado';
  price: { amount: string; currency: 'USD' | 'MXP' };
  on_promotion: boolean;
  stock: Record<string, number>;
  source: 'lista' | 'manual';
  supplier?: string;
  specs: Record<string, unknown>;
}

// Semilla opcional: en el navegador se intenta leer /data/catalog_seed.json
// (el archivo NO va en el repo; cada instalación genera el suyo con el
// script de Python o importa un Excel). Sin semilla, catálogo vacío.
interface SeedFile {
  source_file: string;
  list_date: string;
  exchange_rate: string;
  exchange_rule: string;
  components: SeedComponent[];
}

let seedData: SeedFile | null = null;

async function ensureSeed(): Promise<SeedFile> {
  if (seedData) return seedData;
  try {
    const r = await fetch('/data/catalog_seed.json');
    if (!r.ok) throw new Error(String(r.status));
    seedData = (await r.json()) as SeedFile;
  } catch {
    seedData = {
      source_file: 'sin lista importada',
      list_date: '',
      exchange_rate: '19',
      exchange_rule: 'Importa un Excel de proveedor en Ajustes → Catálogo.',
      components: [],
    };
  }
  state.rate = seedData.exchange_rate;
  state.components = [...seedFile().components].sort(
    (a, b) =>
      KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) ||
      a.description.localeCompare(b.description),
  );
  return seedData;
}

function seedFile(): SeedFile {
  return seedData as SeedFile;
}

const KIND_ORDER = [
  'cpu', 'motherboard', 'ram', 'storage', 'psu', 'case',
  'cooler_air', 'cooler_liquid', 'gpu', 'monitor',
  'keyboard', 'mouse', 'keyboard_mouse_kit', 'expansion_card', 'other',
];

function ceilRate(rate: string): number {
  return Math.ceil(parseFloat(rate));
}

function toMxp(price: { amount: string; currency: string }, eff: number): string {
  if (price.currency === 'MXP') return price.amount;
  return (parseFloat(price.amount) * eff).toFixed(2);
}

interface MockState {
  components: SeedComponent[];
  rate: string;
  rateOverridden: boolean;
  assemblies: Map<string, Assembly>;
  quoteCounters: Map<string, number>;
  saleNoteCounters: Map<string, number>;
  saleNotes: unknown[];
}

const state: MockState = {
  saleNoteCounters: new Map(),
  saleNotes: [],
  quoteCounters: new Map(),
  components: [], // se llena en ensureSeed() con la semilla opcional
  rate: '19',
  rateOverridden: false,
  assemblies: new Map(),
};

function asDto(c: SeedComponent): ComponentDto {
  const stock = {
    a_cen: c.stock?.a_cen ?? 0,
    ave_2: c.stock?.ave_2 ?? 0,
    cedis: c.stock?.cedis ?? 0,
    cmal: c.stock?.cmal ?? 0,
    gdl: c.stock?.gdl ?? 0,
  };
  const sellable = stock.a_cen + stock.ave_2 + stock.cedis;
  return {
    ...c,
    specs: c.specs ?? {},
    stock,
    supplier: c.supplier ?? 'arroba',
    offer_stocks: stock,
    stock_ensamble_total: sellable,
    stock_individual_total: sellable,
    cd_only_best: false,
    offers: [
      {
        supplier: c.supplier ?? 'arroba',
        price: c.price,
        on_promotion: c.on_promotion,
        stocks: stock,
        stock_label: `CEN ${stock.a_cen} · AVE-2 ${stock.ave_2} · CEDIS ${stock.cedis}`,
        stock_ensamble: sellable,
        stock_individual: sellable,
        cd_only: false,
      },
    ],
    price_mxp: { amount: toMxp(c.price, ceilRate(state.rate)), currency: 'MXP' },
  } as ComponentDto;
}

const localDate = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;

// Reglas de caducidad (idénticas a domain/assembly.rs), en fecha local.
function expiryDate(quoteType: string, created: Date): string {
  const d = new Date(created);
  if (quoteType === 'personalizado') {
    d.setDate(d.getDate() + 15);
    return localDate(d);
  }
  const firstNext = new Date(d.getFullYear(), d.getMonth() + 1, 1);
  const isLastDay = (firstNext.getTime() - d.getTime()) === 86400000;
  if (isLastDay) d.setDate(d.getDate() + 15);
  else d.setTime(firstNext.getTime());
  return localDate(d);
}

function statusOf(a: Assembly, today = new Date()): string {
  if (a.in_fabrication) return 'en_fabricacion';
  const expiry = expiryDate(a.quote_type ?? 'personalizado', new Date(a.created_at));
  return localDate(today) >= expiry ? 'caducada' : 'vigente';
}

function newAssembly(name: string, quoteType = 'personalizado'): Assembly {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    name,
    created_at: now,
    updated_at: now,
    confirmed_at: null,
    quote_type: quoteType as Assembly['quote_type'],
    in_fabrication: false,
    quote_number: null,
    based_on: null,
    slots: {},
  } as Assembly;
}

function summarize(a: Assembly): AssemblySummary {
  const r = validate(a);
  return {
    id: a.id,
    name: a.name,
    updated_at: a.updated_at,
    is_valid: r.is_valid,
    confirmed: !!a.confirmed_at,
    quote_type: (a.quote_type ?? 'personalizado') as AssemblySummary['quote_type'],
    status: statusOf(a) as AssemblySummary['status'],
    expiry_date: expiryDate(a.quote_type ?? 'personalizado', new Date(a.created_at)),
    quote_number: a.quote_number ?? null,
    based_on: a.based_on ?? null,
    missing_count: r.issues.filter((i) => i.severity === 'error').length,
    item_count: Object.values(a.slots).reduce((n, v) => n + (v?.length ?? 0), 0),
  };
}

const SLOT_KINDS: Record<string, string[]> = {
  cpu: ['cpu'],
  motherboard: ['motherboard'],
  ram: ['ram'],
  storage: ['storage'],
  psu: ['psu'],
  case: ['case'],
  cooling: ['cooler_air', 'cooler_liquid'],
  gpu: ['gpu'],
  monitor: ['monitor'],
  keyboard: ['keyboard', 'keyboard_mouse_kit'],
  mouse: ['mouse', 'keyboard_mouse_kit'],
  expansion_card: ['expansion_card'],
};

const MINIMUMS: [string, string][] = [
  ['cpu', 'Falta el procesador.'],
  ['motherboard', 'Falta la tarjeta madre.'],
  ['ram', 'Falta la memoria RAM.'],
  ['storage', 'Falta el dispositivo de almacenamiento (SSD o HDD).'],
  ['case', 'Falta el gabinete.'],
];

// Ganancia del carrito de venta individual (mismas reglas que ensambles).
function computeCartProfit(items: CartItem[], percent: number): ProfitReport {
  const eff = ceilRate(state.rate);
  const factor = percent / 100;
  const lines: ProfitLine[] = [];
  for (const item of items) {
    const c = state.components.find((x) => x.id === item.component_id);
    if (!c) continue;
    const unit = c.price.currency === 'USD' ? parseFloat(c.price.amount) * eff : parseFloat(c.price.amount);
    const cost = r2(unit * item.quantity);
    const base_profit = r2(cost * factor);
    const iva = r2((cost + base_profit) * 0.16);
    lines.push({
      component_id: c.id,
      description: c.description,
      sku: c.sku,
      quantity: item.quantity,
      unit_cost: unit.toFixed(2),
      cost: cost.toFixed(2),
      base_profit: base_profit.toFixed(2),
      adjustment: '0.00',
      final_profit: base_profit.toFixed(2),
      iva: iva.toFixed(2),
      price: r2(cost + base_profit + iva).toFixed(2),
    });
  }
  const report: ProfitReport = {
    percent: String(percent),
    target: null,
    difference: null,
    adjustment_per_line: null,
    lines,
    totals: emptyTotals(),
  };
  recomputeTotals(report);
  return report;
}

// Fase 3: ganancias — ganancia = costo × %, IVA UNA vez sobre (costo+ganancia
// base) × 16%; precio objetivo reparte la diferencia en partes iguales.
const r2 = (n: number) => Math.round(n * 100) / 100;

function computeProfit(a: Assembly, percent: number, target?: string | null): ProfitReport {
  const eff = ceilRate(state.rate);
  const factor = percent / 100;
  const lines: ProfitLine[] = [];
  for (const items of Object.values(a.slots)) {
    for (const item of items ?? []) {
      const c = state.components.find((x) => x.id === item.component_id);
      if (!c) continue;
      const unit = c.price.currency === 'USD' ? parseFloat(c.price.amount) * eff : parseFloat(c.price.amount);
      const cost = r2(unit * item.quantity);
      const base_profit = r2(cost * factor);
      const iva = r2((cost + base_profit) * 0.16);
      lines.push({
        component_id: c.id,
        description: c.description,
        sku: c.sku,
        quantity: item.quantity,
        unit_cost: unit.toFixed(2),
        cost: cost.toFixed(2),
        base_profit: base_profit.toFixed(2),
        adjustment: '0.00',
        final_profit: base_profit.toFixed(2),
        iva: iva.toFixed(2),
        price: r2(cost + base_profit + iva).toFixed(2),
      });
    }
  }
  const report: ProfitReport = {
    percent: String(percent),
    target: null,
    difference: null,
    adjustment_per_line: null,
    lines,
    totals: emptyTotals(),
  };
  recomputeTotals(report);
  if (target != null && target !== '' && lines.length > 0) {
    const t = parseFloat(target);
    const current = report.totals.price;
    const difference = r2(t - parseFloat(current));
    const per = r2(difference / lines.length);
    lines.forEach((l, i) => {
      const adj = i === lines.length - 1
        ? r2(difference - per * (lines.length - 1))
        : per;
      l.adjustment = adj.toFixed(2);
      l.final_profit = r2(parseFloat(l.base_profit) + adj).toFixed(2);
      l.price = r2(parseFloat(l.cost) + parseFloat(l.final_profit) + parseFloat(l.iva)).toFixed(2);
    });
    report.target = t.toFixed(2);
    report.difference = difference.toFixed(2);
    report.adjustment_per_line = per.toFixed(2);
    recomputeTotals(report);
  }
  return report;
}

function emptyTotals(): ProfitTotals {
  const z = '0.00';
  return { cost: z, base_profit: z, adjustment: z, final_profit: z, iva: z, price: z };
}

function recomputeTotals(r: ProfitReport) {
  const t = emptyTotals();
  const acc = (key: keyof ProfitLine) =>
    r.lines.reduce((n, l) => n + parseFloat(String(l[key])), 0);
  t.cost = r2(acc('cost')).toFixed(2);
  t.base_profit = r2(acc('base_profit')).toFixed(2);
  t.adjustment = r2(acc('adjustment')).toFixed(2);
  t.final_profit = r2(acc('final_profit')).toFixed(2);
  t.iva = r2(acc('iva')).toFixed(2);
  t.price = r2(acc('price')).toFixed(2);
  r.totals = t;
}

// Fase 2: costo del ensamble — precios a MXN (T.C efectivo), subtotal,
// IVA 16% y total. Mismo redondeo comercial (mitad hacia arriba) que Rust.
function computeCost(a: Assembly): CostBreakdown {
  const eff = ceilRate(state.rate);
  let subtotal = 0;
  for (const items of Object.values(a.slots)) {
    for (const item of items ?? []) {
      const c = state.components.find((x) => x.id === item.component_id);
      if (!c) continue;
      const price =
        c.price.currency === 'USD' ? parseFloat(c.price.amount) * eff : parseFloat(c.price.amount);
      subtotal += price * item.quantity;
    }
  }
  subtotal = Math.round(subtotal * 100) / 100;
  const iva = Math.round(subtotal * 16) / 100;
  const mxp = (n: number) => ({ amount: n.toFixed(2), currency: 'MXP' as const });
  return { subtotal: mxp(subtotal), iva: mxp(iva), total: mxp(subtotal + iva) };
}

function validate(a: Assembly): ValidationReport {
  const issues: ValidationIssue[] = [];
  const qty = (slot: string) =>
    (a.slots[slot as keyof typeof a.slots] ?? []).reduce((n, i) => n + i.quantity, 0);
  const items = (slot: string) => a.slots[slot as keyof typeof a.slots] ?? [];
  const comp = (id: string) => state.components.find((c) => c.id === id);

  for (const [slot, msg] of MINIMUMS) {
    if (qty(slot) === 0) {
      issues.push({ severity: 'error', code: `missing_${slot}`, message: msg });
    }
  }

  // PSU: dedicada, o incluida en gabinete
  const caseWithPsu = items('case')
    .map((i) => comp(i.component_id))
    .find((c) => c && (c.specs as Record<string, unknown>).includes_psu === true);
  let psu: ValidationReport['psu'];
  if (qty('psu') > 0) {
    psu = { kind: 'dedicated' };
    if (caseWithPsu) {
      issues.push({
        severity: 'info',
        code: 'psu_discarded',
        message: 'El gabinete incluye fuente; al elegir una dedicada, la incluida se descarta.',
      });
    }
  } else if (caseWithPsu) {
    const watts = (caseWithPsu.specs as Record<string, unknown>).psu_watts;
    const detail = `${caseWithPsu.description.slice(0, 60)}… — Fuente incluida de ${watts ?? '?'}W`;
    psu = { kind: 'included_in_case', detail };
    issues.push({
      severity: 'info',
      code: 'psu_from_case',
      message: `La fuente de poder queda cubierta por la del gabinete: ${detail}`,
    });
  } else {
    psu = { kind: 'missing' };
    issues.push({
      severity: 'error',
      code: 'missing_psu',
      message: 'Falta la fuente de poder: agrega una dedicada o elige un gabinete que la incluya.',
    });
  }

  // Enfriamiento según disipador del CPU
  const cpus = items('cpu').map((i) => comp(i.component_id)).filter(Boolean) as SeedComponent[];
  const allIncludeCooler =
    cpus.length > 0 && cpus.every((c) => (c.specs as Record<string, unknown>).includes_cooler === true);
  let cooling: ValidationReport['cooling'];
  if (allIncludeCooler) {
    if (qty('cooling') > 0) {
      issues.push({
        severity: 'info',
        code: 'cooling_optional',
        message: 'El procesador ya incluye disipador; el enfriamiento extra es opcional.',
      });
    }
    cooling = { kind: 'included_in_cpu', detail: cpus.map((c) => c.description.slice(0, 60)).join(', ') };
  } else if (qty('cooling') > 0) {
    cooling = {
      kind: 'provided',
      detail: items('cooling').map((i) => comp(i.component_id)?.description.slice(0, 60)).join(', ') ?? '',
    };
  } else {
    cooling = { kind: 'missing' };
    issues.push({
      severity: 'error',
      code: 'missing_cooling',
      message: 'El procesador no incluye disipador: agrega enfriamiento por aire o líquido.',
    });
  }

  // Componentes usados → info; SODIMM en ensamble → error (regla de escritorio)
  for (const c of Object.values(a.slots).flat().map((i) => comp(i!.component_id))) {
    if (c?.kind === 'ram' && (c.specs as Record<string, unknown>).ram_form === 'SODIMM') {
      issues.push({
        severity: 'error',
        code: 'sodimm_not_supported',
        message: `"${c.description.slice(0, 60)}" es una memoria SODIMM (para laptop) y no puede ir en un ensamble de escritorio.`,
      });
    }
    if (c?.condition === 'usado') {
      issues.push({
        severity: 'info',
        code: 'used_component',
        message: `"${c.description.slice(0, 60)}" es un componente usado.`,
      });
    }
  }

  return { is_valid: !issues.some((i) => i.severity === 'error'), issues, psu, cooling };
}

export async function mockInvoke(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
  await ensureSeed();
  await new Promise((r) => setTimeout(r, 20));
  switch (cmd) {
    case 'catalog_overview': {
      const counts: Record<string, number> = {};
      for (const c of state.components) counts[c.kind] = (counts[c.kind] ?? 0) + 1;
      return {
        source_file: seedFile().source_file,
        list_date: seedFile().list_date,
        exchange_rate: { pesos_per_dollar: state.rate },
        effective_rate: { pesos_per_dollar: String(ceilRate(state.rate)) },
        exchange_rule: seedFile().exchange_rule,
        rate_is_overridden: state.rateOverridden,
        total_components: state.components.length,
        counts_by_kind: counts,
      } satisfies CatalogOverview;
    }
    case 'list_components':
      return state.components.filter((c) => c.kind !== 'other').map(asDto);
    case 'get_component':
      return asDto(state.components.find((c) => c.id === args.id) ?? state.components[0]);
    case 'add_manual_component': {
      const input = args.input as Record<string, unknown>;
      const esPch = input.supplier === 'pch';
      const c: SeedComponent = {
        id: crypto.randomUUID(),
        sku: (input.sku as string) || (esPch ? `PCH-${Date.now()}` : `MANUAL-${Date.now()}`),
        barcode: null,
        description: input.description as string,
        brand: (input.brand as string) || 'SIN MARCA',
        department: 'CAPTURA MANUAL',
        kind: input.kind as string,
        condition: (input.condition as 'nuevo' | 'usado') ?? 'usado',
        price: {
          amount: input.price_amount as string,
          currency: input.price_currency as 'USD' | 'MXP',
        },
        on_promotion: false,
        stock: {},
        source: 'manual',
        supplier: esPch ? 'pch' : 'manual',
        specs: (input.specs ?? {}) as Record<string, unknown>,
      };
      state.components.push(c);
      return asDto(c);
    }
    case 'remove_manual_component':
      state.components = state.components.filter((c) => c.id !== args.id);
      return true;
    case 'get_rate':
    case 'set_rate': {
      if (cmd === 'set_rate') {
        state.rate = String(args.rate);
        state.rateOverridden = true;
      }
      return {
        rate: { pesos_per_dollar: state.rate },
        list_rate: { pesos_per_dollar: seedFile().exchange_rate },
        effective_rate: { pesos_per_dollar: String(ceilRate(state.rate)) },
        exchange_rule: seedFile().exchange_rule,
        source_file: seedFile().source_file,
        list_date: seedFile().list_date,
        is_overridden: state.rateOverridden,
        zai_key: '',
      } satisfies RateInfo;
    }
    case 'create_assembly': {
      const asm = newAssembly(String(args.name), String(args.quoteType ?? 'personalizado'));
      state.assemblies.set(asm.id, asm);
      return asm;
    }
    case 'duplicate_assembly': {
      const src = state.assemblies.get(String(args.id));
      if (!src) throw new Error('Ensamble no encontrado');
      const copy: Assembly = JSON.parse(JSON.stringify(src));
      copy.id = crypto.randomUUID();
      copy.name = `${src.name} (personalizado)`;
      copy.created_at = new Date().toISOString();
      copy.updated_at = copy.created_at;
      copy.confirmed_at = null;
      copy.quote_type = 'personalizado';
      copy.in_fabrication = false;
      copy.quote_number = null;
      copy.based_on = src.quote_number
        ? `#${src.quote_number} ${src.name}`
        : src.name;
      state.assemblies.set(copy.id, copy);
      return copy;
    }
    case 'set_fabrication': {
      const asm = state.assemblies.get(String(args.id));
      if (!asm) throw new Error('Ensamble no encontrado');
      asm.in_fabrication = !!args.inFabrication;
      asm.updated_at = new Date().toISOString();
      return asm;
    }
    case 'assign_quote_number': {
      const asm = state.assemblies.get(String(args.id));
      if (!asm) throw new Error('Ensamble no encontrado');
      const now = new Date();
      const key = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
      const n = (state.quoteCounters.get(key) ?? 0) + 1;
      state.quoteCounters.set(key, n);
      const dd = String(now.getDate()).padStart(2, '0');
      const mm = String(now.getMonth() + 1).padStart(2, '0');
      const yy = String(now.getFullYear()).slice(-2);
      asm.quote_number = `${dd}${mm}${yy}-${n}`;
      return asm.quote_number;
    }
    case 'list_assemblies':
      return [...state.assemblies.values()]
        .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
        .map(summarize);
    case 'get_assembly':
      return state.assemblies.get(String(args.id)) ?? null;
    case 'rename_assembly': {
      const asm = state.assemblies.get(String(args.id));
      if (asm) asm.name = String(args.name);
      return asm ?? null;
    }
    case 'delete_assembly':
      return state.assemblies.delete(String(args.id));
    case 'set_assembly_item': {
      const asm = state.assemblies.get(String(args.assemblyId));
      if (!asm) throw new Error('Ensamble no encontrado');
      const slot = String(args.slot);
      const compId = String(args.componentId);
      const c = state.components.find((x) => x.id === compId);
      if (!c) throw new Error('Componente no encontrado en el catálogo');
      if (!SLOT_KINDS[slot]?.includes(c.kind)) {
        throw new Error(`Un componente de tipo ${c.kind} no puede ir en el slot ${slot}`);
      }
      if (c.kind === 'ram' && (c.specs as Record<string, unknown>).ram_form === 'SODIMM') {
        throw new Error('Las memorias SODIMM son para laptop y no pueden ir en un ensamble de escritorio');
      }
      const list = (asm.slots[slot as keyof typeof asm.slots] ??= []);
      const existing = list.find((i) => i.component_id === compId);
      const quantity = Number(args.quantity);
      if (existing) {
        if (quantity === 0) list.splice(list.indexOf(existing), 1);
        else existing.quantity = quantity;
      } else if (quantity > 0) {
        list.push({ component_id: compId, quantity });
      }
      asm.updated_at = new Date().toISOString();
      return asm;
    }
    case 'clear_assembly_slot': {
      const asm = state.assemblies.get(String(args.assemblyId));
      if (!asm) throw new Error('Ensamble no encontrado');
      delete asm.slots[String(args.slot) as keyof typeof asm.slots];
      asm.updated_at = new Date().toISOString();
      return asm;
    }
    case 'validate_assembly': {
      const asm = state.assemblies.get(String(args.assemblyId));
      if (!asm) throw new Error('Ensamble no encontrado');
      return validate(asm);
    }
    case 'assembly_cost': {
      const asm = state.assemblies.get(String(args.assemblyId));
      if (!asm) throw new Error('Ensamble no encontrado');
      return computeCost(asm);
    }
    case 'confirm_assembly': {
      const asm = state.assemblies.get(String(args.assemblyId));
      if (!asm) throw new Error('Ensamble no encontrado');
      const r = validate(asm);
      if (!r.is_valid) {
        throw new Error('El ensamble está incompleto: ' +
          r.issues.filter(i => i.severity === 'error').map(i => i.message).join(' '));
      }
      asm.confirmed_at = new Date().toISOString();
      return asm;
    }
    case 'catalog_filters': {
      const brands = [...new Set(state.components.map((c) => c.brand))].sort();
      const departments = [...new Set(state.components.map((c) => c.department))].sort();
      return { brands, departments };
    }
    case 'list_spec_options': {
      const seed: Record<string, string[]> = {
        socket: ['AM4', 'AM5', 'LGA1700', 'LGA1851', 'LGA1200', 'LGA1151'],
        ddr: ['DDR3', 'DDR4', 'DDR5'],
        form_factor: ['ATX', 'MICRO ATX', 'MINI ITX'],
        ram_form: ['DIMM', 'SODIMM'],
      };
      return seed[String(args.kind)] ?? [];
    }
    case 'add_spec_option':
      throw new Error('Los catálogos de atributos solo se administran en la aplicación de escritorio.');
    case 'add_brand':
      throw new Error('Las marcas solo se administran en la aplicación de escritorio.');
    case 'import_catalog':
    case 'import_catalog_xlsx':
      throw new Error('Importar catálogo solo está disponible en la aplicación de escritorio.');
    case 'ai_normalize_titles':
      throw new Error('La normalización con IA solo está disponible en la aplicación de escritorio.');
    case 'set_zai_key':
      return null;
    case 'get_zai_key':
      return '';
    case 'get_component_by_sku': {
      const found = state.components.find((c) => c.sku === String(args.sku));
      return found ? asDto(found) : null;
    }
    case 'next_quote_number': {
      const now = new Date();
      const key = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
      const n = (state.quoteCounters.get(key) ?? 0) + 1;
      state.quoteCounters.set(key, n);
      const dd = String(now.getDate()).padStart(2, '0');
      const mm = String(now.getMonth() + 1).padStart(2, '0');
      const yy = String(now.getFullYear()).slice(-2);
      return `${dd}${mm}${yy}-${n}`;
    }
    case 'next_sale_note_number': {
      const now = new Date();
      const key = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
      const n = (state.saleNoteCounters.get(key) ?? 0) + 1;
      state.saleNoteCounters.set(key, n);
      const dd = String(now.getDate()).padStart(2, '0');
      const mm = String(now.getMonth() + 1).padStart(2, '0');
      const yyyy = now.getFullYear();
      return `${dd}${mm}${yyyy}-${n}`;
    }
    case 'record_sale_note': {
      const asmId = args.assemblyId ?? args.assembly_id ?? null;
      if (asmId) {
        const asm = state.assemblies.get(String(asmId));
        if (!asm) throw new Error('Ensamble no encontrado');
        if (statusOf(asm) === 'caducada') {
          throw new Error('La cotización está CADUCADA: no se puede generar una nota de venta.');
        }
        const r = validate(asm);
        if (!r.is_valid) {
          throw new Error('El ensamble está incompleto: ' +
            r.issues.filter(i => i.severity === 'error').map(i => i.message).join(' '));
        }
        asm.confirmed_at = new Date().toISOString();
      }
      const record = {
        number: String(args.number),
        quote_number: (args.quote_number as string | null) ?? null,
        assembly_id: asmId ? String(asmId) : null,
        created_at: new Date().toISOString(),
        lines: (args.lines ?? []) as unknown[],
      };
      state.saleNotes.push(record);
      return record;
    }
    case 'cart_profit': {
      const items = (args.items ?? []) as CartItem[];
      const percent = parseFloat(String(args.percent));
      if (isNaN(percent) || percent < 0) throw new Error('Porcentaje inválido');
      const report = computeCartProfit(items, percent);
      const target = args.target as string | null;
      if (target != null && target !== '' && report.lines.length > 0) {
        // mismas reglas que assembly: reparto sobre el subtotal y IVA recalculado
        const t = parseFloat(target);
        const current = report.totals.price;
        const difference = r2(t - parseFloat(current));
        const deltaSubtotal = r2(difference / 1.16);
        const per = r2(deltaSubtotal / report.lines.length);
        report.lines.forEach((l, i) => {
          const adj = i === report.lines.length - 1
            ? r2(deltaSubtotal - per * (report.lines.length - 1))
            : per;
          l.adjustment = adj.toFixed(2);
          l.final_profit = r2(parseFloat(l.base_profit) + adj).toFixed(2);
          l.iva = r2((parseFloat(l.cost) + parseFloat(l.final_profit)) * 0.16).toFixed(2);
          l.price = r2(parseFloat(l.cost) + parseFloat(l.final_profit) + parseFloat(l.iva)).toFixed(2);
        });
        const totalNow = r2(report.lines.reduce((n, l) => n + parseFloat(l.price), 0));
        const residual = r2(t - totalNow);
        if (residual !== 0) {
          const last = report.lines[report.lines.length - 1];
          last.iva = r2(parseFloat(last.iva) + residual).toFixed(2);
          last.price = r2(parseFloat(last.cost) + parseFloat(last.final_profit) + parseFloat(last.iva)).toFixed(2);
        }
        report.target = t.toFixed(2);
        report.difference = difference.toFixed(2);
        report.adjustment_per_line = per.toFixed(2);
        recomputeTotals(report);
      }
      return report;
    }
    case 'list_sale_notes':
      return state.saleNotes;
    case 'save_export':
      // En navegador la descarga la maneja el <a download>; no hay disco.
      return String(args.path);
    case 'assembly_profit': {
      const asm = state.assemblies.get(String(args.assemblyId));
      if (!asm) throw new Error('Ensamble no encontrado');
      const percent = parseFloat(String(args.percent));
      if (isNaN(percent) || percent < 0) throw new Error('Porcentaje inválido');
      return computeProfit(asm, percent, args.target as string | null);
    }
    case 'convert_money': {
      const eff = ceilRate(state.rate);
      const amount = String(args.currency) === 'USD'
        ? (parseFloat(String(args.amount)) * eff).toFixed(2)
        : String(args.amount);
      return { amount, currency: 'MXP' };
    }
    default:
      throw new Error(`Mock: comando desconocido ${cmd}`);
  }
}
