//! Comandos Tauri: frontera del monolito hacia la interfaz gráfica.
//! Capa delgada: delegan en el catálogo y el estado, y arman DTOs.

use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::catalog::NewComponentInput;
use crate::domain::assembly::{Assembly, ComponentLookup, Slot, ValidationReport};
use crate::domain::component::{Component, ComponentKind};
use crate::domain::money::{Currency, ExchangeRate, Money};
use crate::store::{parse_uuid, AppState, CmdResult, SharedState};

/// Componente + precio convertido a pesos (regla: T.C redondeado hacia arriba).
#[derive(Serialize)]
pub struct ComponentDto {
    #[serde(flatten)]
    pub component: Component,
    pub price_mxp: Money,
}

fn dto(c: &Component, rate: &ExchangeRate) -> ComponentDto {
    ComponentDto {
        component: c.clone(),
        price_mxp: c.price_mxp(rate),
    }
}

#[derive(Serialize)]
pub struct CatalogOverview {
    pub source_file: String,
    pub list_date: String,
    pub exchange_rate: ExchangeRate,
    pub effective_rate: ExchangeRate,
    pub exchange_rule: String,
    pub rate_is_overridden: bool,
    pub total_components: usize,
    pub counts_by_kind: std::collections::HashMap<String, usize>,
}

#[tauri::command]
pub fn catalog_overview(state: State<'_, SharedState>) -> CmdResult<CatalogOverview> {
    let s = state.0.lock().unwrap();
    Ok(CatalogOverview {
        source_file: s.catalog().source_file.clone(),
        list_date: s.catalog().list_date.clone(),
        exchange_rate: s.catalog().exchange_rate,
        effective_rate: ExchangeRate::new(s.catalog().exchange_rate.effective()),
        exchange_rule: s.catalog().exchange_rule.clone(),
        rate_is_overridden: s.rate_is_overridden(),
        total_components: s.catalog().components.len(),
        counts_by_kind: s.catalog().counts_by_kind(),
    })
}

#[tauri::command]
pub fn list_components(
    state: State<'_, SharedState>,
    kind: Option<String>,
    search: Option<String>,
) -> CmdResult<Vec<ComponentDto>> {
    let s = state.0.lock().unwrap();
    let kind = kind
        .as_deref()
        .and_then(ComponentKind::parse);
    let query = search.unwrap_or_default();
    let rate = s.rate();
    Ok(s.catalog()
        .search(kind, &query)
        .iter()
        .map(|c| dto(c, &rate))
        .collect())
}

#[tauri::command]
pub fn get_component(state: State<'_, SharedState>, id: String) -> CmdResult<ComponentDto> {
    let s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    let c = s
        .catalog()
        .get_component(&id)
        .ok_or_else(|| "Componente no encontrado".to_string())?;
    Ok(dto(c, &s.rate()))
}

#[tauri::command]
pub fn add_manual_component(
    state: State<'_, SharedState>,
    input: NewComponentInput,
) -> CmdResult<ComponentDto> {
    let mut s = state.0.lock().unwrap();
    let c = s.add_manual_component(input)?;
    let rate = s.rate();
    Ok(dto(&c, &rate))
}

#[tauri::command]
pub fn remove_manual_component(state: State<'_, SharedState>, id: String) -> CmdResult<bool> {
    let mut s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    s.remove_manual_component(id)
}

#[derive(Serialize)]
pub struct RateInfo {
    pub rate: ExchangeRate,
    pub effective_rate: ExchangeRate,
    /// T.C original de la lista de precios (no cambia con overrides).
    pub list_rate: ExchangeRate,
    pub exchange_rule: String,
    pub source_file: String,
    pub list_date: String,
    pub is_overridden: bool,
    /// Clave API de Z.ai guardada (para el formateo de títulos).
    pub zai_key: String,
}

#[tauri::command]
pub fn get_rate(state: State<'_, SharedState>) -> CmdResult<RateInfo> {
    let s = state.0.lock().unwrap();
    Ok(RateInfo {
        rate: s.rate(),
        effective_rate: ExchangeRate::new(s.rate().effective()),
        list_rate: s.list_rate(),
        exchange_rule: s.catalog().exchange_rule.clone(),
        source_file: s.catalog().source_file.clone(),
        list_date: s.catalog().list_date.clone(),
        is_overridden: s.rate_is_overridden(),
        zai_key: s.db_query_zai(),
    })
}

