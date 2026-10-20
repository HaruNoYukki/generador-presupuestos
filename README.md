# Generador de Presupuestos

Aplicación de escritorio (Rust + Tauri v2) con **dos flujos**:

1. **Ensambles**: cotización → ganancias → nota de venta de computadoras
   armadas con componentes del catálogo.
2. **Ventas individuales**: carrito de cualquier producto (sin cotización),
   con ganancia %/objetivo, N/S opcionales y nota de venta directa.

El inventario disponible para vender es únicamente **A-CEN, AVE-2 y CEDIS**
(CMAL y GDL no cuentan como stock vendible).

## Arquitectura — monolito modular

Un solo binario desplegable con módulos internos bien delimitados:

```
src-tauri/src/
├── db.rs            SQLite (presupuestos.db): productos, ensambles, notas,
│                    contadores y ajustes. SKU = clave de upsert/consulta
├── domain/          Dominio puro, sin dependencias de Tauri ni del SO
│   ├── money.rs       Monedas, dinero y la regla del tipo de cambio
│   ├── component.rs   Componente, tipos (CPU, RAM, PSU…), specs, stock
│   ├── assembly.rs    Slots del ensamble, partidas y REGLAS DE VALIDACIÓN
│   ├── cost.rs        Costo: subtotal MXN + IVA 16% = total
│   └── profit.rs      Ganancias: %, IVA recalculado con objetivo
├── catalog/         Catálogo: semilla de la lista + componentes manuales
├── store.rs         Estado de la app + persistencia JSON (app data dir)
├── commands.rs      Comandos Tauri (frontera hacia la GUI, capa delgada)
└── lib.rs           Composición y registro de módulos
src/                  GUI (TypeScript + Vite, sin framework)
data/
├── raw/              Lista de precios original (xlsx)
└── catalog_seed.json Semilla generada (se embebe en el binario)
scripts/
└── extract_catalog.py  xlsx → catalog_seed.json (clasificación por tipo)
```

## Reglas de negocio implementadas (fase 1)

- **Mínimos por ensamble**: 1 procesador, 1 RAM, 1 almacenamiento (SSD/HDD),
  1 tarjeta madre, 1 gabinete y fuente de poder.
- **Fuente de poder**: obligatoria, salvo que el gabinete elegido incluya
  una (p. ej. "GABINETE … CON FUENTE 600W"). En ese caso el campo PSU queda
  cubierto por la especificación incluida; si se agrega una dedicada, la
  incluida se descarta (aviso informativo).
- **Enfriamiento**: obligatorio (aire o líquido) salvo que el procesador
  incluya disipador. Detección desde la lista: "SIN DISIPADOR" /
  "NO INCLUYE DISIPADOR" / SKU terminado en `WOF` → sin disipador;
  SKU `BOX` o "CON DISIPADOR" → con disipador. Si la descripción no lo
  aclara, se exige enfriamiento (criterio conservador).
- **Opcionales**: monitores, teclados, mouse, GPUs y tarjetas de expansión.
  Un kit teclado+mouse cubre ambos slots.
- **Precios en USD o MXP**. Conversión USD→MXP con el tipo de cambio de la
  lista **redondeado hacia arriba al entero superior** (T.C 18.52 → 19).
  Se puede capturar otro T.C desde Ajustes.
- **Componentes**: la lista trae productos nuevos; los usados se capturan
  manualmente (condición "Usado", alta con especificaciones rápidas).
- **Orden del selector**: al elegir componentes, la lista se ordena por
  precio de menor a mayor (en MXN). Las memorias SODIMM son para laptop y
  quedan excluidas de los ensambles (el dominio las rechaza y la validación
  marca `sodimm_not_supported` si alguna quedó persistida).
- **Costo del ensamble (fase 2)**: todos los precios se convierten a MXN,
  se suman (subtotal) y sobre ellos se calcula el IVA de 16%. El costo del
  ensamble es subtotal + IVA. Se muestra en vivo en la vista Ensamble y se
  recalcula con cada cambio (incluido el T.C capturado en Ajustes).

## Uso

```bash
npm install          # dependencias del front-end
npm run tauri dev    # desarrollo (abre la ventana de la app)
npm run tauri build  # build de producción (dmg/appImage/msi según SO)
cargo test           # tests del dominio (en src-tauri/)
```

En navegador sin Tauri (`npx vite`), la GUI corre contra un mock que replica
las mismas reglas (`src/mock.ts`) — útil para maquetar la interfaz.

## Actualizar el catálogo con una lista nueva

En la app: **Ajustes → "⤒ Importar Excel del proveedor (.xlsx)"** y elige
la lista. No hace falta Python ni recompilar: la app lee el Excel, detecta
el proveedor por sus encabezados y hace **UPSERT por SKU** (actualiza los
existentes conservando su id; inserta los nuevos). Si la lista trae
tipo de cambio y fecha, también se actualizan.

### Proveedores (src-tauri/src/suppliers.rs)

