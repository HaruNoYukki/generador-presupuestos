// Metadatos de tipos y slots compartidos por las vistas.
import type { Kind, SlotId } from './types';

export const KIND_LABELS: Record<Kind, string> = {
  cpu: 'Procesador',
  motherboard: 'Tarjeta madre',
  ram: 'Memoria RAM',
  storage: 'Almacenamiento',
  psu: 'Fuente de poder',
  case: 'Gabinete',
  cooler_air: 'Enfriamiento aire',
  cooler_liquid: 'Enfriamiento líquido',
  gpu: 'Tarjeta de video',
  monitor: 'Monitor',
  keyboard: 'Teclado',
  mouse: 'Mouse',
  keyboard_mouse_kit: 'Kit teclado + mouse',
  expansion_card: 'Tarjeta de expansión',
  other: 'Otro',
};

export const PICKABLE_KINDS: Kind[] = [
  'cpu',
  'motherboard',
  'ram',
  'storage',
  'psu',
  'case',
  'cooler_air',
  'cooler_liquid',
  'gpu',
  'monitor',
  'keyboard',
  'mouse',
  'keyboard_mouse_kit',
  'expansion_card',
];

export interface SlotMeta {
  id: SlotId;
  label: string;
  /** Obligatorio para que el ensamble sea válido. */
  required: boolean;
  /** Acepta más de una unidad. */
  multiple: boolean;
  /** Tipos de componente que puede recibir. */
  kinds: Kind[];
  note?: string;
}

export const SLOTS: SlotMeta[] = [
  { id: 'cpu', label: 'Procesador', required: true, multiple: false, kinds: ['cpu'], note: 'Si no incluye disipador, el enfriamiento es obligatorio.' },
  { id: 'motherboard', label: 'Tarjeta madre', required: true, multiple: false, kinds: ['motherboard'] },
  { id: 'ram', label: 'Memoria RAM', required: true, multiple: true, kinds: ['ram'] },
  { id: 'storage', label: 'Almacenamiento', required: true, multiple: true, kinds: ['storage'], note: 'SSD o HDD, al menos uno.' },
  {
    id: 'psu',
    label: 'Fuente de poder',
    required: true,
    multiple: false,
    kinds: ['psu'],
    note: 'Obligatoria, salvo que el gabinete incluya una.',
  },
  { id: 'case', label: 'Gabinete', required: true, multiple: false, kinds: ['case'] },
  { id: 'cooling', label: 'Enfriamiento', required: false, multiple: true, kinds: ['cooler_air', 'cooler_liquid'], note: 'Obligatorio si el procesador no trae disipador.' },
  { id: 'gpu', label: 'Tarjeta de video', required: false, multiple: true, kinds: ['gpu'] },
  { id: 'monitor', label: 'Monitor', required: false, multiple: true, kinds: ['monitor'] },
  { id: 'keyboard', label: 'Teclado', required: false, multiple: true, kinds: ['keyboard', 'keyboard_mouse_kit'] },
  { id: 'mouse', label: 'Mouse', required: false, multiple: true, kinds: ['mouse', 'keyboard_mouse_kit'] },
  { id: 'expansion_card', label: 'Tarjeta de expansión', required: false, multiple: true, kinds: ['expansion_card'] },
];

export function slotMeta(id: SlotId): SlotMeta {
  const s = SLOTS.find((s) => s.id === id);
  if (!s) throw new Error(`Slot desconocido: ${id}`);
  return s;
}

export function kindLabel(k: string): string {
  return KIND_LABELS[k as Kind] ?? k;
}