#[tauri::command]
pub fn set_rate(state: State<'_, SharedState>, rate: String) -> CmdResult<RateInfo> {
    let mut s = state.0.lock().unwrap();
    let parsed = rust_decimal::Decimal::from_str_exact(&rate)
        .map_err(|_| format!("Tipo de cambio inválido: {rate}"))?;
    if parsed <= rust_decimal::Decimal::ZERO {
        return Err("El tipo de cambio debe ser mayor que cero".into());
    }
    s.set_rate(ExchangeRate::new(parsed));
    Ok(RateInfo {
        rate: s.rate(),
        effective_rate: ExchangeRate::new(s.rate().effective()),
        list_rate: s.list_rate(),
        exchange_rule: s.catalog().exchange_rule.clone(),
        source_file: s.catalog().source_file.clone(),
        list_date: s.catalog().list_date.clone(),
        is_overridden: true,
        zai_key: s.db_query_zai(),
    })
}

#[derive(Serialize)]
pub struct AssemblySummary {
    pub id: Uuid,
    pub name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_valid: bool,
    pub confirmed: bool,
    pub missing_count: usize,
    pub item_count: usize,
    pub quote_type: crate::domain::assembly::QuoteType,
    pub status: crate::domain::assembly::QuoteStatus,
    pub expiry_date: chrono::NaiveDate,
    pub quote_number: Option<String>,
    pub based_on: Option<String>,
}

fn summarize(asm: &Assembly, state: &AppState) -> AssemblySummary {
    use crate::domain::assembly::QuoteStatus;
    let report = crate::domain::assembly::validate(asm, state.catalog());
    let today = chrono::Local::now().date_naive();
    AssemblySummary {
        id: asm.id,
        name: asm.name.clone(),
        updated_at: asm.updated_at,
        is_valid: report.is_valid,
        confirmed: asm.confirmed_at.is_some(),
        missing_count: report
            .issues
            .iter()
            .filter(|i| i.severity == crate::domain::assembly::Severity::Error)
            .count(),
        item_count: asm.slots.values().map(|v| v.len()).sum(),
        quote_type: asm.quote_type,
        status: asm.status(today),
        expiry_date: asm.expiry(),
        quote_number: asm.quote_number.clone(),
        based_on: asm.based_on.clone(),
    }
}

#[tauri::command]
pub fn list_assemblies(state: State<'_, SharedState>) -> CmdResult<Vec<AssemblySummary>> {
    let s = state.0.lock().unwrap();
    Ok(s.assemblies().iter().map(|a| summarize(a, &s)).collect())
}

#[tauri::command]
pub fn create_assembly(
    state: State<'_, SharedState>,
    name: String,
    quote_type: Option<String>,
) -> CmdResult<Assembly> {
    use crate::domain::assembly::QuoteType;
    let mut s = state.0.lock().unwrap();
    let quote_type = match quote_type.as_deref() {
        Some("preensamble") => QuoteType::Preensamble,
        _ => QuoteType::Personalizado,
    };
    Ok(s.create_assembly(name, quote_type))
}

/// Duplica un ensamble (normalmente preensamble) como PERSONALIZADO.
#[tauri::command]
pub fn duplicate_assembly(state: State<'_, SharedState>, id: String) -> CmdResult<Assembly> {
    let mut s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    s.duplicate_assembly(id)
}

/// Marca o desmarca la cotización como "en fabricación".
#[tauri::command]
pub fn set_fabrication(
    state: State<'_, SharedState>,
    id: String,
    in_fabrication: bool,
) -> CmdResult<Assembly> {
    let mut s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    s.set_fabrication(id, in_fabrication)
}

/// Asigna el folio de cotización del día (DDMMAA-NC) y lo liga al ensamble.
#[tauri::command]
pub fn assign_quote_number(state: State<'_, SharedState>, id: String) -> CmdResult<String> {
    let mut s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    s.assign_quote_number(id)
}

#[tauri::command]
pub fn get_assembly(state: State<'_, SharedState>, id: String) -> CmdResult<Assembly> {
    let s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    s.assembly(id).cloned().ok_or_else(|| "Ensamble no encontrado".into())
}

#[tauri::command]
pub fn rename_assembly(
    state: State<'_, SharedState>,
    id: String,
    name: String,
) -> CmdResult<Assembly> {
    let mut s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    s.rename_assembly(id, name).ok_or_else(|| "Ensamble no encontrado".into())
}

