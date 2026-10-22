//! Estado de la aplicación con persistencia SQLite (esquema v5, 3FN/BCNF).
//!
//! La base (`presupuestos.db`) es la fuente de verdad: productos maestros +
//! ofertas por proveedor + existencias por almacén, ensambles con partidas
//! normalizadas, notas de venta con renglones, contadores de folios y
//! ajustes. En memoria viven el catálogo y los ensambles para lecturas
//! rápidas; cada operación escribe de inmediato.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::catalog::{Catalog, NewComponentInput, UpdateComponentInput};
use crate::db;
use crate::domain::assembly::{self, Assembly, ComponentLookup, QuoteType, Slot};
use crate::domain::component::{Component, ComponentKind};
use crate::domain::money::{ExchangeRate, Money};

/// Resultado de importar un Excel de proveedor.
#[derive(Debug, Clone, Serialize)]
pub struct XlsxImportResult {
    pub updated: usize,
    pub inserted: usize,
    /// SKUs del proveedor que ya no venían en la lista y quedaron agotados.
    pub zeroed: usize,
    pub supplier: String,
    pub out_of_stock: Vec<OutOfStockHit>,
}

/// Componente de un ensamble vigente cuya existencia disponible quedó por
/// debajo de lo pedido tras la importación.
#[derive(Debug, Clone, Serialize)]
pub struct OutOfStockHit {
    pub component_id: Uuid,
    pub sku: String,
    pub description: String,
    pub kind: ComponentKind,
    /// Proveedor fijado en el renglón (None = mejor oferta).
    pub supplier: Option<String>,
    pub stock_after: u32,
    /// `true` si ANTES de importar sí alcanzaba: se acaba de agotar.
    pub newly_out: bool,
    pub assemblies: Vec<AffectedAssembly>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AffectedAssembly {
    pub id: Uuid,
    pub name: String,
    pub quote_type: QuoteType,
    pub quote_number: Option<String>,
    pub quantity: u32,
}

/// Renglón de un ensamble vigente con su disponibilidad ANTES de importar.
struct LiveItem {
    component_id: Uuid,
    supplier: Option<String>,
    quantity: u32,
    available_before: u32,
    assembly: AffectedAssembly,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyState {
    #[serde(default)]
    exchange_rate_override: Option<String>,
    #[serde(default)]
    manual_components: Vec<Component>,
    #[serde(default)]
    assemblies: Vec<Assembly>,
    #[serde(default)]
    quote_counters: BTreeMap<String, u32>,
    #[serde(default)]
    sale_note_counters: BTreeMap<String, u32>,
    #[serde(default)]
    sale_notes: Vec<crate::domain::sale::SaleNoteRecord>,
}

pub struct AppState {
    db: rusqlite::Connection,
    catalog: Catalog,
    assemblies: HashMap<Uuid, Assembly>,
    list_rate: ExchangeRate,
    rate_override: Option<ExchangeRate>,
    quote_counters: BTreeMap<String, u32>,
    sale_note_counters: BTreeMap<String, u32>,
    sale_notes: Vec<crate::domain::sale::SaleNoteRecord>,
}

impl AppState {
    /// `seed_json` es la semilla opcional (data/catalog_seed.json generada
    /// por el script o la lista inicial). Si no existe, la app arranca con
    /// catálogo vacío y se llena importando un Excel desde Ajustes.
    pub fn load(seed_json: Option<&str>, dir: PathBuf) -> Self {
        let conn = db::open(&dir).expect("no se pudo abrir la base de datos");
        if db::count_products(&conn) == 0 {
            if let Some(json) = seed_json {
                // from_seed_json valida el T.C y normaliza el orden; si la
                // semilla es inválida se arranca con catálogo vacío.
                match Catalog::from_seed_json(json) {
                    Ok(seed) => {
                        for c in &seed.components {
                            let mut comp = c.clone();
                            comp.supplier = "arroba".into();
                            comp.stock_ensamble_total = comp.offer_stocks.values().sum();
                            comp.stock_individual_total = comp.stock_ensamble_total;
                            comp.offers.clear();
                            db::upsert_master(&conn, &comp, true).expect("seed de productos");
                            db::upsert_offer(&conn, &comp).expect("seed de ofertas");
                        }
                        let _ = db::set_setting(&conn, "list_rate", &seed.exchange_rate.rate.to_string());
                        let _ = db::set_setting(&conn, "list_date", &seed.list_date);
                        let _ = db::set_setting(&conn, "source_file", &seed.source_file);
                    }
                    Err(e) => {
                        eprintln!("Semilla del catálogo inválida, se arranca sin catálogo: {e}");
                    }
                }
            }
        }

        let legacy_path = dir.join("state.json");
        if legacy_path.exists() {
            if let Ok(text) = fs::read_to_string(&legacy_path) {
                if let Ok(legacy) = serde_json::from_str::<LegacyState>(&text) {
                    for c in &legacy.manual_components {
                        let mut comp = c.clone();
                        comp.supplier = "manual".into();
                        comp.stock_ensamble_total = 0;
                        comp.stock_individual_total = 0;
                        comp.offers.clear();
                        let _ = db::upsert_master(&conn, &comp, true);
                        let _ = db::upsert_offer(&conn, &comp);
                    }
                    for a in &legacy.assemblies {
                        let _ = db::upsert_assembly(&conn, a);
                    }
                    for n in &legacy.sale_notes {
                        let _ = db::insert_sale_note(&conn, n);
                    }
                    for (day, v) in &legacy.quote_counters {
                        let _ = db::set_counter(&conn, "quote", day, *v as i64);
                    }
                    for (day, v) in &legacy.sale_note_counters {
                        let _ = db::set_counter(&conn, "sale_note", day, *v as i64);
                    }
                    if let Some(rate) = &legacy.exchange_rate_override {
                        let _ = db::set_setting(&conn, "rate_override", rate);
                    }
                }
            }
            let _ = fs::rename(&legacy_path, dir.join("state.json.migrated"));
        }

        let products = db::load_products(&conn).expect("leer productos");
        let seed_rate = db::get_setting(&conn, "list_rate")
            .and_then(|r| rust_decimal::Decimal::from_str_exact(&r).ok())
            .map(ExchangeRate::new)
            .unwrap_or_else(|| ExchangeRate::new(rust_decimal::Decimal::from(19)));
        let list_rate = seed_rate;
        let mut catalog = Catalog::from_parts(
            products,
            seed_rate,
            "USD→MXP: tipo de cambio redondeado hacia arriba al entero superior",
            db::get_setting(&conn, "source_file").unwrap_or_default(),
            db::get_setting(&conn, "list_date").unwrap_or_default(),
        );

        let rate_override = db::get_setting(&conn, "rate_override")
            .and_then(|r| rust_decimal::Decimal::from_str_exact(&r).ok())
            .map(ExchangeRate::new);
        if let Some(rate) = rate_override {
            catalog.set_exchange_rate(rate);
        }

        let mut assemblies = HashMap::new();
        for asm in db::load_assemblies(&conn).expect("leer ensambles") {
            assemblies.insert(asm.id, asm);
        }
        let sale_notes = db::load_sale_notes(&conn).expect("leer notas de venta");
        let quote_counters = db::load_counters(&conn, "quote").unwrap_or_default();
        let sale_note_counters = db::load_counters(&conn, "sale_note").unwrap_or_default();

        Self {
            db: conn,
            catalog,
            assemblies,
            list_rate,
            rate_override,
            quote_counters,
            sale_note_counters,
            sale_notes,
        }
    }

    fn persist_assembly(&self, asm: &Assembly) {
        db::upsert_assembly(&self.db, asm).expect("persistir ensamble");
    }

