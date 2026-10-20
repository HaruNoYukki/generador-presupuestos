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

use serde::Deserialize;
use uuid::Uuid;

use crate::catalog::{Catalog, CatalogSeed, NewComponentInput};
use crate::db;
use crate::domain::assembly::{self, Assembly, ComponentLookup, QuoteType, Slot};
use crate::domain::component::Component;
use crate::domain::money::ExchangeRate;

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
                if let Ok(seed) = serde_json::from_str::<CatalogSeed>(json) {
                    for c in &seed.components {
                        let mut comp = c.clone();
                        comp.supplier = "arroba".into();
                        comp.stock_ensamble_total = comp.offer_stocks.values().sum();
                        comp.stock_individual_total = comp.stock_ensamble_total;
                        comp.offers.clear();
                        db::upsert_master(&conn, &comp, true).expect("seed de productos");
                        db::upsert_offer(&conn, &comp).expect("seed de ofertas");
                    }
                    let _ = db::set_setting(&conn, "list_rate", &seed.exchange_rate);
                    let _ = db::set_setting(&conn, "list_date", &seed.list_date);
                    let _ = db::set_setting(&conn, "source_file", &seed.source_file);
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
        let seed: CatalogSeed =
            serde_json::from_str(json).map_err(|e| format!("JSON de catálogo inválido: {e}"))?;
        self.apply_import(
            seed.components,
            rust_decimal::Decimal::from_str_exact(&seed.exchange_rate).ok(),
            seed.list_date,
            seed.source_file,
        )
    }

    pub fn import_xlsx_catalog(&mut self, path: &str) -> Result<(usize, usize, String), String> {
        let parsed = crate::suppliers::import_xlsx(Path::new(path))?;
        let supplier = parsed.supplier.clone();
        let components: Vec<Component> = parsed
            .products
            .into_iter()
            .map(|p| p.into_component())
            .collect();
        let (updated, inserted) = self.apply_import(
            components,
            parsed.exchange_rate,
            parsed.list_date.unwrap_or_default(),
            format!("{} ({})", parsed.source_file, supplier),
        )?;
        Ok((updated, inserted, supplier))
    }

    fn apply_import(
        &mut self,
        components: Vec<Component>,
        rate: Option<rust_decimal::Decimal>,
        list_date: String,
        source_file: String,
    ) -> Result<(usize, usize), String> {
        let mut updated = 0usize;
        let mut inserted = 0usize;
        for c in &components {
            let existed = db::find_by_sku(&self.db, &c.sku).ok().flatten().is_some();
            let supplier = if c.supplier.is_empty() { "arroba" } else { c.supplier.as_str() };
            let update_details = supplier == "arroba";
            let mut comp = c.clone();
            comp.supplier = supplier.to_string();
            normalize_title(&mut comp.description);
            db::upsert_master(&self.db, &comp, update_details)
                .map_err(|e| format!("upsert_master {}: {e}", comp.sku))?;
            db::upsert_offer(&self.db, &comp)
                .map_err(|e| format!("upsert_offer {}: {e}", comp.sku))?;
            if existed {
                updated += 1;
            } else {
                inserted += 1;
            }
        }
        if let Some(rate) = rate {
            let previo = rust_decimal::Decimal::from_str_exact(
                &db::get_setting(&self.db, "list_rate").unwrap_or_default(),
            )
            .unwrap_or(rate);
            let efectivo = rate.max(previo);
            db::set_setting(&self.db, "list_rate", &efectivo.to_string())
                .map_err(|e| e.to_string())?;
            if self.rate_override.is_none() {
                self.catalog.set_exchange_rate(ExchangeRate::new(efectivo));
            }
            self.list_rate = ExchangeRate::new(efectivo);
        }
        let _ = db::set_setting(&self.db, "list_date", &list_date);
        let _ = db::set_setting(&self.db, "source_file", &source_file);
        let current_rate = self.catalog.exchange_rate;
        self.catalog = Catalog::from_parts(
            db::load_products(&self.db).map_err(|e| e.to_string())?,
            current_rate,
            "USD→MXP: tipo de cambio redondeado hacia arriba al entero superior",
            source_file,
            list_date,
        );
        Ok((updated, inserted))
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
            if asm.status(today) == crate::domain::assembly::QuoteStatus::Caducada {
                return Err(format!(
                    "La cotización {} está CADUCADA (venció el {}): no se puede generar una nota de venta.",
                    asm.quote_number.as_deref().unwrap_or("sin folio"),
                    asm.expiry().format("%d/%m/%Y")
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
        let mut c = self.catalog.add_manual(input).map_err(|e| e.to_string())?;
        c.supplier = "manual".into();
        c.stock_ensamble_total = 0;
        c.stock_individual_total = 0;
        c.offers = vec![crate::domain::component::Offer {
            supplier: "manual".into(),
            price: c.price.clone(),
            on_promotion: c.on_promotion,
            stock_label: String::new(),
            stocks: BTreeMap::new(),
            stock_ensamble: 0,
            stock_individual: 0,
            cd_only: false,
        }];
        db::upsert_master(&self.db, &c, true).map_err(|e| e.to_string())?;
        db::upsert_offer(&self.db, &c).map_err(|e| e.to_string())?;
        Ok(c)
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
    use crate::domain::component::ComponentKind;

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
             "price": {"amount": "210.50", "currency": "USD"}, "on_promotion": false,
             "specs": {"socket": "AM4", "includes_cooler": false}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111102", "sku": "100-100000263BOX", "barcode": null,
             "description": "PROCESADOR AMD RYZEN 7 5700G AM4 CON GRAFICAS CON DISIPADOR",
             "brand": "AMD", "department": "PROCESADORES", "kind": "cpu", "condition": "nuevo",
             "price": {"amount": "314.06", "currency": "USD"}, "on_promotion": false,
             "specs": {"socket": "AM4", "includes_cooler": true}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111103", "sku": "MB-AM5-TEST", "barcode": null,
             "description": "MB TEST AM5 DDR5 PCIE4.0",
             "brand": "TEST", "department": "MOTHER BOARD", "kind": "motherboard", "condition": "nuevo",
             "price": {"amount": "120.00", "currency": "USD"}, "on_promotion": false,
             "specs": {"socket": "AM5", "ddr": "DDR5", "form_factor": "ATX"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111104", "sku": "MB-AM4-TEST", "barcode": null,
             "description": "MB TEST AM4 DDR4 MATX",
             "brand": "TEST", "department": "MOTHER BOARD", "kind": "motherboard", "condition": "nuevo",
             "price": {"amount": "80.00", "currency": "USD"}, "on_promotion": false,
             "specs": {"socket": "AM4", "ddr": "DDR4", "form_factor": "MICRO ATX"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111105", "sku": "RAM-DDR5-TEST", "barcode": null,
             "description": "MEMORIA RAM DIMM 16GB DDR5 5600MHZ",
             "brand": "TEST", "department": "MEMORIA RAM", "kind": "ram", "condition": "nuevo",
             "price": {"amount": "60.00", "currency": "USD"}, "on_promotion": false,
             "specs": {"ddr": "DDR5", "ram_form": "DIMM", "capacity_gb": 16}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111106", "sku": "RAM-DDR4-TEST", "barcode": null,
             "description": "MEMORIA RAM DIMM 16GB DDR4 3200MHZ",
             "brand": "TEST", "department": "MEMORIA RAM", "kind": "ram", "condition": "nuevo",
             "price": {"amount": "40.00", "currency": "USD"}, "on_promotion": false,
             "specs": {"ddr": "DDR4", "ram_form": "DIMM", "capacity_gb": 16}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111107", "sku": "SSD-1TB-TEST", "barcode": null,
             "description": "SSD INTERNO 1TB NVME",
             "brand": "TEST", "department": "SSD INTERNO", "kind": "storage", "condition": "nuevo",
             "price": {"amount": "55.00", "currency": "USD"}, "on_promotion": false,
             "specs": {"storage_kind": "ssd", "internal": true, "capacity_raw": "1TB"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111108", "sku": "PSU-650-TEST", "barcode": null,
             "description": "FUENTE DE PODER TEST 650W BRONCE",
             "brand": "TEST", "department": "COMPONENTES", "kind": "psu", "condition": "nuevo",
             "price": {"amount": "45.00", "currency": "USD"}, "on_promotion": false,
             "specs": {"wattage": 650}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111109", "sku": "EV-1015", "barcode": null,
             "description": "GABINETE TEST ATX CON FUENTE 600W",
             "brand": "TEST", "department": "COMPONENTES", "kind": "case", "condition": "nuevo",
             "price": {"amount": "269.57", "currency": "MXP"}, "on_promotion": false,
             "specs": {"includes_psu": true, "psu_watts": 600, "form_factor": "ATX"}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111110", "sku": "ENF-AIRE-TEST", "barcode": null,
             "description": "VENTILADOR TEST 120MM PWM",
             "brand": "TEST", "department": "ENFRIAMIENTO", "kind": "cooler_air", "condition": "nuevo",
             "price": {"amount": "10.00", "currency": "USD"}, "on_promotion": false,
             "specs": {}, "source": "lista"},
            {"id": "11111111-1111-4111-8111-111111111111", "sku": "ENF-LQ-240", "barcode": null,
             "description": "ENFRIAMIENTO LIQUIDO TEST 240MM",
             "brand": "TEST", "department": "ENFRIAMIENTO", "kind": "cooler_liquid", "condition": "nuevo",
             "price": {"amount": "89.90", "currency": "USD"}, "on_promotion": false,
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

        let (updated, inserted, supplier) = state.import_xlsx_catalog(path.to_str().unwrap()).unwrap();
        assert_eq!(supplier.contains("arroba"), true, "supplier={supplier}");
        assert_eq!(updated, 3, "5800XT, EV-1015 y ENF-LQ-240 ya existían");
        assert_eq!(inserted, 1, "solo la RAM Kingston es nueva");

        // El gabinete se actualizó (700W) conservando su id
        let case = state.find_by_sku("EV-1015").unwrap();
        assert_eq!(case.specs.psu_watts, Some(700));
        assert_eq!(case.price.amount, rust_decimal_macros::dec!(300));
        assert_eq!(case.id, id_original, "el id se conserva");

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
        let mut state = AppState::load(Some(SEED), db_dir.clone());

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
        let (updated, inserted, supplier) =
            state.import_xlsx_catalog(xls.to_str().unwrap()).expect("importar CVA");
        println!(
            "CVA reimportado con SKU de Código de Fabricante ({supplier}): {updated} actualizados, {inserted} nuevos, total {}",
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

    /// El state.json legacy se migra a SQLite una sola vez.
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
}