#[tauri::command]
pub fn delete_assembly(state: State<'_, SharedState>, id: String) -> CmdResult<bool> {
    let mut s = state.0.lock().unwrap();
    let id = parse_uuid(&id)?;
    Ok(s.delete_assembly(id))
}

#[tauri::command]
pub fn set_assembly_item(
    state: State<'_, SharedState>,
    assembly_id: String,
    slot: String,
    component_id: String,
    quantity: u32,
    supplier: Option<String>,
) -> CmdResult<Assembly> {
    let mut s = state.0.lock().unwrap();
    let assembly_id = parse_uuid(&assembly_id)?;
    let slot = Slot::parse(&slot).ok_or_else(|| format!("Slot desconocido: {slot}"))?;
    let component_id = parse_uuid(&component_id)?;
    s.set_assembly_item(assembly_id, slot, component_id, quantity, supplier)
}

#[tauri::command]
pub fn clear_assembly_slot(
    state: State<'_, SharedState>,
    assembly_id: String,
    slot: String,
) -> CmdResult<Assembly> {
    let mut s = state.0.lock().unwrap();
    let assembly_id = parse_uuid(&assembly_id)?;
    let slot = Slot::parse(&slot).ok_or_else(|| format!("Slot desconocido: {slot}"))?;
    s.clear_assembly_slot(assembly_id, slot)
}

#[tauri::command]
pub fn validate_assembly(
    state: State<'_, SharedState>,
    assembly_id: String,
) -> CmdResult<ValidationReport> {
    let s = state.0.lock().unwrap();
    let assembly_id = parse_uuid(&assembly_id)?;
    s.validate(assembly_id)
        .ok_or_else(|| "Ensamble no encontrado".into())
}

/// Desglose de costo del ensamble: subtotal en pesos, IVA 16% y total.
#[tauri::command]
pub fn assembly_cost(
    state: State<'_, SharedState>,
    assembly_id: String,
) -> CmdResult<crate::domain::cost::CostBreakdown> {
    let s = state.0.lock().unwrap();
    let assembly_id = parse_uuid(&assembly_id)?;
    let asm = s
        .assembly(assembly_id)
        .ok_or_else(|| "Ensamble no encontrado".to_string())?;
    let rate = s.rate();
    Ok(crate::domain::cost::compute_cost(asm, s.catalog(), &rate))
}

/// Asigna y devuelve el número de cotización del día (DDMMAA-NC).
#[tauri::command]
pub fn next_quote_number(state: State<'_, SharedState>) -> CmdResult<String> {
    let mut s = state.0.lock().unwrap();
    Ok(s.next_quote_number())
}

/// Guarda bytes (PNG/PDF) en la ruta dada por el diálogo de guardado.
#[tauri::command]
pub fn save_export(
    state: State<'_, SharedState>,
    path: String,
    bytes_base64: String,
) -> CmdResult<String> {
    use base64::Engine as _;
    let s = state.0.lock().unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64.as_bytes())
        .map_err(|e| format!("Export inválido: {e}"))?;
    s.save_export(&path, &bytes)?;
    Ok(path)
}

/// Importa un catálogo JSON (formato catalog_seed.json) con UPSERT por
/// SKU: actualiza los existentes (conservando id) e inserta los nuevos.
#[tauri::command]
pub fn import_catalog(
    state: State<'_, SharedState>,
    path: String,
) -> CmdResult<ImportSummary> {
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("No se pudo leer {path}: {e}"))?;
    let mut s = state.0.lock().unwrap();
    let (updated, inserted) = s.import_catalog(&json)?;
    Ok(ImportSummary {
        updated,
        inserted,
        total: s.catalog().components.len(),
    })
}

#[derive(Serialize)]
pub struct ImportSummary {
    pub updated: usize,
    pub inserted: usize,
    pub total: usize,
}

/// Importa un Excel de lista de precios (cualquier proveedor registrado)
/// con UPSERT por SKU.
#[derive(Serialize)]
pub struct XlsxImportSummary {
    pub updated: usize,
    pub inserted: usize,
    pub total: usize,
    pub supplier: String,
}

#[tauri::command]
pub fn import_catalog_xlsx(
    state: State<'_, SharedState>,
    path: String,
) -> CmdResult<XlsxImportSummary> {
    let mut s = state.0.lock().unwrap();
    let (updated, inserted, supplier) = s.import_xlsx_catalog(&path)?;
    Ok(XlsxImportSummary {
        updated,
        inserted,
        total: s.catalog().components.len(),
        supplier,
    })
}

