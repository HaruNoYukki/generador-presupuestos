//! Persistencia SQLite — esquema v5/v6, normalizado a 3FN/BCNF.
//!
//! - `suppliers`, `brands`, `departments`, `warehouses`: catálogos.
//! - `products`: maestro del SKU (datos intrínsecos; marca y departamento
//!   por FK; `specs` queda en JSON por decisión de diseño acordada;
//!   `title_source` marca si el título ya pasó por la IA: 'raw' | 'ai').
//! - `product_offers` + `offer_stock`: precio y existencias por
//!   (sku, proveedor, almacén). Varios proveedores pueden ofrecer el mismo
//!   SKU con precios y cantidades distintos.
//! - `assemblies` + `assembly_items`: partidas normalizadas (sin JSON),
//!   con el proveedor elegido por partida.
//! - `sale_notes` + `sale_note_lines`: renglones normalizados (snapshot
//!   congelado de descripción y precio, correcto contablemente).
//! - `counters`, `settings`: triviales.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::domain::assembly::{Assembly, AssemblyItem, Slot};
use crate::domain::component::{
    Component, ComponentKind, ComponentSource, Condition, Offer, Specs,
};
use crate::domain::money::{Currency, Money};
use crate::domain::sale::SaleNoteRecord;

pub const DB_FILE: &str = "presupuestos.db";

/// Abre (o crea) la base de datos en `dir` y aplica las migraciones.
pub fn open(dir: &Path) -> rusqlite::Result<Connection> {
    std::fs::create_dir_all(dir).ok();
    let conn = Connection::open(dir.join(DB_FILE))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

// ---------------------------------------------------------------------
// Migraciones
// ---------------------------------------------------------------------

fn create_v1(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS products (
            id              TEXT PRIMARY KEY,
            sku             TEXT NOT NULL UNIQUE,
            barcode         TEXT,
            description     TEXT NOT NULL,
            brand           TEXT NOT NULL,
            department      TEXT NOT NULL,
            kind            TEXT NOT NULL,
            condition       TEXT NOT NULL,
            price_amount    TEXT NOT NULL,
            price_currency  TEXT NOT NULL,
            on_promotion    INTEGER NOT NULL DEFAULT 0,
            stock           TEXT NOT NULL DEFAULT '{}',
            specs           TEXT NOT NULL DEFAULT '{}',
            source          TEXT NOT NULL DEFAULT 'lista',
            updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS assemblies (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL,
            created_at      TEXT NOT NULL,
            updated_at      TEXT NOT NULL,
            confirmed_at    TEXT,
            quote_type      TEXT NOT NULL DEFAULT 'personalizado',
            in_fabrication  INTEGER NOT NULL DEFAULT 0,
            quote_number    TEXT,
            slots           TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE IF NOT EXISTS sale_notes (
            number          TEXT PRIMARY KEY,
            quote_number    TEXT,
            assembly_id     TEXT,
            created_at      TEXT NOT NULL,
            lines           TEXT NOT NULL DEFAULT '[]'
        );
        CREATE TABLE IF NOT EXISTS counters (
            scope           TEXT NOT NULL,
            day             TEXT NOT NULL,
            value           INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (scope, day)
        );
        CREATE TABLE IF NOT EXISTS settings (
            key             TEXT PRIMARY KEY,
            value           TEXT NOT NULL
        );
        ",
    )
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    if version < 1 {
        create_v1(conn)?;
    }
    if version < 2 {
        migrate_v2_sale_notes_nullable(conn)?;
    }
    if version < 3 {
        migrate_v3_based_on(conn)?;
    }
    if version < 4 {
        migrate_v4_offers(conn)?;
    }
    if version < 5 {
        migrate_v5_normalized(conn)?;
    }
    if version < 6 {
        migrate_v6_title_source(conn)?;
    }
    conn.pragma_update(None, "user_version", 6)?;
    Ok(())
}

fn migrate_v2_sale_notes_nullable(conn: &Connection) -> rusqlite::Result<()> {
    let not_null: i64 = conn
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('sale_notes') WHERE name = 'assembly_id'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(1);
    if not_null != 0 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE sale_notes_v2 (
                 number          TEXT PRIMARY KEY,
                 quote_number    TEXT,
                 assembly_id     TEXT,
                 created_at      TEXT NOT NULL,
                 lines           TEXT NOT NULL DEFAULT '[]'
             );
             INSERT INTO sale_notes_v2
                 SELECT number, quote_number, assembly_id, created_at, lines FROM sale_notes;
             DROP TABLE sale_notes;
             ALTER TABLE sale_notes_v2 RENAME TO sale_notes;
             COMMIT;",
        )?;
    }
    Ok(())
}

fn migrate_v3_based_on(conn: &Connection) -> rusqlite::Result<()> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('assemblies') WHERE name = 'based_on'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        conn.execute_batch("ALTER TABLE assemblies ADD COLUMN based_on TEXT;")?;
    }
    let _ = conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_assemblies_quote_number_unique
         ON assemblies(quote_number) WHERE quote_number IS NOT NULL;",
    );
    Ok(())
}

