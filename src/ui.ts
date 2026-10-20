// Helpers de DOM, formato y modales.

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | boolean | undefined | ((e: Event) => void)> = {},
  ...children: (Node | string | null | undefined)[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === null) continue;
    if (key === 'class') node.className = String(value);
    else if (key.startsWith('on') && typeof value === 'function')
      node.addEventListener(key.slice(2), value as EventListener);
    else if (typeof value === 'boolean') {
      if (value) node.setAttribute(key, '');
    } else node.setAttribute(key, String(value));
  }
  for (const child of children) {
    if (child === null || child === undefined) continue;
    node.append(child instanceof Node ? child : document.createTextNode(child));
  }
  return node;
}

const mxnFmt = new Intl.NumberFormat('es-MX', {
  style: 'currency',
  currency: 'MXN',
  minimumFractionDigits: 2,
});

export function money(m: { amount: string; currency: string }): string {
  const n = parseFloat(m.amount);
  return `${m.currency === 'USD' ? 'US$' : ''}${mxnFmt.format(n)}`;
}

export function mxn(amount: string): string {
  return mxnFmt.format(parseFloat(amount));
}

/** Chip compacto del tipo de cotización (preensamble/personalizado). */
export function typeChip(quoteType: string | null | undefined): HTMLElement {
  const isPre = quoteType === 'preensamble';
  return el('span', { class: `chip-type ${isPre ? 'preensamble' : 'personalizado'}` },
    isPre ? 'PREENSAMBLE' : 'PERSONALIZADO');
}

export function clear(node: HTMLElement) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

let activeOverlay: HTMLElement | null = null;

export function closeModal() {
  activeOverlay?.remove();
  activeOverlay = null;
}

export function modal(title: string, body: HTMLElement): HTMLElement {
  closeModal();
  const box = el('div', { class: 'modal-box' },
    el('header', { class: 'modal-head' },
      el('h3', {}, title),
      el('button', { class: 'icon-btn', onclick: closeModal }, '✕'),
    ),
    body,
  );
  const overlay = el('div', { class: 'modal-overlay', onclick: (e) => {
    if (e.target === overlay) closeModal();
  } }, box);
  document.body.append(overlay);
  activeOverlay = overlay;
  return box;
}