/// Listas de marcas y departamentos para los filtros de la interfaz.
#[derive(Serialize)]
pub struct CatalogFilters {
    pub brands: Vec<String>,
    pub departments: Vec<String>,
}

#[tauri::command]
pub fn catalog_filters(state: State<'_, SharedState>) -> CmdResult<CatalogFilters> {
    let s = state.0.lock().unwrap();
    Ok(CatalogFilters {
        brands: crate::db::list_brands(&s.db()).unwrap_or_default(),
        departments: crate::db::list_departments(&s.db()).unwrap_or_default(),
    })
}

/// Guarda/lee la clave API de Z.ai (para el formateo asistido de títulos).
#[tauri::command]
pub fn set_zai_key(state: State<'_, SharedState>, key: String) -> CmdResult<()> {
    let s = state.0.lock().unwrap();
    crate::db::set_setting(&s.db(), "zai_api_key", &key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_zai_key(state: State<'_, SharedState>) -> CmdResult<String> {
    let s = state.0.lock().unwrap();
    Ok(s.db_query_zai())
}

/// Normaliza los títulos de los productos con Z.ai (usa la clave guardada).
/// Procesa en lotes; devuelve cuántos títulos se mejoraron.
#[derive(Serialize)]
pub struct AiTitlesSummary {
    pub improved: usize,
    pub remaining: usize,
}

#[tauri::command]
pub fn ai_normalize_titles(
    state: State<'_, SharedState>,
    limit: u32,
) -> CmdResult<AiTitlesSummary> {
    let mut s = state.0.lock().unwrap();
    let (improved, remaining) = s.ai_normalize_titles(limit)?;
    Ok(AiTitlesSummary { improved, remaining })
}

/// Valores de un catálogo de atributo normalizado (socket, ddr,
/// form_factor, ram_form).
#[tauri::command]
pub fn list_spec_options(
    state: State<'_, SharedState>,
    kind: String,
) -> CmdResult<Vec<String>> {
    let s = state.0.lock().unwrap();
    Ok(crate::db::list_spec_options(&s.db(), &kind).unwrap_or_default())
}

/// Agrega un valor nuevo a un catálogo de atributo. Solo tipos conocidos.
#[tauri::command]
pub fn add_spec_option(
    state: State<'_, SharedState>,
    kind: String,
    value: String,
) -> CmdResult<bool> {
    const KINDS: &[&str] = &["socket", "ddr", "form_factor", "ram_form"];
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("Catálogo desconocido: {kind}"));
    }
    let value = value.trim().to_uppercase();
    if value.is_empty() {
        return Err("El valor no puede estar vacío".into());
    }
    let s = state.0.lock().unwrap();
    crate::db::add_spec_option(&s.db(), &kind, &value).map_err(|e| e.to_string())
}

/// Agrega una marca nueva al catálogo de marcas.
#[tauri::command]
pub fn add_brand(state: State<'_, SharedState>, name: String) -> CmdResult<()> {
    let s = state.0.lock().unwrap();
    crate::db::add_brand(&s.db(), name.trim()).map_err(|e| e.to_string())
}

/// Consulta principal de productos: por SKU (único).
#[tauri::command]
pub fn get_component_by_sku(
    state: State<'_, SharedState>,
    sku: String,
) -> CmdResult<Option<ComponentDto>> {
    let s = state.0.lock().unwrap();
    Ok(s.find_by_sku(&sku).map(|c| dto(&c, &s.rate())))
}

/// Asigna el número de nota de venta del día (DDMMAAAA-NC).
#[tauri::command]
pub fn next_sale_note_number(state: State<'_, SharedState>) -> CmdResult<String> {
    let mut s = state.0.lock().unwrap();
    Ok(s.next_sale_note_number())
}

/// Registra la nota de venta emitida con sus números de serie. El ensamble
/// es opcional: NULL en ventas individuales.
#[tauri::command]
pub fn record_sale_note(
    state: State<'_, SharedState>,
    number: String,
    quote_number: Option<String>,
    assembly_id: Option<String>,
    lines: Vec<crate::domain::sale::SaleNoteLine>,
) -> CmdResult<crate::domain::sale::SaleNoteRecord> {
    use crate::domain::sale::SaleNoteRecord;
    let mut s = state.0.lock().unwrap();
    if lines.is_empty() {
        return Err("La nota de venta no tiene renglones".into());
    }
    let assembly_id = match assembly_id {
        Some(id) if !id.is_empty() => Some(parse_uuid(&id)?),
        _ => None,
    };
    let record = SaleNoteRecord {
        number,
        quote_number,
        assembly_id,
        created_at: chrono::Utc::now(),
        lines,
    };
    s.record_sale_note(record)
}