fn migrate_v4_offers(conn: &Connection) -> rusqlite::Result<()> {
    let has_offers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='product_offers'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_offers == 0 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE products_v4 (
                 id              TEXT PRIMARY KEY,
                 sku             TEXT NOT NULL UNIQUE,
                 barcode         TEXT,
                 description     TEXT NOT NULL,
                 brand           TEXT NOT NULL,
                 department      TEXT NOT NULL,
                 kind            TEXT NOT NULL,
                 condition       TEXT NOT NULL,
                 specs           TEXT NOT NULL DEFAULT '{}',
                 updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO products_v4 (id, sku, barcode, description, brand,
                 department, kind, condition, specs, updated_at)
                 SELECT id, sku, barcode, description, brand, department,
                     kind, condition, specs, updated_at FROM products;
             CREATE TABLE product_offers (
                 sku                 TEXT NOT NULL,
                 supplier            TEXT NOT NULL,
                 barcode             TEXT,
                 price_amount        TEXT NOT NULL,
                 price_currency      TEXT NOT NULL,
                 on_promotion        INTEGER NOT NULL DEFAULT 0,
                 stock_json          TEXT NOT NULL DEFAULT '{}',
                 stock_ensamble      INTEGER NOT NULL DEFAULT 0,
                 stock_individual    INTEGER NOT NULL DEFAULT 0,
                 cd_only             INTEGER NOT NULL DEFAULT 0,
                 updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
                 PRIMARY KEY (sku, supplier)
             );
             INSERT INTO product_offers (sku, supplier, barcode, price_amount,
                 price_currency, on_promotion, stock_json, stock_ensamble,
                 stock_individual, cd_only)
             SELECT sku,
                 CASE WHEN source = 'manual' THEN 'manual' ELSE 'arroba' END,
                 barcode, price_amount, price_currency, on_promotion, stock,
                 CASE WHEN source = 'manual' THEN 0
                      ELSE COALESCE(json_extract(stock, '$.a_cen'), 0)
                         + COALESCE(json_extract(stock, '$.ave_2'), 0)
                         + COALESCE(json_extract(stock, '$.cedis'), 0) END,
                 CASE WHEN source = 'manual' THEN 0
                      ELSE COALESCE(json_extract(stock, '$.a_cen'), 0)
                         + COALESCE(json_extract(stock, '$.ave_2'), 0)
                         + COALESCE(json_extract(stock, '$.cedis'), 0) END,
                 0
             FROM products;
             DROP TABLE products;
             ALTER TABLE products_v4 RENAME TO products;
             COMMIT;",
        )?;
    }
    Ok(())
}