Cada formato de lista es un **adaptador** (`SupplierAdapter`) con tres
peculiaridades: nombre, detección por firma de encabezados y valores por
defecto (moneda/marca/departamento). El resto — lectura del libro con
calamine, mapeo de columnas por nombre (insensible a orden, acentos y
mayúsculas), fila de T.C/fecha y la clasificación de tipos — es compartido.

- **nykniu**: formato actual completo (departamento, moneda, promoción,
  almacenes A-CEN/AVE-2/CEDIS/CMAL/GDL, fila de T.C).
- **generico**: fallback para otros proveedores — bastan SKU +
  descripción + costo; el tipo de producto se infiere de la descripción.

Para agregar un proveedor nuevo: implementa `SupplierAdapter` con su
`detect()` y agrégalo a `adapters()` (el orden importa: específico antes
que genérico).

(Avanzado) El flujo JSON de `scripts/extract_catalog.py` sigue disponible
en Ajustes → "Importar catálogo (JSON)".

## Persistencia

Todo vive en **SQLite** (`presupuestos.db` en el directorio de datos de la
app, modo WAL). El esquema está documentado en `docs/esquema.mmd`
(diagrama Mermaid). Primera corrida: registra los 602 productos de la
semilla embebida. Si existe el `state.json` de versiones anteriores, se
migra automáticamente una vez y queda como `state.json.migrated`.

El script extrae SKU, código de barras, descripción, departamento, moneda,
marca, promoción, existencias por almacén (A-CEN, AVE-2, CEDIS, CMAL, GDL),
costo y el T.C de la fila 2, y clasifica cada artículo por tipo de componente
con especificaciones (watts de PSU, PSU incluida en gabinete, disipador del
CPU, DDR/capacidad de RAM, SSD/HDD, VRAM…).

## Estado — fases

- [x] Fase 1 — Catálogo, dominio, ensamble y validación
- [x] Fase 2 — Módulo de cálculo de costo
- [x] Fase 3 — Cálculo de ganancias (presets 10/20/30%, % libre, precio
  objetivo con reparto equitativo; el IVA se recalcula con el objetivo)
- [x] Fase 4 — Cotización NYKNIU exportable (PDF/PNG, número DDMMAA-NC;
  la vista de resumen queda pospuesta)
- [x] Fase 5 — Nota de venta (N/S por unidad, ligada a la cotización,
  número DDMMAAAA-NC, exportable PDF/PNG)
- [ ] Fase 4 — Resumen
- [ ] Fase 5 — Notas de pedido
- [ ] Fase 6 — Notas de venta

- **Ganancias (fase 3)**: al confirmar un ensamble completo (botón
  "✓ Confirmar" — lo guarda como cotización), la pestaña Ganancias muestra
  la tabla por componente: costo sin IVA, ganancia (presets 10/20/30% o
  porcentaje libre), IVA de 16% y precio, con las sumas de cada columna.
  El precio objetivo reparte la diferencia equitativamente entre
  componentes ajustando la ganancia (columna "Ajuste") y el IVA SE
  RECALCULA sobre la nueva base, de modo que Subtotal + IVA = Total
  siempre coincide.
- **Tipos y estados de cotización**: las cotizaciones son **preensambles**
  (caducan el día 1 del mes siguiente) o **personalizadas** (caducan 15
  días naturales después de generarse; si una preensamble se genera el
  último día del mes, también recibe esos 15 días). Estados: **vigente**,
  **en fabricación** (lo marca el negocio: no caduca y siempre puede
  venderse) y **caducada** (no puede generar nota de venta). Un
  personalizado puede usar de base un preensamble con
  "⧉ Duplicar como personalizado" (nace sin folio ni confirmación). La
  barra de cotizaciones incluye el filtro **"Solo preensambles vigentes"**
  y la **búsqueda por # de cotización**.
- **Cotización / nota de pedido (fase 4)**: pestaña "Cotización" con el
  formato NYKNIU: encabezado con logo, número DDMMAA-NC (consecutivo por
  día, persistente, ligado al ensamble), correo ventas@nykniu.com; tabla
  Tipo/Componente/Ud./Costo (las líneas suman el Total, como en el PDF
  original); desglose Subtotal + IVA = Total; leyendas de garantía y firma
  "Atentamente — Nykniu". Exporta a PDF o PNG (diálogo nativo en la app).
  **Tipografía única**: todo el documento (tabla, leyendas y firma) usa la
  misma monospace de la tabla, también en los exports.
- **Nota de venta (fase 5)**: pestaña "Nota de venta" con el formato del
  NV.pdf: título NOTA DE VENTA, número propio DDMMAAAA-NC (independiente
  del de cotización), correo ventas@nykniu.com sin teléfono, y columna
  N/S. Una línea por unidad: si un SKU lleva 2+ piezas se generan tantos
  renglones como unidades, cada uno con su número de serie capturable.
  La nota queda ligada a la cotización del ensamble (mismo % y objetivo):
  el precio de cada componente y el total coinciden exactamente. Emitir
  la nota exige número y todos los N/S capturados; queda registrada con
  su cotización en el estado persistente.

## Datos

Ubicación de la base: `~/Library/Application Support/com.harunoyukki.generador-presupuestos/presupuestos.db`
(macOS; equivalente en Linux/Windows).