/// Ganancia del carrito de venta individual: mismas reglas que ensambles
/// (presets, IVA recalculado con el objetivo).
#[tauri::command]
pub fn cart_profit(
    state: State<'_, SharedState>,
    items: Vec<crate::domain::cart::CartItem>,
    percent: String,
    target: Option<String>,
) -> CmdResult<crate::domain::profit::ProfitReport> {
    let s = state.0.lock().unwrap();
    let percent = rust_decimal::Decimal::from_str_exact(&percent)
        .map_err(|_| format!("Porcentaje inválido: {percent}"))?;
    if percent < rust_decimal::Decimal::ZERO {
        return Err("El porcentaje de ganancia no puede ser negativo".into());
    }
    let rate = s.rate();
    let mut report = crate::domain::cart::compute_cart_profit(
        &items,
        &|id| s.find_component(id),
        &rate,
        percent,
    );
    if let Some(t) = target {
        let t = rust_decimal::Decimal::from_str_exact(&t)
            .map_err(|_| format!("Precio objetivo inválido: {t}"))?;
        if t < rust_decimal::Decimal::ZERO {
            return Err("El precio objetivo no puede ser negativo".into());
        }
        crate::domain::profit::apply_target(&mut report, t);
    }
    Ok(report)
}

/// Lista las notas de venta emitidas.
#[tauri::command]
pub fn list_sale_notes(state: State<'_, SharedState>) -> CmdResult<Vec<crate::domain::sale::SaleNoteRecord>> {
    let s = state.0.lock().unwrap();
    Ok(s.sale_notes().to_vec())
}

/// Confirma el ensamble (debe estar completo) y lo guarda como cotización.
#[tauri::command]
pub fn confirm_assembly(
    state: State<'_, SharedState>,
    assembly_id: String,
) -> CmdResult<Assembly> {
    let mut s = state.0.lock().unwrap();
    let assembly_id = parse_uuid(&assembly_id)?;
    s.confirm_assembly(assembly_id)
}

/// Tabla de ganancias por componente con el porcentaje dado y, opcionalmente,
/// un precio objetivo (total con IVA) que ajusta la ganancia en partes iguales.
#[tauri::command]
pub fn assembly_profit(
    state: State<'_, SharedState>,
    assembly_id: String,
    percent: String,
    target: Option<String>,
) -> CmdResult<crate::domain::profit::ProfitReport> {
    let s = state.0.lock().unwrap();
    let assembly_id = parse_uuid(&assembly_id)?;
    let asm = s
        .assembly(assembly_id)
        .ok_or_else(|| "Ensamble no encontrado".to_string())?;
    let rate = s.rate();

    let percent = rust_decimal::Decimal::from_str_exact(&percent)
        .map_err(|_| format!("Porcentaje inválido: {percent}"))?;
    if percent < rust_decimal::Decimal::ZERO {
        return Err("El porcentaje de ganancia no puede ser negativo".into());
    }

    let mut report = crate::domain::profit::compute_profit(asm, s.catalog(), &rate, percent);
    if let Some(t) = target {
        let t = rust_decimal::Decimal::from_str_exact(&t)
            .map_err(|_| format!("Precio objetivo inválido: {t}"))?;
        if t < rust_decimal::Decimal::ZERO {
            return Err("El precio objetivo no puede ser negativo".into());
        }
        crate::domain::profit::apply_target(&mut report, t);
    }
    Ok(report)
}

/// Convierte un monto con la regla de la casa (T.C redondeado hacia arriba).
#[tauri::command]
pub fn convert_money(
    state: State<'_, SharedState>,
    amount: String,
    currency: String,
) -> CmdResult<Money> {
    let s = state.0.lock().unwrap();
    let amount = rust_decimal::Decimal::from_str_exact(&amount)
        .map_err(|_| format!("Monto inválido: {amount}"))?;
    let currency = match currency.to_uppercase().as_str() {
        "USD" => Currency::Usd,
        "MXP" | "MXN" => Currency::Mxp,
        other => return Err(format!("Moneda no soportada: {other}")),
    };
    Ok(Money::new(amount, currency).to_mxp(&s.rate()))
}