    fn set_counter_db(&self, scope: &str, day: &str, value: u32) {
        db::set_counter(&self.db, scope, day, value as i64).expect("persistir contador");
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn db(&self) -> &rusqlite::Connection {
        &self.db
    }

    pub fn db_query_zai(&self) -> String {
        crate::db::get_setting(&self.db, "zai_api_key").unwrap_or_default()
    }

    pub fn rate(&self) -> ExchangeRate {
        self.catalog.exchange_rate
    }

    #[allow(dead_code)]
    pub fn list_rate(&self) -> ExchangeRate {
        self.list_rate
    }

    pub fn rate_is_overridden(&self) -> bool {
        self.rate_override.is_some()
    }

    pub fn set_rate(&mut self, rate: ExchangeRate) {
        self.rate_override = Some(rate);
        self.catalog.set_exchange_rate(rate);
        db::set_setting(&self.db, "rate_override", &rate.rate.to_string())
            .expect("persistir T.C");
    }

    pub fn find_by_sku(&self, sku: &str) -> Option<Component> {
        db::find_by_sku(&self.db, sku).ok().flatten()
    }

    pub fn find_component(&self, id: &Uuid) -> Option<Component> {
        self.catalog.get_component(id).cloned()
    }

    pub fn import_catalog(&mut self, json: &str) -> Result<(usize, usize), String> {
        // Misma validación que el arranque (from_seed_json valida el T.C).
        let seed = Catalog::from_seed_json(json)
            .map_err(|e| format!("JSON de catálogo inválido: {e}"))?;
        let (updated, inserted, _) = self.apply_import(
            seed.components,
            Some(seed.exchange_rate.rate),
            seed.list_date,
            seed.source_file,
            None,
        )?;
        Ok((updated, inserted))
    }

    /// Importa el Excel de un proveedor y devuelve, además de los
    /// contadores, el informe de componentes que quedaron sin existencia
    /// suficiente y están en un preensamble o personalizado vigente.
    pub fn import_xlsx_catalog(&mut self, path: &str) -> Result<XlsxImportResult, String> {
        let parsed = crate::suppliers::import_xlsx(Path::new(path))?;
        let supplier = parsed.supplier.clone();
        let components: Vec<Component> = parsed
            .products
            .into_iter()
            .map(|p| p.into_component())
            .collect();
        let before = self.snapshot_live_items();
        // El id del proveedor viaja en cada producto (`parsed.supplier` es
        // el nombre para mostrar). Solo las listas COMPLETAS de arroba y
        // CVA autorizan a dar por agotado lo que ya no viene; la genérica
        // puede ser parcial.
        let supplier_id = components
            .first()
            .map(|c| if c.supplier.is_empty() { "arroba".to_string() } else { c.supplier.clone() })
            .unwrap_or_default();
        let sweep = matches!(supplier_id.as_str(), "arroba" | "cva").then_some(supplier_id.as_str());
        let (updated, inserted, zeroed) = self.apply_import(
            components,
            parsed.exchange_rate,
            parsed.list_date.unwrap_or_default(),
            format!("{} ({})", parsed.source_file, supplier),
            sweep,
        )?;
        let out_of_stock = self.out_of_stock_report(&before);
        Ok(XlsxImportResult {
            updated,
            inserted,
            zeroed,
            supplier,
            out_of_stock,
        })
    }

    /// Existencia disponible por renglón de cada ensamble VIGENTE antes de
    /// importar: (component_id, supplier fijado) → (disponible, [referencias]).
    fn snapshot_live_items(&self) -> Vec<LiveItem> {
        let today = chrono::Local::now().date_naive();
        let mut out = Vec::new();
        for asm in self.assemblies.values() {
            if asm.status(today) == assembly::QuoteStatus::Caducada {
                continue;
            }
            for &slot in Slot::ALL.iter() {
                for item in asm.items(slot) {
                    let available_before = self
                        .catalog
                        .get_component(&item.component_id)
                        .map(|c| c.stock_for_assembly(item.supplier.as_deref()))
                        .unwrap_or(0);
                    out.push(LiveItem {
                        component_id: item.component_id,
                        supplier: item.supplier.clone(),
                        quantity: item.quantity,
                        available_before,
                        assembly: AffectedAssembly {
                            id: asm.id,
                            name: asm.name.clone(),
                            quote_type: asm.quote_type,
                            quote_number: asm.quote_number.clone(),
                            quantity: item.quantity,
                        },
                    });
                }
            }
        }
        out
    }

    /// Compara el snapshot con el catálogo ya recargado: todo renglón cuya
    /// existencia disponible quedó por debajo de lo pedido se reporta,
    /// marcando si ANTES sí alcanzaba (`newly_out`).
    fn out_of_stock_report(&self, before: &[LiveItem]) -> Vec<OutOfStockHit> {
        let mut by_component: BTreeMap<(Uuid, Option<String>), OutOfStockHit> = BTreeMap::new();
        for li in before {
            let comp = self.catalog.get_component(&li.component_id);
            let available_after = comp
                .map(|c| c.stock_for_assembly(li.supplier.as_deref()))
                .unwrap_or(0);
            if available_after >= li.quantity {
                continue;
            }
            let key = (li.component_id, li.supplier.clone());
            let hit = by_component.entry(key).or_insert_with(|| OutOfStockHit {
                component_id: li.component_id,
                sku: comp.map(|c| c.sku.clone()).unwrap_or_default(),
                description: comp
                    .map(|c| c.description.clone())
                    .unwrap_or_else(|| "(componente eliminado)".into()),
                kind: comp.map(|c| c.kind).unwrap_or(crate::domain::component::ComponentKind::Other),
                supplier: li.supplier.clone(),
                stock_after: available_after,
                newly_out: false,
                assemblies: Vec::new(),
            });
            if li.available_before >= li.quantity {
                hit.newly_out = true;
            }
            hit.assemblies.push(li.assembly.clone());
        }
        let mut hits: Vec<OutOfStockHit> = by_component.into_values().collect();
        // Los que ACABAN de agotarse primero; luego por descripción.
        hits.sort_by(|a, b| b.newly_out.cmp(&a.newly_out).then(a.description.cmp(&b.description)));
        hits
    }

    /// UPSERT por SKU de todos los componentes y, si `sweep_supplier` viene,
    /// barrido de existencias del proveedor para los SKUs ausentes. Todo en
    /// una transacción: un Excel truncado no deja la base a medias.
    /// Devuelve (actualizados, insertados, agotados por barrido).
    fn apply_import(
        &mut self,
        components: Vec<Component>,
        rate: Option<rust_decimal::Decimal>,
        list_date: String,
        source_file: String,
        sweep_supplier: Option<&str>,
    ) -> Result<(usize, usize, usize), String> {
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut updated = 0usize;
        let mut inserted = 0usize;
        let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &components {
            let existed = db::find_by_sku(&tx, &c.sku).ok().flatten().is_some();
            let supplier = if c.supplier.is_empty() { "arroba" } else { c.supplier.as_str() };
            let update_details = supplier == "arroba";
            let mut comp = c.clone();
            comp.supplier = supplier.to_string();
            normalize_title(&mut comp.description);
            db::upsert_master(&tx, &comp, update_details)
                .map_err(|e| format!("upsert_master {}: {e}", comp.sku))?;
            db::upsert_offer(&tx, &comp)
                .map_err(|e| format!("upsert_offer {}: {e}", comp.sku))?;
            present.insert(comp.sku.clone());
            if existed {
                updated += 1;
            } else {
                inserted += 1;
            }
        }
        let zeroed = match sweep_supplier {
            Some(s) if !components.is_empty() => {
                db::zero_absent_offer_stock(&tx, s, &present).map_err(|e| e.to_string())?
            }
            _ => 0,
        };
        if let Some(rate) = rate {
            let previo = rust_decimal::Decimal::from_str_exact(
                &db::get_setting(&tx, "list_rate").unwrap_or_default(),
            )
            .unwrap_or(rate);
            let efectivo = rate.max(previo);
            db::set_setting(&tx, "list_rate", &efectivo.to_string())
                .map_err(|e| e.to_string())?;
            if self.rate_override.is_none() {
                self.catalog.set_exchange_rate(ExchangeRate::new(efectivo));
            }
            self.list_rate = ExchangeRate::new(efectivo);
        }
        let _ = db::set_setting(&tx, "list_date", &list_date);
        let _ = db::set_setting(&tx, "source_file", &source_file);
        tx.commit().map_err(|e| e.to_string())?;
        let current_rate = self.catalog.exchange_rate;
        self.catalog = Catalog::from_parts(
            db::load_products(&self.db).map_err(|e| e.to_string())?,
            current_rate,
            "USD→MXP: tipo de cambio redondeado hacia arriba al entero superior",
            source_file,
            list_date,
        );
        Ok((updated, inserted, zeroed))
    }

    /// Mejora de títulos con Z.ai (requiere clave API en Ajustes). Solo
    /// procesa productos PENDIENTES ('raw'): los ya normalizados con IA
    /// quedan excluidos, así cada corrida avanza por el catálogo. Devuelve
    /// (mejorados, pendientes restantes).
    pub fn ai_normalize_titles(&mut self, limit: u32) -> Result<(usize, usize), String> {
        if self.db_query_zai().trim().is_empty() {
            return Err("Configura tu clave API de Z.ai en Ajustes primero.".into());
        }
        self.ai_normalize_titles_with(limit, crate::ai::normalize_titles_batch)
    }

    /// Variante con el llamador de Z.ai inyectable (para pruebas).
    fn ai_normalize_titles_with(
        &mut self,
        limit: u32,
        fetch: fn(&str, &[(String, String)]) -> Result<Vec<(String, String)>, String>,
    ) -> Result<(usize, usize), String> {
        let key = self.db_query_zai();
        let candidatos = db::list_raw_title_candidates(&self.db, limit)
            .map_err(|e| e.to_string())?;
        if candidatos.is_empty() {
            return Ok((0, 0));
        }
        let titulos = fetch(&key, &candidatos)?;
        let mut mejorados = 0usize;
        // Toda la partida queda marcada 'ai' — incluso los que Z.ai dejó
        // igual: ya fueron revisados y no se reenvían en la próxima corrida.
        for (sku, titulo) in &titulos {
            if let Some(mut comp) = self
                .catalog
                .components
                .iter()
                .find(|c| c.sku == *sku)
                .cloned()
            {
                if comp.description != *titulo {
                    comp.description = titulo.clone();
                    normalize_title(&mut comp.description);
                    db::upsert_master(&self.db, &comp, true).map_err(|e| e.to_string())?;
                    mejorados += 1;
                }
            }
            db::set_title_source(&self.db, sku, "ai").map_err(|e| e.to_string())?;
        }
        // Los que el lote no alcanzó a cubrir también quedan pendientes:
        // list_raw_title_candidates ya los excluye por marcado.
        let restantes = db::count_raw_title_candidates(&self.db) as usize;
        let current_rate = self.catalog.exchange_rate;
        self.catalog = Catalog::from_parts(
            db::load_products(&self.db).map_err(|e| e.to_string())?,
            current_rate,
            "USD→MXP: tipo de cambio redondeado hacia arriba al entero superior",
            self.catalog.source_file.clone(),
            self.catalog.list_date.clone(),
        );
        Ok((mejorados, restantes))
    }

    pub fn assemblies(&self) -> Vec<&Assembly> {
        let mut list: Vec<&Assembly> = self.assemblies.values().collect();
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        list
    }

    pub fn assembly(&self, id: Uuid) -> Option<&Assembly> {
        self.assemblies.get(&id)
    }

    pub fn create_assembly(&mut self, name: String, quote_type: QuoteType) -> Assembly {
        let asm = Assembly::new(name, quote_type);
        self.assemblies.insert(asm.id, asm.clone());
        self.persist_assembly(&asm);
        asm
    }

    pub fn duplicate_assembly(&mut self, id: Uuid) -> Result<Assembly, String> {
        let source = self
            .assemblies
            .get(&id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        let mut copy = source.clone();
        copy.id = Uuid::new_v4();
        copy.name = format!("{} (personalizado)", source.name);
        copy.created_at = chrono::Utc::now();
        copy.updated_at = chrono::Utc::now();
        copy.confirmed_at = None;
        copy.quote_type = QuoteType::Personalizado;
        copy.in_fabrication = false;
        copy.quote_number = None;
        copy.based_on = Some(match source.quote_number {
            Some(ref folio) => format!("#{folio} {}", source.name),
            None => source.name.clone(),
        });
        self.assemblies.insert(copy.id, copy.clone());
        self.persist_assembly(&copy);
        Ok(copy)
    }

    pub fn set_fabrication(&mut self, id: Uuid, in_fabrication: bool) -> Result<Assembly, String> {
        let asm = self
            .assemblies
            .get_mut(&id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        asm.in_fabrication = in_fabrication;
        asm.updated_at = chrono::Utc::now();
        let clone = asm.clone();
        self.persist_assembly(&clone);
        Ok(clone)
    }

    pub fn assign_quote_number(&mut self, id: Uuid) -> Result<String, String> {
        let number = self.next_quote_number();
        let asm = self
            .assemblies
            .get_mut(&id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        asm.quote_number = Some(number.clone());
        let clone = asm.clone();
        self.persist_assembly(&clone);
        Ok(number)
    }

    pub fn rename_assembly(&mut self, id: Uuid, name: String) -> Option<Assembly> {
        let asm = self.assemblies.get_mut(&id)?;
        asm.name = name;
        let clone = asm.clone();
        self.persist_assembly(&clone);
        Some(clone)
    }

    pub fn delete_assembly(&mut self, id: Uuid) -> bool {
        let existed = self.assemblies.remove(&id).is_some();
        if existed {
            db::delete_assembly(&self.db, &id.to_string()).expect("eliminar ensamble");
        }
        existed
    }

    pub fn next_quote_number(&mut self) -> String {
        let today = chrono::Local::now().date_naive();
        let key = today.format("%Y-%m-%d").to_string();
        let counter = self.quote_counters.entry(key.clone()).or_insert(0);
        *counter += 1;
        let n = *counter;
        self.set_counter_db("quote", &key, n);
        format!("{}-{}", today.format("%d%m%y"), n)
    }

    pub fn next_sale_note_number(&mut self) -> String {
        let today = chrono::Local::now().date_naive();
        let key = today.format("%Y-%m-%d").to_string();
        let counter = self.sale_note_counters.entry(key.clone()).or_insert(0);
        *counter += 1;
        let n = *counter;
        self.set_counter_db("sale_note", &key, n);
        format!("{}-{}", today.format("%d%m%Y"), n)
    }

    pub fn record_sale_note(
        &mut self,
        record: crate::domain::sale::SaleNoteRecord,
    ) -> Result<crate::domain::sale::SaleNoteRecord, String> {
        if let Some(id) = record.assembly_id {
            let asm = self
                .assemblies
                .get(&id)
                .ok_or_else(|| "Ensamble no encontrado".to_string())?;
            let today = chrono::Local::now().date_naive();
            // Regla del dominio: caducada no vende (en fabricación sí).
            if !asm.can_issue_sale_note(today) {
                return Err(format!(
                    "La cotización {} está CADUCADA (venció el {}): no se puede generar una nota de venta.",
                    asm.quote_number.as_deref().unwrap_or("sin folio"),
                    asm.expiry().format("%d/%m/%Y")
                ));
            }
            // Un ensamble con componentes agotados (tras una actualización
            // de lista) tampoco se vende.
            let report = assembly::validate(asm, &self.catalog);
            let agotados: Vec<String> = report
                .issues
                .iter()
                .filter(|i| i.code == "out_of_stock")
                .map(|i| i.message.clone())
                .collect();
            if !agotados.is_empty() {
                return Err(format!(
                    "La cotización {} tiene componentes sin existencia y no se puede vender: {}",
                    asm.quote_number.as_deref().unwrap_or("sin folio"),
                    agotados.join(" ")
                ));
            }
        }
        db::insert_sale_note(&self.db, &record).map_err(|e| e.to_string())?;
        self.sale_notes.push(record.clone());
        Ok(record)
    }

    pub fn sale_notes(&self) -> &[crate::domain::sale::SaleNoteRecord] {
        &self.sale_notes
    }

    pub fn save_export(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("No se pudo crear la carpeta: {e}"))?;
        }
        fs::write(&path, bytes).map_err(|e| format!("No se pudo guardar: {e}"))
    }

    pub fn confirm_assembly(&mut self, id: Uuid) -> Result<Assembly, String> {
        let report = self
            .validate(id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        if !report.is_valid {
            let pendientes: Vec<String> = report
                .issues
                .iter()
                .filter(|i| i.severity == assembly::Severity::Error)
                .map(|i| i.message.clone())
                .collect();
            return Err(format!("El ensamble está incompleto: {}", pendientes.join(" ")));
        }
        let asm = self
            .assemblies
            .get_mut(&id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        asm.confirmed_at = Some(chrono::Utc::now());
        let clone = asm.clone();
        self.persist_assembly(&clone);
        Ok(clone)
    }

    pub fn set_assembly_item(
        &mut self,
        assembly_id: Uuid,
        slot: Slot,
        component_id: Uuid,
        quantity: u32,
        supplier: Option<String>,
    ) -> Result<Assembly, String> {
        let component = self
            .catalog
            .get_component(&component_id)
            .cloned()
            .ok_or_else(|| "Componente no encontrado en el catálogo".to_string())?;
        let asm = self
            .assemblies
            .get_mut(&assembly_id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        // No se puede PEDIR más de lo que hay para ensamble. Reducir o
        // quitar (quantity 0) siempre se permite, para poder corregir un
        // ensamble cuyo componente se agotó después de armarlo.
        let actual = asm
            .items(slot)
            .iter()
            .find(|i| i.component_id == component_id)
            .map(|i| i.quantity)
            .unwrap_or(0);
        if quantity > actual {
            let disponible = component.stock_for_assembly(supplier.as_deref());
            if quantity > disponible {
                return Err(format!(
                    "Sin existencia para ensamble: {} (pedido {}, disponible {}).",
                    component.sku, quantity, disponible
                ));
            }
        }
        asm.set_quantity(slot, &component, quantity, supplier)
            .map_err(|e| e.to_string())?;
        let clone = asm.clone();
        self.persist_assembly(&clone);
        Ok(clone)
    }

    pub fn clear_assembly_slot(&mut self, assembly_id: Uuid, slot: Slot) -> Result<Assembly, String> {
        let asm = self
            .assemblies
            .get_mut(&assembly_id)
            .ok_or_else(|| "Ensamble no encontrado".to_string())?;
        asm.clear_slot(slot);
        let clone = asm.clone();
        self.persist_assembly(&clone);
        Ok(clone)
    }

    pub fn validate(&self, assembly_id: Uuid) -> Option<assembly::ValidationReport> {
        self.assemblies
            .get(&assembly_id)
            .map(|asm| assembly::validate(asm, &self.catalog))
    }

    pub fn add_manual_component(&mut self, input: NewComponentInput) -> Result<Component, String> {
        let existencia = input.stock.unwrap_or(0);
        let es_pch = input.supplier.as_deref() == Some("pch");
        let mut input = input;
        // SKU autogenerado con prefijo del proveedor si viene vacío
        if input.sku.as_deref().map_or(true, |s| s.trim().is_empty()) {
            let prefix = if es_pch { "PCH" } else { "MANUAL" };
            input.sku = Some(format!(
                "{}-{}",
                prefix,
                chrono::Utc::now().format("%Y%m%d%H%M%S%3f")
            ));
        }
        let sku = input.sku.clone().unwrap();

        // Regla multi-proveedor: si el SKU YA existe, una captura PCH no
        // crea un producto nuevo ni toca el maestro — registra/actualiza
        // SOLAMENTE la oferta PCH (precio + existencia). Una captura
        // manual genérica sobre un SKU existente sí se rechaza.
        if let Some(existing) = self.find_by_sku(&sku) {
            if !es_pch {
                return Err(format!(
                    "El SKU {} ya existe (proveedor {}). Si quieres actualizarlo, edítalo desde el catálogo.",
                    sku,
                    if existing.supplier.is_empty() { "sin proveedor" } else { &existing.supplier }
                ));
            }
            let precio = rust_decimal::Decimal::from_str_exact(&input.price_amount)
                .map_err(|_| format!("Precio inválido: {}", input.price_amount))?;
            let mut oferta_pch = existing.clone();
            oferta_pch.supplier = "pch".into();
            oferta_pch.price = Money::new(precio, input.price_currency);
            oferta_pch.on_promotion = false;
            oferta_pch.offer_stocks.clear();
            if existencia > 0 {
                oferta_pch.offer_stocks.insert("pch_existencia".into(), existencia);
            }
            oferta_pch.stock_ensamble_total = existencia;
            oferta_pch.stock_individual_total = existencia;
            oferta_pch.cd_only_best = false;
            db::upsert_offer(&self.db, &oferta_pch).map_err(|e| e.to_string())?;
            return self
                .find_by_sku(&sku)
                .ok_or_else(|| "No se pudo recargar la oferta".to_string());
        }

        let mut c = self.catalog.add_manual(input).map_err(|e| e.to_string())?;
        // Captura manual de proveedor: 'pch' (sin Excel, existencia a mano)
        // o 'manual' genérico.
        c.supplier = if es_pch { "pch".into() } else { "manual".into() };
        // SKU auto-generado con prefijo del proveedor
        if es_pch && c.sku.starts_with("MANUAL-") {
            c.sku = format!("PCH-{}", &c.sku["MANUAL-".len()..]);
        }
        c.stock_ensamble_total = existencia;
        c.stock_individual_total = existencia;
        c.offer_stocks.clear();
        if existencia > 0 {
            c.offer_stocks
                .insert(format!("{}_existencia", c.supplier), existencia);
        }
        c.offers = vec![crate::domain::component::Offer {
            supplier: c.supplier.clone(),
            price: c.price.clone(),
            on_promotion: c.on_promotion,
            stock_label: format!("Existencia {existencia}"),
            stocks: c.offer_stocks.clone(),
            stock_ensamble: existencia,
            stock_individual: existencia,
            cd_only: false,
            price_overridden: false,
            stock_overridden: false,
            list_price: None,
            list_stock: None,
        }];
        db::upsert_master(&self.db, &c, true).map_err(|e| e.to_string())?;
        db::upsert_offer(&self.db, &c).map_err(|e| e.to_string())?;
        // El catálogo en memoria recibió una copia sin oferta ni existencia
        // (add_manual la crea antes de fijarlas): se recarga desde la base
        // para que la validación de stock vea lo que realmente se guardó.
        self.reload_catalog()?;
        self.catalog
            .get_component(&c.id)
            .cloned()
            .ok_or_else(|| "No se pudo recargar el componente".into())
    }

    /// Edita un componente existente. El maestro (descripción, marca, tipo,
    /// condición, specs) siempre; precio/existencia solo de una oferta
    /// `manual` o `pch` — las de lista se actualizan con el Excel.
    pub fn update_component(
        &mut self,
        id: Uuid,
        input: UpdateComponentInput,
    ) -> Result<Component, String> {
        let existing = self
            .catalog
            .get_component(&id)
            .cloned()
            .ok_or_else(|| "Componente no encontrado en el catálogo".to_string())?;
        if input.description.trim().is_empty() {
            return Err("La descripción es obligatoria".into());
        }
        let mut description = input.description.trim().to_string();
        normalize_title(&mut description);
        let brand = input
            .brand
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or("SIN MARCA");

        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        db::update_master_manual(
            &tx,
            &id.to_string(),
            &description,
            brand,
            input.condition,
            input.kind,
            &input.specs,
            existing.has_list_offer(),
        )
        .map_err(|e| e.to_string())?;

        if let Some(supplier) = input.supplier.as_deref() {
            let offer = existing
                .offers
                .iter()
                .find(|o| o.supplier == supplier)
                .ok_or_else(|| format!("El componente no tiene una oferta del proveedor {supplier}"))?;
            let amount = match input.price_amount.as_deref() {
                Some(a) => rust_decimal::Decimal::from_str_exact(a.trim())
                    .map_err(|_| format!("Precio inválido: {a}"))?,
                None => offer.price.amount,
            };
            if amount < rust_decimal::Decimal::ZERO {
                return Err("El precio debe ser mayor o igual a cero".into());
            }
            let currency = input.price_currency.unwrap_or(offer.price.currency);
            let money = Money::new(amount, currency);
            let stock = input.stock.unwrap_or(offer.stock_ensamble);

            match supplier {
                // Captura a mano (sin Excel): se escribe directo en la oferta.
                "manual" | "pch" => {
                    db::update_offer_manual(&tx, &existing.sku, supplier, &money, stock)
                        .map_err(|e| e.to_string())?;
                }
                // Oferta de lista (arroba/cva/…): el Excel trae datos
                // incompletos, así que se permite corregir. Solo se protege
                // lo que difiere del valor vigente; si se deja igual a la
                // lista, la próxima importación vuelve a mandar.
                _ => {
                    let price_changed = money != offer.price;
                    let stock_changed = stock != offer.stock_ensamble;
                    if price_changed || stock_changed {
                        db::set_offer_override(
                            &tx,
                            &existing.sku,
                            supplier,
                            price_changed.then_some(&money),
                            stock_changed.then_some(stock),
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.reload_catalog()?;
        self.catalog
            .get_component(&id)
            .cloned()
            .ok_or_else(|| "No se pudo recargar el componente".into())
    }

    /// Quita la corrección manual de una oferta de lista ("usar lista"):
    /// vuelve a mandar el valor que trajo la última importación.
    pub fn clear_offer_override(&mut self, id: Uuid, supplier: &str) -> Result<Component, String> {
        let existing = self
            .catalog
            .get_component(&id)
            .cloned()
            .ok_or_else(|| "Componente no encontrado en el catálogo".to_string())?;
        if matches!(supplier, "manual" | "pch") {
            return Err("Las ofertas capturadas a mano no tienen valor de lista que restablecer".into());
        }
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        db::clear_offer_override(&tx, &existing.sku, supplier).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.reload_catalog()?;
        self.catalog
            .get_component(&id)
            .cloned()
            .ok_or_else(|| "No se pudo recargar el componente".into())
    }

    /// Reconstruye el catálogo en memoria desde la base conservando
    /// tipo de cambio, archivo y fecha de lista.
    fn reload_catalog(&mut self) -> Result<(), String> {
        let current_rate = self.catalog.exchange_rate;
        let source_file = self.catalog.source_file.clone();
        let list_date = self.catalog.list_date.clone();
        self.catalog = Catalog::from_parts(
            db::load_products(&self.db).map_err(|e| e.to_string())?,
            current_rate,
            "USD→MXP: tipo de cambio redondeado hacia arriba al entero superior",
            source_file,
            list_date,
        );
        Ok(())
    }

    pub fn remove_manual_component(&mut self, id: Uuid) -> Result<bool, String> {
        let removed = self.catalog.remove_manual(id);
        if removed {
            db::delete_product(&self.db, &id.to_string()).map_err(|e| e.to_string())?;
        }
        Ok(removed)
    }
}

/// Normalización mecánica del título único: colapsa espacios y recorta.
pub fn normalize_title(title: &mut String) {
    let collapsed: Vec<String> = title.split_whitespace().map(|w| w.to_string()).collect();
    *title = collapsed.join(" ");
}

/// Estado compartido de Tauri.
pub struct SharedState(pub Mutex<AppState>);

pub type CmdResult<T> = Result<T, String>;

pub fn parse_uuid(s: &str) -> Result<Uuid, String> {
    Uuid::parse_str(s).map_err(|e| format!("UUID inválido: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::component::{ComponentKind, Overrides};

    /// Semilla sintética para pruebas (datos inventados, sin precios reales
    /// de proveedores — los fixtures del repo son seguros de publicar).
    const SEED: &str = r#"{
        "source_file": "SEED-TEST.xlsx",
        "list_date": "2026-10-15",
        "exchange_rate": "18.52",
        "exchange_rule": "USD→MXP: tipo de cambio redondeado hacia arriba al entero superior",
        "components": [
            {"id": "11111111-1111-4111-8111-111111111101", "sku": "100-100001582WOF", "barcode": null,
             "description": "PROCESADOR AMD RYZEN 7 5800XT AM4 8NUCLEOS NO INCLUYE DISIPADOR",
             "brand": "AMD", "department": "PROCESADORES", "kind": "cpu", "condition": "nuevo",
             "price": {"amount": "210.50", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"socket": "AM4", "includes_cooler": false}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111102", "sku": "100-100000263BOX", "barcode": null,
             "description": "PROCESADOR AMD RYZEN 7 5700G AM4 CON GRAFICAS CON DISIPADOR",
             "brand": "AMD", "department": "PROCESADORES", "kind": "cpu", "condition": "nuevo",
             "price": {"amount": "314.06", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"socket": "AM4", "includes_cooler": true}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111103", "sku": "MB-AM5-TEST", "barcode": null,
             "description": "MB TEST AM5 DDR5 PCIE4.0",
             "brand": "TEST", "department": "MOTHER BOARD", "kind": "motherboard", "condition": "nuevo",
             "price": {"amount": "120.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"socket": "AM5", "ddr": "DDR5", "form_factor": "ATX"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111104", "sku": "MB-AM4-TEST", "barcode": null,
             "description": "MB TEST AM4 DDR4 MATX",
             "brand": "TEST", "department": "MOTHER BOARD", "kind": "motherboard", "condition": "nuevo",
             "price": {"amount": "80.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"socket": "AM4", "ddr": "DDR4", "form_factor": "MICRO ATX"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111105", "sku": "RAM-DDR5-TEST", "barcode": null,
             "description": "MEMORIA RAM DIMM 16GB DDR5 5600MHZ",
             "brand": "TEST", "department": "MEMORIA RAM", "kind": "ram", "condition": "nuevo",
             "price": {"amount": "60.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"ddr": "DDR5", "ram_form": "DIMM", "capacity_gb": 16}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111106", "sku": "RAM-DDR4-TEST", "barcode": null,
             "description": "MEMORIA RAM DIMM 16GB DDR4 3200MHZ",
             "brand": "TEST", "department": "MEMORIA RAM", "kind": "ram", "condition": "nuevo",
             "price": {"amount": "40.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"ddr": "DDR4", "ram_form": "DIMM", "capacity_gb": 16}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111107", "sku": "SSD-1TB-TEST", "barcode": null,
             "description": "SSD INTERNO 1TB NVME",
             "brand": "TEST", "department": "SSD INTERNO", "kind": "storage", "condition": "nuevo",
             "price": {"amount": "55.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"storage_kind": "ssd", "internal": true, "capacity_raw": "1TB"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111108", "sku": "PSU-650-TEST", "barcode": null,
             "description": "FUENTE DE PODER TEST 650W BRONCE",
             "brand": "TEST", "department": "COMPONENTES", "kind": "psu", "condition": "nuevo",
             "price": {"amount": "45.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"wattage": 650}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111109", "sku": "EV-1015", "barcode": null,
             "description": "GABINETE TEST ATX CON FUENTE 600W",
             "brand": "TEST", "department": "COMPONENTES", "kind": "case", "condition": "nuevo",
             "price": {"amount": "269.57", "currency": "MXP"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {"includes_psu": true, "psu_watts": 600, "form_factor": "ATX"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111110", "sku": "ENF-AIRE-TEST", "barcode": null,
             "description": "VENTILADOR TEST 120MM PWM",
             "brand": "TEST", "department": "ENFRIAMIENTO", "kind": "cooler_air", "condition": "nuevo",
             "price": {"amount": "10.00", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111111", "sku": "ENF-LQ-240", "barcode": null,
             "description": "ENFRIAMIENTO LIQUIDO TEST 240MM",
             "brand": "TEST", "department": "ENFRIAMIENTO", "kind": "cooler_liquid", "condition": "nuevo",
             "price": {"amount": "89.90", "currency": "USD"}, "on_promotion": false, "offer_stocks": {"a_cen": 5},
             "specs": {}, "source": "lista"}
        ]
    }"#;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("genpresu-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn first_of(state: &AppState, kind: ComponentKind) -> Uuid {
        state
            .catalog()
            .search(Some(kind), "")
            .iter()
            // Las SODIMM son para laptop: el flujo de escritorio usa DIMM.
            .find(|c| !crate::domain::assembly::is_laptop_ram(c))
            .map(|c| c.id)
            .or_else(|| state.catalog().search(Some(kind), "").first().map(|c| c.id))
            .expect("debe haber al menos un componente de ese tipo")
    }

    /// Flujo completo sobre la semilla real: crear ensamble, llenar slots con
    /// componentes del catálogo, validar y persistir/recargar.
    #[test]
    fn full_assembly_flow_with_real_seed() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());

        let asm = state.create_assembly("Equipo de oficina".into(), crate::domain::assembly::QuoteType::Personalizado);
        let cpu = first_of(&state, ComponentKind::Cpu);
        let mobo = first_of(&state, ComponentKind::Motherboard);
        let ram = first_of(&state, ComponentKind::Ram);
        let storage = first_of(&state, ComponentKind::Storage);
        let psu = first_of(&state, ComponentKind::Psu);
        let case = first_of(&state, ComponentKind::Case);

        for (slot, id) in [
            (Slot::Cpu, cpu),
            (Slot::Motherboard, mobo),
            (Slot::Ram, ram),
            (Slot::Storage, storage),
            (Slot::Psu, psu),
            (Slot::Case, case),
        ] {
            state.set_assembly_item(asm.id, slot, id, 1, None).unwrap();
        }

        // La mayoría de los CPU de la lista no traen disipador → falta enfriamiento
        let report = state.validate(asm.id).unwrap();
        if !report.is_valid {
            let cooler = first_of(&state, ComponentKind::CoolerAir);
            state.set_assembly_item(asm.id, Slot::Cooling, cooler, 1, None).unwrap();
        }
        let report = state.validate(asm.id).unwrap();
        assert!(report.is_valid, "issues: {:?}", report.issues);

        // Fase 2: el costo se calcula sobre la semilla real con la regla
        // subtotal (precios a MXN) + IVA 16% = total.
        let rate = state.rate();
        let cost = crate::domain::cost::compute_cost(
            state.assembly(asm.id).unwrap(),
            state.catalog(),
            &rate,
        );
        assert!(cost.subtotal.amount > rust_decimal::Decimal::ZERO);
        assert_eq!(
            cost.subtotal.amount + cost.iva.amount,
            cost.total.amount,
            "subtotal + IVA debe ser igual al total"
        );
        let iva_esperado = (cost.subtotal.amount
            * rust_decimal::Decimal::new(16, 2))
        .round_dp_with_strategy(
            2,
            rust_decimal::RoundingStrategy::MidpointAwayFromZero,
        );
        assert_eq!(cost.iva.amount, iva_esperado, "IVA debe ser 16% del subtotal");

        // Persistencia: recargar debe conservar el ensamble y su contenido
        let reloaded = AppState::load(Some(SEED), dir.clone());
        let saved = reloaded.assembly(asm.id).expect("el ensamble debe persistir");
        assert_eq!(saved.name, "Equipo de oficina");
        assert_eq!(saved.total_quantity(Slot::Cpu), 1);
        assert_eq!(saved.total_quantity(Slot::Storage), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Confirmar exige ensamble completo; al confirmar queda persistido.
    #[test]
    fn confirm_requires_valid_and_persists() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());

        // Ensamble vacío: no se puede confirmar
        let asm = state.create_assembly("Incompleto".into(), crate::domain::assembly::QuoteType::Personalizado);
        let err = state.confirm_assembly(asm.id).unwrap_err();
        assert!(err.contains("incompleto"), "error: {err}");

        // Llenar el ensamble y confirmar
        let cpu = first_of(&state, ComponentKind::Cpu);
        let mobo = first_of(&state, ComponentKind::Motherboard);
        let ram = first_of(&state, ComponentKind::Ram);
        let storage = first_of(&state, ComponentKind::Storage);
        let psu = first_of(&state, ComponentKind::Psu);
        let case = first_of(&state, ComponentKind::Case);
        for (slot, id) in [
            (Slot::Cpu, cpu),
            (Slot::Motherboard, mobo),
            (Slot::Ram, ram),
            (Slot::Storage, storage),
            (Slot::Psu, psu),
            (Slot::Case, case),
        ] {
            state.set_assembly_item(asm.id, slot, id, 1, None).unwrap();
        }
        let report = state.validate(asm.id).unwrap();
        if !report.is_valid {
            let cooler = first_of(&state, ComponentKind::CoolerAir);
            state.set_assembly_item(asm.id, Slot::Cooling, cooler, 1, None).unwrap();
        }

        let confirmed = state.confirm_assembly(asm.id).expect("debe confirmar");
        assert!(confirmed.confirmed_at.is_some());

        // Persistido tras recargar
        let reloaded = AppState::load(Some(SEED), dir.clone());
        assert!(reloaded
            .assembly(asm.id)
            .expect("persiste")
            .confirmed_at
            .is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// El número de cotización es DDMMAA-NC: consecutivo por día y
    /// persistente entre corridas.
    #[test]
    fn quote_number_is_consecutive_and_persistent() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let n1 = state.next_quote_number();
        let n2 = state.next_quote_number();

        let today = chrono::Local::now().date_naive();
        let prefijo = today.format("%d%m%y").to_string();
        let sufijo = |s: &str| s.rsplit('-').next().unwrap().to_string();
        assert!(n1.starts_with(&prefijo), "n1={n1}");
        assert_eq!(sufijo(&n1), "1");
        assert_eq!(sufijo(&n2), "2");

        // Persistido: una nueva instancia continúa el consecutivo del día
        let mut reloaded = AppState::load(Some(SEED), dir.clone());
        let n3 = reloaded.next_quote_number();
        assert_eq!(sufijo(&n3), "3");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Número de nota de venta: DDMMAAAA-NC, consecutivo por día,
    /// independiente del contador de cotizaciones, y el registro persiste.
    #[test]
    fn sale_note_number_and_record_persist() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let n1 = state.next_sale_note_number();
        let n2 = state.next_sale_note_number();
        let today = chrono::Local::now().date_naive();
        let prefijo = today.format("%d%m%Y").to_string();
        let sufijo = |s: &str| s.rsplit('-').next().unwrap().to_string();
        assert!(n1.starts_with(&prefijo) && n1.ends_with("-1"), "n1={n1}");
        assert_eq!(sufijo(&n2), "2");

        let asm = state.create_assembly("Venta".into(), crate::domain::assembly::QuoteType::Personalizado);
        let record = state
            .record_sale_note(crate::domain::sale::SaleNoteRecord {
                number: n1.clone(),
                quote_number: Some("151026-1".into()),
                assembly_id: Some(asm.id),
                created_at: chrono::Utc::now(),
                lines: vec![],
            })
            .expect("el ensamble vigente permite vender");
        assert_eq!(record.number, n1);

        let mut reloaded = AppState::load(Some(SEED), dir.clone());
        assert_eq!(reloaded.sale_notes().len(), 1);
        assert_eq!(reloaded.sale_notes()[0].quote_number.as_deref(), Some("151026-1"));
        // El contador de la nota de venta también continúa
        assert_eq!(sufijo(&reloaded.next_sale_note_number()), "3");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exchange_rate_override_persists() {
        let dir = temp_dir();
        {
            let mut state = AppState::load(Some(SEED), dir.clone());
            assert!(!state.rate_is_overridden());
            state.set_rate(ExchangeRate::new(rust_decimal::Decimal::try_from(20.5).unwrap()));
        }
        let reloaded = AppState::load(Some(SEED), dir.clone());
        assert!(reloaded.rate_is_overridden());
        // 20.5 → efectivo 21
        assert_eq!(
            reloaded.rate().effective(),
            rust_decimal::Decimal::from(21)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// La primera corrida registra TODOS los productos de la semilla en la
    /// base, consultables por SKU; la segunda no duplica.
    #[test]
    fn first_run_registers_all_products_queryable_by_sku() {
        let dir = temp_dir();
        let state = AppState::load(Some(SEED), dir.clone());
        assert_eq!(state.catalog().components.len(), 11, "11 productos sintéticos registrados");
        let case = state.find_by_sku("EV-1015").expect("consulta por SKU");
        assert_eq!(case.kind, ComponentKind::Case);
        assert_eq!(case.specs.psu_watts, Some(600));
        let reloaded = AppState::load(Some(SEED), dir.clone());
        assert_eq!(reloaded.catalog().components.len(), 11);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Importar un catálogo actualiza por SKU (conservando id) e inserta nuevos.
    #[test]
    fn import_catalog_upserts_by_sku() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let id_original = state.find_by_sku("EV-1015").unwrap().id;

        let mini = serde_json::json!({
            "source_file": "LISTA NUEVA.xlsx",
            "list_date": "2026-10-20",
            "exchange_rate": "19.50",
            "components": [
                {
                    "id": "00000000-0000-0000-0000-000000000099",
                    "sku": "EV-1015",
                    "barcode": null,
                    "description": "GABINETE EVOTEC ACTUALIZADO",
                    "brand": "EVOTEC", "department": "COMPONENTES",
                    "kind": "case", "condition": "nuevo",
                    "price": {"amount": "50.00", "currency": "USD"},
                    "on_promotion": false, "stock": {}, "source": "lista",
                    "specs": {"includes_psu": true, "psu_watts": 700}
                },
                {
                    "id": "00000000-0000-0000-0000-000000000098",
                    "sku": "NUEVO-SKU-1",
                    "barcode": null,
                    "description": "PRODUCTO NUEVO",
                    "brand": "X", "department": "COMPONENTES",
                    "kind": "psu", "condition": "nuevo",
                    "price": {"amount": "10.00", "currency": "USD"},
                    "on_promotion": false, "stock": {}, "source": "lista",
                    "specs": {"wattage": 750}
                }
            ]
        });
        let (updated, inserted) = state.import_catalog(&mini.to_string()).expect("importar");
        assert_eq!((updated, inserted), (1, 1));
        assert_eq!(state.catalog().components.len(), 12);

        let case = state.find_by_sku("EV-1015").unwrap();
        assert_eq!(case.description, "GABINETE EVOTEC ACTUALIZADO");
        assert_eq!(case.specs.psu_watts, Some(700));
        assert_eq!(case.id, id_original, "el id se conserva entre imports");

        let reloaded = AppState::load(Some(SEED), dir.clone());
        assert!(reloaded.find_by_sku("NUEVO-SKU-1").is_some());
        assert_eq!(reloaded.list_rate().effective(), rust_decimal::Decimal::from(20));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Un personalizado generado desde un preensamble guarda su origen.
    #[test]
    fn duplicate_from_preensamble_records_origin() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let pre = state.create_assembly(
            "Base Gamer".into(),
            crate::domain::assembly::QuoteType::Preensamble,
        );
        state.assign_quote_number(pre.id).unwrap();
        let folio = state
            .assembly(pre.id)
            .and_then(|a| a.quote_number.clone())
            .expect("folio asignado");

        let copy = state.duplicate_assembly(pre.id).unwrap();
        assert_eq!(copy.quote_type, crate::domain::assembly::QuoteType::Personalizado);
        assert_eq!(copy.based_on.as_deref(), Some(format!("#{folio} Base Gamer").as_str()));
        assert!(copy.quote_number.is_none());
        assert!(copy.confirmed_at.is_none());

        // Persistido
        let reloaded = AppState::load(Some(SEED), dir.clone());
        let saved = reloaded.assembly(copy.id).expect("persiste");
        assert_eq!(
            saved.based_on.as_deref(),
            Some(format!("#{folio} Base Gamer").as_str())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Importar un Excel actualiza por SKU (conservando el id de EV-1015)
    /// e inserta los productos nuevos.
    #[test]
    fn import_xlsx_upserts_by_sku() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let id_original = state.find_by_sku("EV-1015").unwrap().id;

        let bytes = include_bytes!("../tests/fixtures/lista_nykniu.xlsx");
        let mut path = dir.clone();
        path.push("lista.xlsx");
        std::fs::write(&path, bytes).unwrap();

        let r = state.import_xlsx_catalog(path.to_str().unwrap()).unwrap();
        assert!(r.supplier.contains("arroba"), "supplier={}", r.supplier);
        assert_eq!(r.updated, 3, "5800XT, EV-1015 y ENF-LQ-240 ya existían");
        assert_eq!(r.inserted, 1, "solo la RAM Kingston es nueva");

        // El gabinete se actualizó (700W) conservando su id
        let case = state.find_by_sku("EV-1015").unwrap();
        assert_eq!(case.specs.psu_watts, Some(700));
        assert_eq!(case.price.amount, rust_decimal_macros::dec!(300));
        assert_eq!(case.id, id_original, "el id se conserva");

        // Barrido: los SKUs del SEED que no vienen en el Excel de arroba
        // quedan agotados pero conservan su oferta (último precio).
        assert_eq!(r.zeroed, 8, "11 del SEED − 3 que sí venían");
        let swept = state.find_by_sku("PSU-650-TEST").expect("la oferta se conserva");
        assert_eq!(swept.stock_ensamble_total, 0);
        assert_eq!(swept.price.amount, rust_decimal_macros::dec!(45.00));

        // El T.C de la lista nueva se aplicó (18.75 → efectivo 19)
        assert_eq!(state.list_rate().effective(), rust_decimal::Decimal::from(19));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// IMPORTACIÓN REAL (#[ignore], se ejecuta manualmente con
    /// `cargo test import_cva_real -- --ignored --nocapture`): reimporta la
    /// lista CVA usando el SKU correcto (Código de Fabricante). Primero
    /// limpia las ofertas CVA previas (que usaban la Clave interna).
    #[test]
    #[ignore]
    fn import_cva_real() {
        let home = std::env::var("HOME").unwrap();
        let db_dir = std::path::PathBuf::from(home.clone())
            .join("Library/Application Support/com.harunoyukki.generador-presupuestos");
        let xls = std::path::PathBuf::from(home).join("Downloads/dtihoxjk4d.xls");
        let state = AppState::load(Some(SEED), db_dir.clone());

        // Limpiar ofertas CVA previas (claves internas obsoletas)
        {
            let conn = state.db();
            conn.execute("DELETE FROM offer_stock WHERE supplier = 'cva'", [])
                .expect("limpiar existencias CVA");
            conn.execute("DELETE FROM product_offers WHERE supplier = 'cva'", [])
                .expect("limpiar ofertas CVA");
            // Maestros que quedaron sin ninguna oferta: fuera
            conn.execute(
                "DELETE FROM products WHERE sku NOT IN (SELECT DISTINCT sku FROM product_offers)",
                [],
            )
            .expect("limpiar maestros huérfanos");
        }
        // Recargar tras la limpieza
        let mut state = AppState::load(Some(SEED), db_dir.clone());
        let r = state.import_xlsx_catalog(xls.to_str().unwrap()).expect("importar CVA");
        println!(
            "CVA reimportado con SKU de Código de Fabricante ({}): {} actualizados, {} nuevos, {} agotados, total {}",
            r.supplier,
            r.updated,
            r.inserted,
            r.zeroed,
            state.catalog().components.len()
        );
    }

    /// La normalización con IA solo procesa pendientes ('raw'): los ya
    /// normalizados quedan excluidos de las corridas siguientes.
    #[test]
    fn ai_titles_skip_already_normalized() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let total_raw = state.catalog().components.len() as u32;

        // Lote falso: duplica el título → nada "mejora", pero todo se marca
        let fetch_duplicar = |_: &str, entries: &[(String, String)]| {
            Ok(entries
                .iter()
                .map(|(sku, t)| (sku.clone(), format!("{t} X")))
                .collect())
        };
        let (improved1, remaining1) = state
            .ai_normalize_titles_with(3, fetch_duplicar)
            .expect("primer lote");
        assert_eq!(improved1, 3);
        assert_eq!(remaining1, total_raw as usize - 3);

        // Segunda corrida con el mismo lote falso: como los primeros 3 ya
        // están marcados 'ai', los candidatos son los SIGUIENTES 3.
        let (improved2, remaining2) = state
            .ai_normalize_titles_with(3, fetch_duplicar)
            .expect("segundo lote");
        assert_eq!(improved2, 3);
        assert_eq!(remaining2, total_raw as usize - 6);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Producto PCH capturado a mano: proveedor 'pch', existencia propia,
    /// seleccionable en ensambles y ventanas de venta sin bloqueo por stock.
    #[test]
    fn pch_products_roundtrip() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());

        let c = state
            .add_manual_component(NewComponentInput {
                description: "PROCESADOR INTEL I5 PCH".into(),
                sku: None,
                brand: Some("INTEL".into()),
                kind: crate::domain::component::ComponentKind::Cpu,
                price_amount: "4200".into(),
                price_currency: crate::domain::money::Currency::Mxp,
                condition: crate::domain::component::Condition::Nuevo,
                specs: crate::domain::component::Specs {
                    socket: Some("LGA1700".into()),
                    ..Default::default()
                },
                supplier: Some("pch".into()),
                stock: Some(4),
            })
            .expect("alta PCH");

        // Proveedor y existencia
        assert_eq!(c.supplier, "pch");
        assert_eq!(c.stock_ensamble_total, 4);
        assert_eq!(c.stock_individual_total, 4);
        assert!(c.sku.starts_with("PCH-"), "SKU autogenerado: {}", c.sku);

        // Persistido con existencias por almacén PCH
        let reloaded = AppState::load(Some(SEED), dir.clone());
        let saved = reloaded.find_by_sku(&c.sku).expect("consulta por SKU");
        assert_eq!(saved.supplier, "pch");
        assert_eq!(saved.stock_ensamble_total, 4);
        assert_eq!(saved.stock_individual_total, 4);
        assert!(!saved.cd_only_best, "PCH no es solo-CD");

        // Se puede usar en un ensamble sin bloqueo
        let mut asm = state.create_assembly(
            "Con PCH".into(),
            crate::domain::assembly::QuoteType::Personalizado,
        );
        asm = state.set_assembly_item(asm.id, Slot::Cpu, saved.id, 1, Some("pch".to_string())).unwrap();
        assert_eq!(asm.total_quantity(Slot::Cpu), 1);

        // No es PCH: alta manual genérica conserva supplier 'manual'
        let manual = state
            .add_manual_component(NewComponentInput {
                description: "USADO GENÉRICO".into(),
                sku: None,
                brand: None,
                kind: crate::domain::component::ComponentKind::Gpu,
                price_amount: "500".into(),
                price_currency: crate::domain::money::Currency::Mxp,
                condition: crate::domain::component::Condition::Usado,
                specs: crate::domain::component::Specs::default(),
                supplier: None,
                stock: None,
            })
            .unwrap();
        assert_eq!(manual.supplier, "manual");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Reglas de alta manual sobre SKUs existentes:
    /// - PCH sobre SKU existente → registra/actualiza SOLO su oferta
    ///   (precio + existencia), maestro intacto.
    /// - Captura manual genérica sobre SKU existente → rechazada.
    #[test]
    fn manual_sku_rules_over_existing() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let base = NewComponentInput {
            description: "PROCESADOR PCH UNO".into(),
            sku: Some("DUP-1".into()),
            brand: Some("INTEL".into()),
            kind: crate::domain::component::ComponentKind::Cpu,
            price_amount: "4200".into(),
            price_currency: crate::domain::money::Currency::Mxp,
            condition: crate::domain::component::Condition::Nuevo,
            specs: crate::domain::component::Specs::default(),
            supplier: Some("pch".into()),
            stock: Some(2),
        };
        state.add_manual_component(base.clone()).expect("primera alta PCH");

        // Manual genérico sobre el mismo SKU → rechazada
        let err = state
            .add_manual_component(NewComponentInput {
                description: "PROCESADOR PCH DISTINTO".into(),
                sku: base.sku.clone(),
                brand: None,
                kind: crate::domain::component::ComponentKind::Cpu,
                price_amount: "999".into(),
                price_currency: crate::domain::money::Currency::Mxp,
                condition: crate::domain::component::Condition::Nuevo,
                specs: crate::domain::component::Specs::default(),
                supplier: None,
                stock: None,
            })
            .unwrap_err();
        assert!(err.contains("DUP-1") && err.contains("ya existe"), "err: {err}");

        // PCH sobre SKU existente → actualiza SOLO su oferta PCH
        let updated = state
            .add_manual_component(NewComponentInput {
                description: "PROCESADOR PCH UNO (actualizado)".into(),
                sku: base.sku.clone(),
                brand: Some("INTEL".into()),
                kind: crate::domain::component::ComponentKind::Cpu,
                price_amount: "3900".into(),
                price_currency: crate::domain::money::Currency::Mxp,
                condition: crate::domain::component::Condition::Nuevo,
                specs: crate::domain::component::Specs::default(),
                supplier: Some("pch".into()),
                stock: Some(7),
            })
            .expect("PCH sobre SKU existente registra su oferta");

        // Maestro intacto (no lo pisa la recaptura)
        assert_eq!(updated.description, "PROCESADOR PCH UNO");
        // Oferta PCH: precio y existencia actualizados
        let pch_offer = updated.offers.iter().find(|o| o.supplier == "pch").expect("oferta PCH");
        assert_eq!(pch_offer.price.amount.to_string(), "3900");
        assert_eq!(pch_offer.stock_individual, 7);
        assert_eq!(updated.stock_individual_total, 7);

        // Persistido tras recargar
        let reloaded = AppState::load(Some(SEED), dir.clone());
        let saved = reloaded.find_by_sku("DUP-1").unwrap();
        let pch = saved.offers.iter().find(|o| o.supplier == "pch").unwrap();
        assert_eq!(pch.price.amount.to_string(), "3900");
        assert_eq!(pch.stock_individual, 7);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// El state.json legacy se migra a SQLite una sola vez.    /// El state.json legacy se migra a SQLite una sola vez.
    #[test]
    fn legacy_state_json_migrates_once() {
        let dir = temp_dir();
        {
            let _state = AppState::load(Some(SEED), dir.clone());
            let asm = crate::domain::assembly::Assembly::new(
                "Legacy",
                crate::domain::assembly::QuoteType::Preensamble,
            );
            let legacy = serde_json::json!({
                "assemblies": [{
                    "id": asm.id.to_string(),
                    "name": "Legacy",
                    "created_at": "2026-10-01T00:00:00Z",
                    "updated_at": "2026-10-01T00:00:00Z",
                    "quote_type": "preensamble",
                    "slots": {}
                }],
                "quote_counters": {"2026-10-01": 5}
            });
            std::fs::write(dir.join("state.json"), legacy.to_string()).unwrap();
        }
        let reloaded = AppState::load(Some(SEED), dir.clone());
        let legacy = reloaded
            .assemblies()
            .into_iter()
            .find(|a| a.name == "Legacy")
            .expect("el ensamble legacy migró a SQLite");
        assert_eq!(legacy.quote_type, crate::domain::assembly::QuoteType::Preensamble);
        assert!(dir.join("state.json.migrated").exists());
        assert!(!dir.join("state.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn write_fixture(dir: &PathBuf) -> String {
        let bytes = include_bytes!("../tests/fixtures/lista_nykniu.xlsx");
        let path = dir.join("lista.xlsx");
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn edit_input(c: &Component) -> UpdateComponentInput {
        UpdateComponentInput {
            description: c.description.clone(),
            brand: Some(c.brand.clone()),
            kind: c.kind,
            condition: c.condition,
            specs: c.specs.clone(),
            supplier: None,
            price_amount: None,
            price_currency: None,
            stock: None,
        }
    }

    /// Marcar a mano "este gabinete NO incluye fuente" sobre un SKU de lista
    /// se guarda como override (solo la clave que difiere) y sobrevive a la
    /// siguiente importación de arroba, que sí pisa el resto de specs.
    #[test]
    fn manual_spec_override_survives_arroba_reimport() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let case = state.find_by_sku("EV-1015").unwrap();
        assert_eq!(case.specs.includes_psu, Some(true));
        assert!(case.has_list_offer());

        let mut input = edit_input(&case);
        input.specs.includes_psu = Some(false);
        let edited = state.update_component(case.id, input).unwrap();
        assert_eq!(edited.specs.includes_psu, Some(false));
        assert_eq!(edited.list_specs.includes_psu, Some(true), "la lista no se toca");
        assert_eq!(edited.overrides.specs.len(), 1, "solo la clave corregida: {:?}", edited.overrides.specs);
        assert!(edited.overrides.specs.contains_key("includes_psu"));
        assert!(edited.included_psu_spec().is_none());

        // Re-importar arroba: la lista trae EV-1015 CON FUENTE 700W.
        let path = write_fixture(&dir);
        state.import_xlsx_catalog(&path).unwrap();
        let after = state.find_by_sku("EV-1015").unwrap();
        assert_eq!(after.specs.includes_psu, Some(false), "el override gana");
        assert_eq!(after.specs.psu_watts, Some(700), "lo no corregido sí se actualiza");
        assert_eq!(after.list_specs.psu_watts, Some(700));

        // Volver al valor de la lista limpia el override.
        let mut input = edit_input(&after);
        input.specs.includes_psu = Some(true);
        let restored = state.update_component(after.id, input).unwrap();
        assert!(restored.overrides == Overrides::default(), "{:?}", restored.overrides);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Precio y existencia de una oferta de LISTA se corrigen a mano como
    /// override protegido del import (el Excel de CVA trae datos
    /// incompletos); las ofertas manual/PCH se escriben directo.
    #[test]
    fn update_component_manual_offer_and_list_price_rules() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let cpu = state.find_by_sku("100-100001582WOF").unwrap();
        assert_eq!(cpu.price.amount, rust_decimal_macros::dec!(210.50));

        // Corrección de precio sobre la oferta arroba: override con los
        // valores de lista preservados para poder revertir.
        let mut input = edit_input(&cpu);
        input.supplier = Some("arroba".into());
        input.price_amount = Some("1".into());
        let edited = state.update_component(cpu.id, input).unwrap();
        assert_eq!(edited.price.amount, rust_decimal_macros::dec!(1));
        let arroba = edited.offers.iter().find(|o| o.supplier == "arroba").unwrap();
        assert!(arroba.price_overridden);
        assert!(!arroba.stock_overridden);
        assert_eq!(arroba.list_price.as_ref().unwrap().amount, rust_decimal_macros::dec!(210.50));
        assert_eq!(arroba.list_stock, None, "solo se corrigió el precio");

        // Dejar el precio igual al vigente no crea override nuevo.
        let mut input = edit_input(&edited);
        input.supplier = Some("arroba".into());
        input.price_amount = Some("1".into());
        let same = state.update_component(edited.id, input).unwrap();
        assert!(same.offers[0].price_overridden, "el override previo se conserva");

        let manual = state
            .add_manual_component(NewComponentInput {
                description: "GPU USADA".into(),
                sku: Some("USADA-GPU".into()),
                brand: None,
                kind: crate::domain::component::ComponentKind::Gpu,
                price_amount: "500".into(),
                price_currency: crate::domain::money::Currency::Mxp,
                condition: crate::domain::component::Condition::Usado,
                specs: crate::domain::component::Specs::default(),
                supplier: None,
                stock: Some(2),
            })
            .unwrap();
        assert_eq!(manual.stock_ensamble_total, 2, "la existencia manual se persiste");
        assert!(!manual.has_list_offer());

        let mut input = edit_input(&manual);
        input.description = "GPU USADA RTX 3060".into();
        input.specs.vram = Some("12GB".into());
        input.supplier = Some("manual".into());
        input.price_amount = Some("650".into());
        input.stock = Some(7);
        let edited = state.update_component(manual.id, input).unwrap();
        assert_eq!(edited.description, "GPU USADA RTX 3060");
        assert_eq!(edited.specs.vram.as_deref(), Some("12GB"));
        assert!(edited.overrides == Overrides::default(), "sin lista no hay override");
        assert_eq!(edited.price.amount, rust_decimal_macros::dec!(650));
        assert_eq!(edited.stock_ensamble_total, 7);

        let reloaded = AppState::load(Some(SEED), dir.clone());
        let saved = reloaded.find_by_sku("USADA-GPU").unwrap();
        assert_eq!(saved.stock_ensamble_total, 7);
        assert_eq!(saved.price.amount, rust_decimal_macros::dec!(650));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// La corrección de una oferta de lista sobrevive a la reimportación
    /// del Excel (que sí actualiza lo no corregido), y "usar lista" la
    /// revierte al valor importado.
    #[test]
    fn list_offer_override_survives_reimport_and_reverts() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let psu = state.find_by_sku("PSU-650-TEST").unwrap();

        // Corregir la oferta arroba: precio y existencia.
        let mut input = edit_input(&psu);
        input.supplier = Some("arroba".into());
        input.price_amount = Some("99".into());
        input.stock = Some(42);
        let edited = state.update_component(psu.id, input).unwrap();
        assert_eq!(edited.price.amount, rust_decimal_macros::dec!(99));
        assert_eq!(edited.stock_ensamble_total, 42);

        // Reimportar arroba: la lista trae PSU-650-TEST con stock 0 (no
        // viene y el barrido lo agotaría), pero el override gana.
        let path = write_fixture(&dir);
        state.import_xlsx_catalog(&path).unwrap();
        let after = state.find_by_sku("PSU-650-TEST").unwrap();
        assert_eq!(after.price.amount, rust_decimal_macros::dec!(99), "el override de precio gana");
        assert_eq!(after.stock_ensamble_total, 42, "el override de existencia gana");
        let arroba = after.offers.iter().find(|o| o.supplier == "arroba").unwrap();
        assert_eq!(arroba.list_stock, Some(0), "la lista trajo agotado");
        assert_eq!(arroba.stock_label, "Editado a mano: 42");

        // Otro SKU sin override sí sigue la lista: quedó agotado por el barrido.
        let swept = state.find_by_sku("SSD-1TB-TEST").unwrap();
        assert_eq!(swept.stock_ensamble_total, 0);

        // "Usar lista" revierte al valor importado.
        let reverted = state.clear_offer_override(after.id, "arroba").unwrap();
        assert_eq!(reverted.price.amount, rust_decimal_macros::dec!(45.00));
        assert_eq!(reverted.stock_ensamble_total, 0);
        let arroba = reverted.offers.iter().find(|o| o.supplier == "arroba").unwrap();
        assert!(!arroba.price_overridden && !arroba.stock_overridden);
        assert!(arroba.list_price.is_none() && arroba.list_stock.is_none());

        // Revertir una oferta manual no procede.
        let err = state.clear_offer_override(psu.id, "pch").unwrap_err();
        assert!(err.contains("no tienen valor de lista"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// El informe post-import lista los componentes de ensambles vigentes
    /// que quedaron sin existencia suficiente, marcando los que ACABAN de
    /// agotarse; los que siguen con existencia no aparecen.
    #[test]
    fn xlsx_import_reports_live_assemblies_out_of_stock() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let psu = state.find_by_sku("PSU-650-TEST").unwrap();
        let case = state.find_by_sku("EV-1015").unwrap();
        let asm = state.create_assembly("Oficina".into(), QuoteType::Preensamble);
        state.set_assembly_item(asm.id, Slot::Psu, psu.id, 1, None).unwrap();
        state.set_assembly_item(asm.id, Slot::Case, case.id, 1, None).unwrap();
        let asm2 = state.create_assembly("Gamer".into(), QuoteType::Personalizado);
        state.set_assembly_item(asm2.id, Slot::Psu, psu.id, 2, None).unwrap();

        let path = write_fixture(&dir);
        let r = state.import_xlsx_catalog(&path).unwrap();
        assert_eq!(r.out_of_stock.len(), 1, "{:?}", r.out_of_stock);
        let hit = &r.out_of_stock[0];
        assert_eq!(hit.sku, "PSU-650-TEST");
        assert_eq!(hit.stock_after, 0);
        assert!(hit.newly_out);
        let names: Vec<&str> = hit.assemblies.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Oficina") && names.contains(&"Gamer"), "{names:?}");
        assert!(hit.assemblies.iter().any(|a| a.quantity == 2));

        // La validación del ensamble ahora lo marca y bloquea confirmar.
        let report = state.validate(asm.id).unwrap();
        assert!(report.issues.iter().any(|i| i.code == "out_of_stock"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// No se puede pedir más de lo disponible; reducir o quitar siempre.
    #[test]
    fn set_assembly_item_respects_available_stock() {
        let dir = temp_dir();
        let mut state = AppState::load(Some(SEED), dir.clone());
        let psu = state.find_by_sku("PSU-650-TEST").unwrap();
        assert_eq!(psu.stock_ensamble_total, 5);
        let asm = state.create_assembly("Tope".into(), QuoteType::Personalizado);
        state.set_assembly_item(asm.id, Slot::Psu, psu.id, 5, None).unwrap();
        let err = state.set_assembly_item(asm.id, Slot::Psu, psu.id, 6, None).unwrap_err();
        assert!(err.contains("disponible 5"), "{err}");

        // Tras agotarse, reducir se permite; subir no.
        let path = write_fixture(&dir);
        state.import_xlsx_catalog(&path).unwrap();
        assert_eq!(state.find_by_sku("PSU-650-TEST").unwrap().stock_ensamble_total, 0);
        state.set_assembly_item(asm.id, Slot::Psu, psu.id, 2, None).unwrap();
        assert!(state.set_assembly_item(asm.id, Slot::Psu, psu.id, 3, None).is_err());
        let cleared = state.set_assembly_item(asm.id, Slot::Psu, psu.id, 0, None).unwrap();
        assert_eq!(cleared.total_quantity(Slot::Psu), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