/// v5: normalización a 3FN/BCNF. Catálogos en SQL; los JSON de slots y
/// líneas de nota se extraen en Rust.
fn migrate_v5_normalized(conn: &Connection) -> rusqlite::Result<()> {
    let has_brands: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='brands'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_brands > 0 {
        return Ok(()); // ya migrado
    }

    conn.execute_batch(
        "BEGIN;
         CREATE TABLE suppliers (id TEXT PRIMARY KEY, name TEXT NOT NULL);
         INSERT INTO suppliers (id, name) VALUES
            ('arroba', 'Arroba (proveedor actual)'),
            ('cva', 'CVA'),
            ('manual', 'Captura manual');

         CREATE TABLE brands (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
         CREATE TABLE departments (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
         INSERT INTO brands (name) SELECT DISTINCT brand FROM products ORDER BY brand;
         INSERT INTO departments (name) SELECT DISTINCT department FROM products ORDER BY department;

         CREATE TABLE warehouses (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
         INSERT INTO warehouses (name) VALUES
            ('a_cen'), ('ave_2'), ('cedis'), ('cmal'), ('gdl'), ('disp'), ('disp_cd');

         CREATE TABLE products_v5 (
             id              TEXT PRIMARY KEY,
             sku             TEXT NOT NULL UNIQUE,
             barcode         TEXT,
             description     TEXT NOT NULL,
             brand_id        INTEGER NOT NULL REFERENCES brands(id),
             department_id   INTEGER NOT NULL REFERENCES departments(id),
             kind            TEXT NOT NULL,
             condition       TEXT NOT NULL,
             specs           TEXT NOT NULL DEFAULT '{}',
             updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO products_v5 (id, sku, barcode, description, brand_id,
             department_id, kind, condition, specs, updated_at)
         SELECT p.id, p.sku, p.barcode, p.description, b.id, d.id,
             p.kind, p.condition, p.specs, p.updated_at
         FROM products p
         JOIN brands b ON b.name = p.brand
         JOIN departments d ON d.name = p.department;

         CREATE TABLE product_offers_v5 (
             sku                 TEXT NOT NULL REFERENCES products_v5(sku) ON DELETE CASCADE,
             supplier            TEXT NOT NULL REFERENCES suppliers(id),
             barcode             TEXT,
             price_amount        TEXT NOT NULL,
             price_currency      TEXT NOT NULL,
             on_promotion        INTEGER NOT NULL DEFAULT 0,
             cd_only             INTEGER NOT NULL DEFAULT 0,
             updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
             PRIMARY KEY (sku, supplier)
         );
         INSERT INTO product_offers_v5 (sku, supplier, barcode, price_amount,
             price_currency, on_promotion, cd_only)
         SELECT sku, supplier, barcode, price_amount, price_currency, on_promotion, cd_only
         FROM product_offers;

         CREATE TABLE offer_stock (
             sku             TEXT NOT NULL,
             supplier        TEXT NOT NULL,
             warehouse_id    INTEGER NOT NULL REFERENCES warehouses(id),
             quantity        INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (sku, supplier, warehouse_id),
             FOREIGN KEY (sku, supplier) REFERENCES product_offers_v5(sku, supplier) ON DELETE CASCADE
         );
         INSERT INTO offer_stock (sku, supplier, warehouse_id, quantity)
         SELECT o.sku, o.supplier, w.id, COALESCE(json_extract(o.stock_json, '$.' || w.name), 0)
         FROM product_offers o
         JOIN warehouses w ON w.name IN ('a_cen', 'ave_2', 'cedis', 'cmal', 'gdl')
         WHERE o.supplier IN ('arroba', 'manual')
           AND COALESCE(json_extract(o.stock_json, '$.' || w.name), 0) > 0;
         INSERT INTO offer_stock (sku, supplier, warehouse_id, quantity)
         SELECT sku, 'cva', (SELECT id FROM warehouses WHERE name = 'disp'), stock_ensamble
         FROM product_offers WHERE supplier = 'cva' AND stock_ensamble > 0;
         INSERT INTO offer_stock (sku, supplier, warehouse_id, quantity)
         SELECT sku, 'cva', (SELECT id FROM warehouses WHERE name = 'disp_cd'),
             stock_individual - stock_ensamble
         FROM product_offers WHERE supplier = 'cva' AND stock_individual > stock_ensamble;

         CREATE TABLE assemblies_v5 (
             id              TEXT PRIMARY KEY,
             name            TEXT NOT NULL,
             created_at      TEXT NOT NULL,
             updated_at      TEXT NOT NULL,
             confirmed_at    TEXT,
             quote_type      TEXT NOT NULL DEFAULT 'personalizado',
             in_fabrication  INTEGER NOT NULL DEFAULT 0,
             quote_number    TEXT,
             based_on        TEXT
         );
         INSERT INTO assemblies_v5 SELECT id, name, created_at, updated_at,
             confirmed_at, quote_type, in_fabrication, quote_number, based_on
         FROM assemblies;
         CREATE TABLE assembly_items (
             assembly_id     TEXT NOT NULL,
             slot            TEXT NOT NULL,
             component_id    TEXT NOT NULL,
             quantity        INTEGER NOT NULL DEFAULT 1,
             supplier        TEXT,
             PRIMARY KEY (assembly_id, slot, component_id)
         );
         CREATE TABLE sale_note_lines (
             note_number     TEXT NOT NULL,
             line_no         INTEGER NOT NULL,
             component_id    TEXT,
             sku             TEXT NOT NULL,
             description     TEXT NOT NULL,
             kind            TEXT NOT NULL,
             serial          TEXT NOT NULL DEFAULT '',
             price_amount    TEXT NOT NULL,
             PRIMARY KEY (note_number, line_no)
         );
         COMMIT;",
    )?;

    // Partidas de ensamble desde el JSON de slots (estructuras anidadas).
    {
        let mut stmt = conn.prepare("SELECT id, slots FROM assemblies")?;
        let pairs: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for (id, slots_json) in pairs {
            let slots: BTreeMap<String, Vec<serde_json::Value>> =
                match serde_json::from_str(&slots_json) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
            for (slot, items) in slots {
                for item in items {
                    let component_id = match item.get("component_id").and_then(|v| v.as_str()) {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let quantity = item.get("quantity").and_then(|v| v.as_i64()).unwrap_or(1);
                    let supplier = item.get("supplier").and_then(|v| v.as_str());
                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO assembly_items
                         (assembly_id, slot, component_id, quantity, supplier)
                         VALUES (?1,?2,?3,?4,?5)",
                        params![id, slot, component_id, quantity, supplier],
                    );
                }
            }
        }
    }
    // Renglones de nota de venta desde el JSON (snapshot congelado).
    {
        let mut stmt = conn.prepare("SELECT number, lines FROM sale_notes")?;
        let pairs: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for (number, lines_json) in pairs {
            let lines: Vec<serde_json::Value> = match serde_json::from_str(&lines_json) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for (i, line) in lines.iter().enumerate() {
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO sale_note_lines
                     (note_number, line_no, component_id, sku, description, kind, serial, price_amount)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        number,
                        i as i64,
                        line.get("component_id").and_then(|v| v.as_str()),
                        line.get("sku").and_then(|v| v.as_str()).unwrap_or(""),
                        line.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                        line.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                        line.get("serial").and_then(|v| v.as_str()).unwrap_or(""),
                        line.get("price").and_then(|v| v.as_str()).unwrap_or("0"),
                    ],
                );
            }
        }
    }

    // Swap final: tablas normalizadas toman el nombre oficial.
    conn.execute_batch(
        "BEGIN;
         DROP TABLE product_offers;
         DROP TABLE products;
         ALTER TABLE products_v5 RENAME TO products;
         ALTER TABLE product_offers_v5 RENAME TO product_offers;
         DROP TABLE assemblies;
         ALTER TABLE assemblies_v5 RENAME TO assemblies;
         ALTER TABLE sale_notes RENAME TO sale_notes_old;
         CREATE TABLE sale_notes (
             number          TEXT PRIMARY KEY,
             quote_number    TEXT,
             assembly_id     TEXT,
             created_at      TEXT NOT NULL
         );
         INSERT INTO sale_notes (number, quote_number, assembly_id, created_at)
             SELECT number, quote_number, assembly_id, created_at FROM sale_notes_old;
         DROP TABLE sale_notes_old;
         COMMIT;",
    )?;
    Ok(())
}

/// v6: marcado de títulos normalizados con IA. Los productos nuevos nacen
/// 'raw'; el flujo de IA los marca 'ai', y upsert_master los devuelve a
/// 'raw' si el proveedor cambia la descripción después de normalizar.
fn migrate_v6_title_source(conn: &Connection) -> rusqlite::Result<()> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('products') WHERE name = 'title_source'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        conn.execute_batch(
            "ALTER TABLE products ADD COLUMN title_source TEXT NOT NULL DEFAULT 'raw';",
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Catálogos (marcas, departamentos, proveedores, almacenes)
// ---------------------------------------------------------------------

fn catalog_id(conn: &Connection, table: &str, name: &str) -> rusqlite::Result<i64> {
    conn.execute(
        &format!("INSERT OR IGNORE INTO {table} (name) VALUES (?1)"),
        params![name],
    )?;
    conn.query_row(
        &format!("SELECT id FROM {table} WHERE name = ?1"),
        params![name],
        |r| r.get(0),
    )
}

/// Lista de marcas para filtros de la interfaz.
pub fn list_brands(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM brands ORDER BY name")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// Lista de departamentos para filtros de la interfaz.
pub fn list_departments(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM departments ORDER BY name")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

// ---------------------------------------------------------------------
// Productos: maestro + ofertas
// ---------------------------------------------------------------------

/// Inserta o actualiza el MAESTRO de un SKU (datos intrínsecos). La marca y
/// el departamento se resuelven contra sus tablas catálogo. El `id` se
/// conserva si el SKU ya existía. `update_details=false` respeta la
/// descripción/marca existente (importaciones de otros proveedores). Si el
/// proveedor cambia la descripción de un título ya normalizado con IA, el
/// marcado vuelve a 'raw' para que la IA lo revise de nuevo.
pub fn upsert_master(conn: &Connection, c: &Component, update_details: bool) -> rusqlite::Result<()> {
    let brand_id = catalog_id(conn, "brands", &c.brand)?;
    let dept_id = catalog_id(conn, "departments", &c.department)?;
    let specs = serde_json::to_string(&c.specs).unwrap_or_else(|_| "{}".into());
    conn.execute(
        "INSERT INTO products (id, sku, barcode, description, brand_id, department_id,
             kind, condition, specs, title_source, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'raw', datetime('now'))
         ON CONFLICT(sku) DO UPDATE SET
            barcode        = COALESCE(excluded.barcode, products.barcode),
            description    = CASE WHEN ?10 THEN excluded.description ELSE products.description END,
            brand_id       = CASE WHEN ?10 THEN excluded.brand_id ELSE products.brand_id END,
            department_id  = CASE WHEN ?10 THEN excluded.department_id ELSE products.department_id END,
            kind           = CASE WHEN ?10 THEN excluded.kind ELSE products.kind END,
            specs          = CASE WHEN ?10 AND excluded.specs != '{}' THEN excluded.specs ELSE products.specs END,
            title_source   = CASE WHEN ?10 AND excluded.description != products.description
                                  THEN 'raw' ELSE products.title_source END,
            updated_at     = datetime('now')",
        params![
            c.id.to_string(),
            c.sku,
            c.barcode,
            c.description,
            brand_id,
            dept_id,
            serde_json::to_value(c.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            serde_json::to_value(c.condition).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            specs,
            update_details,
        ],
    )?;
    Ok(())
}

/// Inserta o actualiza la OFERTA de un proveedor para un SKU, con su
/// desglose de existencias por almacén (3FN). Clave de conflicto:
/// (sku, supplier).
pub fn upsert_offer(conn: &Connection, c: &Component) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO product_offers (sku, supplier, barcode, price_amount, price_currency,
            on_promotion, cd_only, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7, datetime('now'))
         ON CONFLICT(sku, supplier) DO UPDATE SET
            barcode         = excluded.barcode,
            price_amount    = excluded.price_amount,
            price_currency  = excluded.price_currency,
            on_promotion    = excluded.on_promotion,
            cd_only         = excluded.cd_only,
            updated_at      = datetime('now')",
        params![
            c.sku,
            c.supplier,
            c.barcode,
            c.price.amount.to_string(),
            serde_json::to_value(c.price.currency).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            c.on_promotion as i64,
            c.cd_only_best as i64,
        ],
    )?;
    conn.execute(
        "DELETE FROM offer_stock WHERE sku = ?1 AND supplier = ?2",
        params![c.sku, c.supplier],
    )?;
    for (warehouse, qty) in &c.offer_stocks {
        if *qty == 0 {
            continue;
        }
        let wid: i64 = catalog_id(conn, "warehouses", warehouse)?;
        conn.execute(
            "INSERT INTO offer_stock (sku, supplier, warehouse_id, quantity)
             VALUES (?1,?2,?3,?4)",
            params![c.sku, c.supplier, wid, *qty as i64],
        )?;
    }
    Ok(())
}

pub fn delete_product(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    let sku: Option<String> = conn
        .query_row("SELECT sku FROM products WHERE id = ?1", params![id], |r| r.get(0))
        .ok();
    let mut n = conn.execute("DELETE FROM products WHERE id = ?1", params![id])?;
    if let Some(sku) = sku {
        n += conn.execute("DELETE FROM offer_stock WHERE sku = ?1", params![sku])?;
        n += conn.execute("DELETE FROM product_offers WHERE sku = ?1", params![sku])?;
    }
    Ok(n)
}

/// Carga el catálogo completo: maestro + ofertas (con existencias por
/// almacén) agrupados por SKU. Cada `Component` representa la MEJOR oferta
/// (menor precio con disponibilidad individual) y trae el desglose.
pub fn load_products(conn: &Connection) -> rusqlite::Result<Vec<Component>> {
    struct MasterRow {
        id: uuid::Uuid,
        barcode: Option<String>,
        description: String,
        brand: String,
        department: String,
        kind: ComponentKind,
        condition: Condition,
        specs: Specs,
    }
    struct OfferRow {
        supplier: String,
        barcode: Option<String>,
        price: Money,
        on_promotion: bool,
        stocks: BTreeMap<String, u32>,
        stock_ensamble: u32,
        stock_individual: u32,
        cd_only: bool,
    }

    let mut masters: BTreeMap<String, MasterRow> = BTreeMap::new();
    let mut offers_by_sku: BTreeMap<String, Vec<OfferRow>> = BTreeMap::new();

    let mut stmt = conn.prepare(
        "SELECT p.id, p.sku, p.barcode, p.description,
                b.name AS brand_name, d.name AS department_name,
                p.kind, p.condition, p.specs
         FROM products p
         JOIN brands b ON b.id = p.brand_id
         JOIN departments d ON d.id = p.department_id
         ORDER BY p.sku",
    )?;
    let rows = stmt.query_map([], |row| {
        let sku: String = row.get("sku")?;
        let kind_s: String = row.get("kind")?;
        let condition_s: String = row.get("condition")?;
        let specs_s: String = row.get("specs")?;
        masters.insert(
            sku,
            MasterRow {
                id: uuid::Uuid::parse_str(&row.get::<_, String>("id")?).unwrap_or_default(),
                barcode: row.get("barcode")?,
                description: row.get("description")?,
                brand: row.get("brand_name")?,
                department: row.get("department_name")?,
                kind: serde_json::from_value(serde_json::Value::String(kind_s))
                    .unwrap_or(ComponentKind::Other),
                condition: serde_json::from_value(serde_json::Value::String(condition_s))
                    .unwrap_or(Condition::Nuevo),
                specs: serde_json::from_str(&specs_s).unwrap_or_default(),
            },
        );
        Ok(())
    })?;
    rows.collect::<rusqlite::Result<()>>()?;
    drop(stmt);

    let mut stmt = conn.prepare(
        "SELECT o.sku, o.supplier, o.barcode, o.price_amount, o.price_currency,
                o.on_promotion, o.cd_only, w.name, os.quantity
         FROM product_offers o
         LEFT JOIN offer_stock os ON os.sku = o.sku AND os.supplier = o.supplier
         LEFT JOIN warehouses w ON w.id = os.warehouse_id
         ORDER BY o.sku, o.price_amount",
    )?;
    let rows = stmt.query_map([], |row| {
        let sku: String = row.get("sku")?;
        let currency_s: String = row.get("price_currency")?;
        let entry = offers_by_sku.entry(sku).or_default();
        let supplier: String = row.get("supplier")?;
        if entry
            .last()
            .map(|last| last.supplier != supplier)
            .unwrap_or(true)
        {
            entry.push(OfferRow {
                supplier: supplier.clone(),
                barcode: row.get("barcode")?,
                price: Money::new(
                    rust_decimal::Decimal::from_str_exact(&row.get::<_, String>("price_amount")?)
                        .unwrap_or_default(),
                    serde_json::from_value(serde_json::Value::String(currency_s))
                        .unwrap_or(Currency::Mxp),
                ),
                on_promotion: row.get::<_, i64>("on_promotion")? != 0,
                stocks: BTreeMap::new(),
                stock_ensamble: 0,
                stock_individual: 0,
                cd_only: row.get::<_, i64>("cd_only")? != 0,
            });
        }
        let warehouse: Option<String> = row.get("name")?;
        let qty: Option<i64> = row.get("quantity")?;
        if let (Some(warehouse), Some(qty)) = (warehouse, qty) {
            if let Some(last) = entry.last_mut() {
                last.stocks.insert(warehouse, qty.max(0) as u32);
            }
        }
        Ok(())
    })?;
    rows.collect::<rusqlite::Result<()>>()?;
    drop(stmt);

    let mut out = Vec::new();
    for (sku, master) in masters {
        let mut offers = offers_by_sku.remove(&sku).unwrap_or_default();
        for o in &mut offers {
            // Reglas por proveedor sobre el desglose de almacenes
            match o.supplier.as_str() {
                "cva" => {
                    o.stock_ensamble = *o.stocks.get("disp").unwrap_or(&0);
                    o.stock_individual =
                        o.stock_ensamble + o.stocks.get("disp_cd").copied().unwrap_or(0);
                }
                "arroba" => {
                    let s: u32 = ["a_cen", "ave_2", "cedis"]
                        .iter()
                        .filter_map(|w| o.stocks.get(*w).copied())
                        .sum();
                    o.stock_ensamble = s;
                    o.stock_individual = s;
                }
                _ => {
                    o.stock_ensamble = 0;
                    o.stock_individual = 0;
                }
            }
        }

        let best = offers
            .iter()
            .filter(|o| o.stock_individual > 0)
            .min_by_key(|o| (o.price.amount, o.supplier.clone()))
            .or_else(|| offers.iter().min_by_key(|o| (o.price.amount, o.supplier.clone())));

        let (price, on_promotion, supplier, cd_only_best, offer_stocks) = match best {
            Some(o) => (
                o.price.clone(),
                o.on_promotion,
                o.supplier.clone(),
                o.cd_only && o.stock_individual > 0,
                o.stocks.clone(),
            ),
            None => (
                Money::mxp(rust_decimal::Decimal::ZERO),
                false,
                String::new(),
                false,
                BTreeMap::new(),
            ),
        };

        out.push(Component {
            id: master.id,
            sku,
            barcode: master.barcode.or_else(|| best.and_then(|o| o.barcode.clone())),
            description: master.description,
            brand: master.brand,
            department: master.department,
            kind: master.kind,
            condition: master.condition,
            price,
            on_promotion,
            offer_stocks,
            source: if offers.len() == 1 && offers[0].supplier == "manual" {
                ComponentSource::Manual
            } else {
                ComponentSource::Lista
            },
            specs: master.specs,
            supplier,
            stock_ensamble_total: offers.iter().map(|o| o.stock_ensamble).sum(),
            stock_individual_total: offers.iter().map(|o| o.stock_individual).sum(),
            cd_only_best,
            offers: offers
                .iter()
                .map(|o| Offer {
                    supplier: o.supplier.clone(),
                    price: o.price.clone(),
                    on_promotion: o.on_promotion,
                    stock_label: stock_label(&o.supplier, &o.stocks),
                    stocks: o.stocks.clone(),
                    stock_ensamble: o.stock_ensamble,
                    stock_individual: o.stock_individual,
                    cd_only: o.cd_only,
                })
                .collect(),
        });
    }
    Ok(out)
}

/// Etiqueta legible del desglose de una oferta, según sus almacenes.
fn stock_label(supplier: &str, stocks: &BTreeMap<String, u32>) -> String {
    match supplier {
        "cva" => format!(
            "Disp {} · Disp CD {}",
            stocks.get("disp").copied().unwrap_or(0),
            stocks.get("disp_cd").copied().unwrap_or(0)
        ),
        "arroba" => format!(
            "CEN {} · AVE-2 {} · CEDIS {}",
            stocks.get("a_cen").copied().unwrap_or(0),
            stocks.get("ave_2").copied().unwrap_or(0),
            stocks.get("cedis").copied().unwrap_or(0)
        ),
        _ => String::new(),
    }
}

/// Consulta principal por SKU (único).
pub fn find_by_sku(conn: &Connection, sku: &str) -> rusqlite::Result<Option<Component>> {
    Ok(load_products(conn)?.into_iter().find(|c| c.sku == sku))
}

pub fn count_products(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM products", [], |r| r.get(0))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Títulos: candidatos para normalización con IA
// ---------------------------------------------------------------------

/// Productos pendientes de normalización con IA ('raw'), en orden de SKU.
pub fn list_raw_title_candidates(
    conn: &Connection,
    limit: u32,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT sku, description FROM products
         WHERE title_source = 'raw'
         ORDER BY sku LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    rows.collect()
}

/// Marca el estado del título de un SKU: 'raw' | 'ai'.
pub fn set_title_source(conn: &Connection, sku: &str, source: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE products SET title_source = ?2, updated_at = datetime('now') WHERE sku = ?1",
        params![sku, source],
    )?;
    Ok(())
}

pub fn count_raw_title_candidates(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM products WHERE title_source = 'raw'",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Ensamles + partidas
// ---------------------------------------------------------------------

pub fn upsert_assembly(conn: &Connection, a: &Assembly) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO assemblies (id, name, created_at, updated_at, confirmed_at,
            quote_type, in_fabrication, quote_number, based_on)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            updated_at = excluded.updated_at,
            confirmed_at = excluded.confirmed_at,
            quote_type = excluded.quote_type,
            in_fabrication = excluded.in_fabrication,
            quote_number = excluded.quote_number,
            based_on = excluded.based_on",
        params![
            a.id.to_string(),
            a.name,
            a.created_at.to_rfc3339(),
            a.updated_at.to_rfc3339(),
            a.confirmed_at.map(|d| d.to_rfc3339()),
            serde_json::to_value(a.quote_type).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            a.in_fabrication as i64,
            a.quote_number,
            a.based_on,
        ],
    )?;
    conn.execute(
        "DELETE FROM assembly_items WHERE assembly_id = ?1",
        params![a.id.to_string()],
    )?;
    for (slot, items) in &a.slots {
        for item in items {
            conn.execute(
                "INSERT INTO assembly_items (assembly_id, slot, component_id, quantity, supplier)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    a.id.to_string(),
                    serde_json::to_value(slot).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                    item.component_id.to_string(),
                    item.quantity as i64,
                    item.supplier,
                ],
            )?;
        }
    }
    Ok(())
}

pub fn delete_assembly(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM assemblies WHERE id = ?1", params![id])
}

pub fn load_assemblies(conn: &Connection) -> rusqlite::Result<Vec<Assembly>> {
    let mut assemblies = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, name, created_at, updated_at, confirmed_at, quote_type,
                in_fabrication, quote_number, based_on
         FROM assemblies ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let created: String = row.get("created_at")?;
        let updated: String = row.get("updated_at")?;
        let confirmed: Option<String> = row.get("confirmed_at")?;
        let quote_type: String = row.get("quote_type")?;
        Ok((
            row.get::<_, String>("id")?,
            Assembly {
                id: uuid::Uuid::parse_str(&row.get::<_, String>("id")?).unwrap_or_default(),
                name: row.get("name")?,
                created_at: chrono::DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                confirmed_at: confirmed
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|d| d.with_timezone(&chrono::Utc)),
                quote_type: serde_json::from_value(serde_json::Value::String(quote_type))
                    .unwrap_or(crate::domain::assembly::QuoteType::Personalizado),
                in_fabrication: row.get::<_, i64>("in_fabrication")? != 0,
                quote_number: row.get("quote_number")?,
                based_on: row.get("based_on")?,
                slots: BTreeMap::new(),
            },
        ))
    })?;
    for pair in rows {
        let (id, asm) = pair?;
        assemblies.insert(id, asm);
    }
    drop(stmt);

    let mut stmt = conn.prepare(
        "SELECT assembly_id, slot, component_id, quantity, supplier
         FROM assembly_items",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (assembly_id, slot_s, component_id, quantity, supplier) = row?;
        if let Some(asm) = assemblies.get_mut(&assembly_id) {
            let slot: Slot = serde_json::from_value(serde_json::Value::String(slot_s))
                .unwrap_or(Slot::Storage);
            let item = AssemblyItem {
                component_id: uuid::Uuid::parse_str(&component_id).unwrap_or_default(),
                quantity: quantity.max(0) as u32,
                supplier,
            };
            asm.slots.entry(slot).or_default().push(item);
        }
    }
    drop(stmt);

    Ok(assemblies.into_values().collect())
}

