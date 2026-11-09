// Iconografía de la marca (manual §6): set Lucide, el mismo que el
// storefront usa con lucide-react. Los emojis están prohibidos en la UI.
import { createElement, type IconNode } from 'lucide';

export {
  ArrowUpRight, ChartColumn, Check, ChevronLeft, ChevronRight, Cog, Copy,
  Download, Globe, Info, Lock, Minus, Pencil, PenLine, Play, Plus, RefreshCw,
  Repeat, RotateCcw, Sparkles, Trash, TriangleExclamationPoint, Upload, X,
} from 'lucide';

/** Tamaños canónicos (manual §6): 12 en chips, 16 en línea con texto y
 * botones, 20 en navegación, 24 en controles grandes. */
export type IconSize = 12 | 16 | 20 | 24;

/** SVG de Lucide que hereda el color del texto (`currentColor`). */
export function icon(node: IconNode, size: IconSize = 16): SVGElement {
  return createElement(node, {
    width: size,
    height: size,
    class: 'icon',
    'aria-hidden': 'true',
  });
}
