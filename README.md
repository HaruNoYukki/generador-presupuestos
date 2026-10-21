# Generador de Presupuestos

Aplicación de escritorio (Rust + Tauri v2) para cotizar y vender
computadoras armadas y productos individuales, con inventario multi-proveedor
(arroba y CVA), cálculo de ganancias, notas de venta exportables a PDF/PNG y
base de datos SQLite normalizada.

## Requisitos

- [Rust](https://rustup.rs) (estable)
- [Node.js](https://nodejs.org) 18+
- [LibreOffice](https://www.libreoffice.org) (solo si vas a importar listas
  en `.xls`; sirve para la conversión interna — ver abajo)

## Compilar y ejecutar

```bash
npm install
npm run tauri dev     # desarrollo
npm run tauri build   # producción (.app / .dmg / .msi según tu SO)
```

## Primera corrida (importante)

La aplicación **no incluye catálogo de productos**: cada instalación genera
el suyo desde el Excel de su proveedor. Dos caminos:

### Opción A — Importar Excel directo (recomendada)

1. Abre la app → pestaña **Ajustes**.
2. Botón **"⤒ Importar Excel del proveedor (.xlsx)"** y elige la lista.
3. La app detecta el formato por sus encabezados (CVA, arroba o genérico)
   y hace UPSERT por SKU.

Para actualizar precios después, simplemente importa la lista nueva: los
productos existentes se actualizan (conservando su identidad) y los nuevos
se agregan. No hace falta recompilar.

### Opción B — Semilla JSON (avanzada)

```bash
python3 scripts/extract_catalog.py "ruta/a/lista.xlsx"
# genera data/catalog_seed.json (ignorado por git)
```

La app la detecta automáticamente al arrancar (busca en el directorio de
datos, en los recursos del paquete y en `data/`).

> ⚠️ **`data/catalog_seed.json` no se publica en el repositorio** porque
> contiene precios de proveedores. Si no existe, la app compila y arranca
> con catálogo vacío.

## Proveedores (`src-tauri/src/suppliers.rs`)

Cada formato de lista es un **adaptador** (`SupplierAdapter`): nombre,
detección por firma de encabezados y valores por defecto. La lectura del
Excel (calamine), el mapeo de columnas por nombre (insensible a orden,
acentos y mayúsculas) y la clasificación de tipos son compartidos.

| Adaptador | Formato |
|---|---|
| `arroba` | Lista actual: departamento, moneda, promoción, almacenes A-CEN/AVE-2/CEDIS/CMAL/GDL y fila de T.C |
| `cva` | `.xls` o `.xlsx`: Clave, **Codigo de Fabricante** (SKU), Marca, Grupo, Precio, Moneda, Promos, **Disp** y **Disp CD**, UPC |
| `generico` | Fallback: SKU + descripción + costo; el tipo se infiere de la descripción |
| captura manual | Sin lista: alta a mano desde la app (Catálogo o el picker de ensambles), con existencia propia. Incluye al proveedor **PCH** |

Para agregar un proveedor nuevo: implementa `SupplierAdapter` con su
`detect()` y agrégalo a `adapters()` (específicos antes que genéricos).

### Catálogos de atributos (Ajustes)

Marca, Socket, DDR, Factor de forma y Formato de memoria RAM son
**catálogos normalizados** (tablas `brands` y `spec_options`): los
formularios de alta usan listas desplegables, nunca texto libre, para
evitar problemas de normalización. Valores nuevos se agregan desde
**Ajustes → "Catálogos del producto"** (su propio menú, uno por catálogo).
Los imports de listas también registran automáticamente cualquier valor
nuevo que traigan.

## Inventario multi-proveedor

- El mismo SKU puede existir en varios proveedores con precio y
  existencias distintos (`product_offers`, clave `(sku, supplier)`).
- El catálogo muestra siempre el **menor precio disponible** con su
  procedencia (chip @ o CVA).
- **Inventario desglosado** por proveedor en la vista Catálogo.
- **Stock por flujo**:
  - Ensamble: arroba → CEN+AVE-2+CEDIS; CVA → **Disp**.
  - Venta individual: arroba → igual; CVA → **Disp + Disp CD**.
- Los artículos **solo-CD** (CVA) van bloqueados en el picker de ensambles
  con la marca "SOLO CD · no ensamble".
- En el picker puedes **ver los precios de los demás proveedores** y
  cambiar de proveedor en cualquier partida sin importar quién sea el más
  barato.
- **Alta desde el picker**: cada lista desplegable tiene el botón
  "＋ Agregar [slot] nuevo" para capturar un producto al instante y
  usarlo de inmediato (útil para proveedores sin Excel como PCH).
- **Advertencia PCH en fabricación**: al marcar "En fabricación" un
  ensamble que contiene productos de PCH aparece un banner con cada uno
  (descripción y SKU) pidiendo verificar existencia manualmente.

## Reglas de negocio

- **Tipo de cambio**: USD→MXP con el T.C **redondeado hacia arriba al
  entero** (18.52 → 19). El T.C de referencia es siempre **el más elevado**
  entre las listas importadas.
- **Costo del ensamble**: precios a MXN, suma (subtotal), IVA 16% —
  subtotal + IVA = total, en vivo.
- **Ganancias**: presets 10/20/30% o % libre por componente. Precio
  objetivo: la diferencia se reparte equitativamente ajustando la ganancia
  y el IVA se recalcula — Subtotal + IVA = Total siempre coincide.
- **Validación de ensamble**: mínimos por slot, compatibilidad de socket
  CPU↔MOBO, DDR RAM↔MOBO y factor de forma gabinete↔MOBO (si faltan datos
  del gabinete se marca advertencia para revisión manual). SODIMM
  excluida de ensambles.
- **Cotizaciones**: número DDMMAA-NC consecutivo por día; tipos
  **preensamble** (caduca el día 1 del mes siguiente; si se genera el
  último día del mes, 15 días naturales) y **personalizado** (15 días
  naturales). Estados: vigente, en fabricación (no caduca) y caducada (no
  genera nota de venta).
- **Notas de venta**: número propio DDMMAAAA-NC, ligadas a su cotización
  con precios coincidentes; un renglón por unidad con su número de serie;
  exportación a PDF y PNG con tipografía única monospace.
- **Títulos con IA** (opcional): guarda tu clave API de Z.ai en Ajustes y
  normaliza los títulos del catálogo en lotes de 100. Los ya normalizados
  quedan excluidos de las corridas siguientes.

## Estructura

```
src-tauri/src/
├── db.rs           SQLite (v6): maestro + ofertas + existencias (3FN/BCNF)
├── domain/         Dominio puro: componente, dinero, ensamble, costo,
│                   ganancias, carrito, notas de venta
├── suppliers.rs    Importación de Excel por proveedor (calamine)
├── catalog/        Catálogo en memoria (mejor oferta por SKU)
├── store.rs        Estado de la app y persistencia
├── commands.rs     Comandos Tauri (frontera hacia la GUI)
├── ai.rs           Normalización de títulos con Z.ai
└── lib.rs          Composición

src/                GUI (TypeScript + Vite, sin framework)
docs/esquema.mmd    Diagrama de la base (Mermaid)
scripts/            extract_catalog.py (semilla JSON opcional)
```

## Tests

```bash
cd src-tauri
cargo test
```

Los fixtures de prueba (`tests/fixtures/`) contienen **datos sintéticos**;
las listas reales de proveedores no se publican en el repositorio.

## Persistencia

SQLite en modo WAL: `presupuestos.db` en el directorio de datos de la app
(macOS: `~/Library/Application Support/com.harunoyukki.generador-presupuestos/`).
Migraciones automáticas con control de versión (`PRAGMA user_version`).

## Licencia

Pendiente de definir.