// ---------------------------------------------------------------------
// Notas de venta
// ---------------------------------------------------------------------

pub fn insert_sale_note(conn: &Connection, n: &SaleNoteRecord) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sale_notes (number, quote_number, assembly_id, created_at)
         VALUES (?1,?2,?3,?4)",
        params![
            n.number,
            n.quote_number,
            n.assembly_id.map(|id| id.to_string()),
            n.created_at.to_rfc3339(),
        ],
    )?;
    conn.execute(
        "DELETE FROM sale_note_lines WHERE note_number = ?1",
        params![n.number],
    )?;
    for (i, line) in n.lines.iter().enumerate() {
        conn.execute(
            "INSERT INTO sale_note_lines (note_number, line_no, component_id, sku,
                 description, kind, serial, price_amount)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                n.number,
                i as i64,
                line.component_id.to_string(),
                line.sku,
                line.description,
                line.kind,
                line.serial,
                line.price.to_string(),
            ],
        )?;
    }
    Ok(())
}

pub fn load_sale_notes(conn: &Connection) -> rusqlite::Result<Vec<SaleNoteRecord>> {
    let mut notes = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT number, quote_number, assembly_id, created_at
         FROM sale_notes ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let created: String = row.get("created_at")?;
        let assembly_id: Option<String> = row.get("assembly_id")?;
        Ok((
            row.get::<_, String>("number")?,
            SaleNoteRecord {
                number: row.get("number")?,
                quote_number: row.get("quote_number")?,
                assembly_id: assembly_id.and_then(|s| uuid::Uuid::parse_str(&s).ok()),
                created_at: chrono::DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                lines: Vec::new(),
            },
        ))
    })?;
    for pair in rows {
        let (number, note) = pair?;
        notes.insert(number, note);
    }
    drop(stmt);

    let mut stmt = conn.prepare(
        "SELECT note_number, line_no, component_id, sku, description, kind, serial, price_amount
         FROM sale_note_lines ORDER BY note_number, line_no",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>("note_number")?,
            crate::domain::sale::SaleNoteLine {
                component_id: row
                    .get::<_, Option<String>>("component_id")?
                    .and_then(|s| uuid::Uuid::parse_str(&s).ok())
                    .unwrap_or_default(),
                sku: row.get("sku")?,
                description: row.get("description")?,
                kind: row.get("kind")?,
                serial: row.get("serial")?,
                price: rust_decimal::Decimal::from_str_exact(
                    &row.get::<_, String>("price_amount")?,
                )
                .unwrap_or_default(),
                line_no: row.get::<_, i64>("line_no")? as u32,
            },
        ))
    })?;
    for row in rows {
        let (number, line) = row?;
        if let Some(note) = notes.get_mut(&number) {
            note.lines.push(line);
        }
    }

    Ok(notes.into_values().collect())
}

// ---------------------------------------------------------------------
// Contadores y ajustes
// ---------------------------------------------------------------------

pub fn set_counter(conn: &Connection, scope: &str, day: &str, value: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO counters (scope, day, value) VALUES (?1,?2,?3)
         ON CONFLICT(scope, day) DO UPDATE SET value = excluded.value",
        params![scope, day, value],
    )?;
    Ok(())
}

pub fn load_counters(conn: &Connection, scope: &str) -> rusqlite::Result<BTreeMap<String, u32>> {
    let mut stmt = conn.prepare("SELECT day, value FROM counters WHERE scope = ?1")?;
    let rows = stmt.query_map(params![scope], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for r in rows {
        let (day, value) = r?;
        map.insert(day, value);
    }
    Ok(map)
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| {
        r.get(0)
    })
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1,?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Currency;
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("genpresu-db-{}", Uuid::new_v4()));
        let c = open(&dir).unwrap();
        let _ = std::thread::spawn(move || drop(dir));
        c
    }

    fn product(sku: &str, price: &str, desc: &str) -> Component {
        Component {
            id: Uuid::new_v4(),
            sku: sku.into(),
            barcode: None,
            description: desc.into(),
            brand: "TEST".into(),
            department: "TEST".into(),
            kind: ComponentKind::Psu,
            condition: Condition::Nuevo,
            price: Money::new(
                rust_decimal::Decimal::from_str_exact(price).unwrap(),
                Currency::Usd,
            ),
            on_promotion: false,
            offer_stocks: BTreeMap::new(),
            source: ComponentSource::Lista,
            specs: Specs { wattage: Some(650), ..Default::default() },
            supplier: "arroba".into(),
            stock_ensamble_total: 0,
            stock_individual_total: 0,
            cd_only_best: false,
            offers: vec![],
        }
    }

    #[test]
    fn upsert_by_sku_updates_fields_and_keeps_id() {
        let c = conn();
        let mut original = product("SKU-1", "54.00", "Fuente original");
        original.offer_stocks.insert("a_cen".into(), 5);
        original.stock_ensamble_total = 5;
        original.stock_individual_total = 5;
        upsert_master(&c, &original, true).unwrap();
        upsert_offer(&c, &original).unwrap();

        // Mismo SKU, otros datos (p. ej. lista de precios nueva)
        let mut updated = original.clone();
        updated.description = "Fuente 2027".into();
        updated.price = Money::new(dec!(49.99), Currency::Usd);
        upsert_master(&c, &updated, true).unwrap();
        upsert_offer(&c, &updated).unwrap();

        assert_eq!(count_products(&c), 1, "mismo SKU = una sola fila");
        let found = find_by_sku(&c, "SKU-1").unwrap().expect("debe existir");
        assert_eq!(found.description, "Fuente 2027");
        assert_eq!(found.price.amount, dec!(49.99));
        // El id se conserva para no romper referencias de ensambles
        assert_eq!(found.id, original.id);
        // La oferta trae su desglose de almacén
        assert_eq!(found.offer_stocks.get("a_cen"), Some(&5));
    }

    #[test]
    fn new_sku_inserts_and_finds() {
        let c = conn();
        for (sku, price, desc) in [("AAA-1", "10.00", "Uno"), ("BBB-2", "20.00", "Dos")] {
            let mut p = product(sku, price, desc);
            p.offer_stocks.insert("a_cen".into(), 1);
            p.stock_ensamble_total = 1;
            p.stock_individual_total = 1;
            upsert_master(&c, &p, true).unwrap();
            upsert_offer(&c, &p).unwrap();
        }
        assert_eq!(count_products(&c), 2);
        let found = find_by_sku(&c, "BBB-2").unwrap().unwrap();
        assert_eq!(found.description, "Dos");
        assert!(find_by_sku(&c, "ZZZ-9").unwrap().is_none());
    }

    #[test]
    fn same_sku_two_suppliers_best_price_wins() {
        let c = conn();
        let mut arroba = product("SKU-DUP", "100.00", "Fuente compartida");
        arroba.supplier = "arroba".into();
        arroba.offer_stocks.insert("a_cen".into(), 5);
        arroba.stock_ensamble_total = 5;
        arroba.stock_individual_total = 5;
        upsert_master(&c, &arroba, true).unwrap();
        upsert_offer(&c, &arroba).unwrap();
        let mut cva = product("SKU-DUP", "80.00", "Fuente compartida");
        cva.supplier = "cva".into();
        cva.price = Money::new(dec!(80.00), Currency::Usd);
        cva.offer_stocks.insert("disp".into(), 3);
        cva.offer_stocks.insert("disp_cd".into(), 6);
        cva.stock_ensamble_total = 3;
        cva.stock_individual_total = 9;
        upsert_master(&c, &cva, false).unwrap();
        upsert_offer(&c, &cva).unwrap();

        let best = find_by_sku(&c, "SKU-DUP").unwrap().expect("existe");
        assert_eq!(best.price.amount, dec!(80.00), "gana el menor precio");
        assert_eq!(best.supplier, "cva", "indicando su procedencia");
        assert_eq!(best.offers.len(), 2, "desglose de ambas ofertas");
        // Totales por flujo suman ambos proveedores
        assert_eq!(best.stock_ensamble_total, 8);
        assert_eq!(best.stock_individual_total, 14);
        // Etiquetas de inventario por proveedor
        let cva_offer = best.offers.iter().find(|o| o.supplier == "cva").unwrap();
        assert!(cva_offer.stock_label.contains("Disp 3"));
    }

    #[test]
    fn title_source_lifecycle() {
        let c = conn();
        let mut p = product("SKU-AI", "10.00", "TITULO CRUDO DEL PROVEEDOR");
        p.offer_stocks.insert("a_cen".into(), 1);
        p.stock_ensamble_total = 1;
        p.stock_individual_total = 1;
        upsert_master(&c, &p, true).unwrap();
        upsert_offer(&c, &p).unwrap();

        // Nace como candidato 'raw'
        let cand = list_raw_title_candidates(&c, 10).unwrap();
        assert_eq!(cand.len(), 1);
        assert_eq!(cand[0].0, "SKU-AI");

        // La IA lo procesa → se marca 'ai' y sale de los candidatos
        set_title_source(&c, "SKU-AI", "ai").unwrap();
        assert!(list_raw_title_candidates(&c, 10).unwrap().is_empty());
        assert_eq!(count_raw_title_candidates(&c), 0);

        // Reimportar el mismo SKU sin cambio de descripción NO lo devuelve a raw
        upsert_master(&c, &p, true).unwrap();
        assert!(list_raw_title_candidates(&c, 10).unwrap().is_empty());

        // Si el proveedor CAMBIA la descripción → vuelve a 'raw'
        let mut changed = p.clone();
        changed.description = "TITULO NUEVO DEL PROVEEDOR 2027".into();
        upsert_master(&c, &changed, true).unwrap();
        let cand = list_raw_title_candidates(&c, 10).unwrap();
        assert_eq!(cand.len(), 1, "descripción nueva = candidato de nuevo");
        assert_eq!(cand[0].1, "TITULO NUEVO DEL PROVEEDOR 2027");
    }

    #[test]
    fn assemblies_roundtrip_with_slots() {
        use crate::domain::assembly::{Assembly, QuoteType, Slot};
        let c = conn();
        let mut asm = Assembly::new("Equipo", QuoteType::Preensamble);
        let p = product("SKU-1", "54.00", "Fuente");
        asm.set_quantity(Slot::Psu, &p, 2, None).unwrap();
        asm.confirmed_at = Some(chrono::Utc::now());
        asm.quote_number = Some("161026-1".into());
        upsert_assembly(&c, &asm).unwrap();

        let loaded = load_assemblies(&c).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].quote_number.as_deref(), Some("161026-1"));
        assert_eq!(loaded[0].quote_type, QuoteType::Preensamble);
        assert_eq!(loaded[0].total_quantity(Slot::Psu), 2);
        assert!(loaded[0].confirmed_at.is_some());
    }

    #[test]
    fn counters_and_settings_roundtrip() {
        let c = conn();
        set_counter(&c, "quote", "2026-10-16", 3).unwrap();
        set_counter(&c, "sale_note", "2026-10-16", 1).unwrap();
        assert_eq!(load_counters(&c, "quote").unwrap()["2026-10-16"], 3);
        set_setting(&c, "list_rate", "18.52").unwrap();
        assert_eq!(get_setting(&c, "list_rate").as_deref(), Some("18.52"));
    }
}
