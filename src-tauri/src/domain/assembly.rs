//! Ensamble de computadora: slots, partidas y reglas de validación.
//!
//! Requisitos mínimos por ensamble:
//! - 1 procesador, 1 memoria RAM, 1 dispositivo de almacenamiento (SSD/HDD),
//!   1 tarjeta madre y 1 gabinete.
//! - Fuente de poder: obligatoria, salvo que el gabinete seleccionado ya
//!   incluya una (el campo se llena con la especificación de la fuente
//!   incluida o simplemente se descarta).
//! - Enfriamiento (aire o líquido): obligatorio salvo que el procesador ya
//!   incluya disipador.
//! - Monitores, teclados, mouse, GPUs y tarjetas de expansión: opcionales.

use chrono::{Datelike, DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use super::component::{Component, ComponentKind};

/// Slots del ensamble. Cada slot acepta ciertos tipos de componente
/// (ver [`Slot::accepts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    Cpu,
    Motherboard,
    Ram,
    Storage,
    Psu,
    Case,
    Cooling,
    Gpu,
    Monitor,
    Keyboard,
    Mouse,
    ExpansionCard,
}

impl Slot {
    pub fn label(&self) -> &'static str {
        match self {
            Slot::Cpu => "Procesador",
            Slot::Motherboard => "Tarjeta madre",
            Slot::Ram => "Memoria RAM",
            Slot::Storage => "Almacenamiento",
            Slot::Psu => "Fuente de poder",
            Slot::Case => "Gabinete",
            Slot::Cooling => "Enfriamiento",
            Slot::Gpu => "Tarjeta de video",
            Slot::Monitor => "Monitor",
            Slot::Keyboard => "Teclado",
            Slot::Mouse => "Mouse",
            Slot::ExpansionCard => "Tarjeta de expansión",
        }
    }

    /// Indica si el slot acepta más de una unidad.
    pub fn multiple_allowed(&self) -> bool {
        matches!(
            self,
            Slot::Ram
                | Slot::Storage
                | Slot::Cooling
                | Slot::Gpu
                | Slot::Monitor
                | Slot::ExpansionCard
        )
    }

    /// Tipos de componente que encajan en el slot.
    pub fn accepts(&self, kind: ComponentKind) -> bool {
        match self {
            Slot::Cpu => kind == ComponentKind::Cpu,
            Slot::Motherboard => kind == ComponentKind::Motherboard,
            Slot::Ram => kind == ComponentKind::Ram,
            Slot::Storage => kind == ComponentKind::Storage,
            Slot::Psu => kind == ComponentKind::Psu,
            Slot::Case => kind == ComponentKind::Case,
            Slot::Cooling => kind.is_cooler(),
            Slot::Gpu => kind == ComponentKind::Gpu,
            Slot::Monitor => kind == ComponentKind::Monitor,
            // Un kit teclado+mouse puede cubrir cualquiera de los dos slots.
            Slot::Keyboard => matches!(
                kind,
                ComponentKind::Keyboard | ComponentKind::KeyboardMouseKit
            ),
            Slot::Mouse => matches!(kind, ComponentKind::Mouse | ComponentKind::KeyboardMouseKit),
            Slot::ExpansionCard => kind == ComponentKind::ExpansionCard,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }

    pub const ALL: [Slot; 12] = [
        Slot::Cpu,
        Slot::Motherboard,
        Slot::Ram,
        Slot::Storage,
        Slot::Psu,
        Slot::Case,
        Slot::Cooling,
        Slot::Gpu,
        Slot::Monitor,
        Slot::Keyboard,
        Slot::Mouse,
        Slot::ExpansionCard,
    ];
}

/// Una partida del ensamble: componente + cantidad + proveedor elegido.
/// `supplier` = None usa la mejor oferta (menor precio disponible);
/// Some(folio) fija la compra a un proveedor concreto aunque no sea el
/// más barato (opción de "cambiar de proveedor" en el picker).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssemblyItem {
    pub component_id: Uuid,
    pub quantity: u32,
    #[serde(default)]
    pub supplier: Option<String>,
}

/// Ensamble (presupuesto de una computadora).
/// Tipo de cotización. Las preensambladas caducan el día 1 del mes
/// siguiente (salvo que se generen el último día del mes: entonces tienen
/// 15 días naturales); las personalizadas, 15 días naturales después de
/// generadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteType {
    Preensamble,
    Personalizado,
}

impl QuoteType {
    pub fn label(&self) -> &'static str {
        match self {
            QuoteType::Preensamble => "Preensamble",
            QuoteType::Personalizado => "Personalizado",
        }
    }
}

/// Estado de la cotización. "Caducada" se deriva de las fechas; "En
/// fabricación" lo marca el negocio y BLOQUEA la caducidad (una cotización
/// en fabricación siempre puede generar nota de venta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStatus {
    Vigente,
    EnFabricacion,
    Caducada,
}

impl QuoteStatus {
    pub fn label(&self) -> &'static str {
        match self {
            QuoteStatus::Vigente => "Vigente",
            QuoteStatus::EnFabricacion => "En fabricación",
            QuoteStatus::Caducada => "Caducada",
        }
    }
}

/// Días naturales de vigencia de personalizadas y preensambladas generadas
/// el último día del mes.
pub const VIGENCIA_DIAS: chrono::Duration = chrono::Duration::days(15);

/// Fecha de caducidad de una cotización según su tipo y fecha de creación.
pub fn expiry_date(quote_type: QuoteType, created: chrono::NaiveDate) -> chrono::NaiveDate {
    match quote_type {
        QuoteType::Personalizado => created + VIGENCIA_DIAS,
        QuoteType::Preensamble => {
            let first_of_next = if created.month() == 12 {
                chrono::NaiveDate::from_ymd_opt(created.year() + 1, 1, 1)
            } else {
                chrono::NaiveDate::from_ymd_opt(created.year(), created.month() + 1, 1)
            }
            .expect("fecha válida");
            // Generada el último día del mes: los mismos 15 días naturales
            // (caducar el día 1 dejaría menos de un día de vigencia).
            if (first_of_next - created) == chrono::Duration::days(1) {
                created + VIGENCIA_DIAS
            } else {
                first_of_next
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assembly {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Fecha de confirmación: al confirmar, el ensamble se guarda como
    /// cotización lista para calcular ganancias.
    #[serde(default)]
    pub confirmed_at: Option<DateTime<Utc>>,
    /// Tipo de cotización (preensamble o personalizado).
    #[serde(default = "default_quote_type")]
    pub quote_type: QuoteType,
    /// Marcado por el negocio: en fabricación (impide caducar).
    #[serde(default)]
    pub in_fabrication: bool,
    /// Folio de cotización asignado (DDMMAA-NC), ligado al ensamble.
    #[serde(default)]
    pub quote_number: Option<String>,
    /// Origen de un personalizado creado desde un preensamble
    /// (p. ej. "#151026-1 Preensamble Gamer").
    #[serde(default)]
    pub based_on: Option<String>,
    pub slots: BTreeMap<Slot, Vec<AssemblyItem>>,
}

fn default_quote_type() -> QuoteType {
    QuoteType::Personalizado
}

impl Assembly {
    pub fn new(name: impl Into<String>, quote_type: QuoteType) -> Self {
        let mut slots = BTreeMap::new();
        for slot in Slot::ALL {
            slots.insert(slot, Vec::new());
        }
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            created_at: now,
            updated_at: now,
            confirmed_at: None,
            quote_type,
            in_fabrication: false,
            quote_number: None,
            based_on: None,
            slots,
        }
    }

    /// Fecha de caducidad según el tipo y la fecha de creación.
    pub fn expiry(&self) -> chrono::NaiveDate {
        expiry_date(self.quote_type, self.created_at.date_naive())
    }

    /// Estado efectivo a la fecha dada: "En fabricación" nunca caduca.
    pub fn status(&self, today: chrono::NaiveDate) -> QuoteStatus {
        if self.in_fabrication {
            return QuoteStatus::EnFabricacion;
        }
        if today >= self.expiry() {
            QuoteStatus::Caducada
        } else {
            QuoteStatus::Vigente
        }
    }

    /// Puede generar nota de venta: todo estado salvo caducada.
    pub fn can_issue_sale_note(&self, today: chrono::NaiveDate) -> bool {
        self.status(today) != QuoteStatus::Caducada
    }

    /// Fija la cantidad de un componente en un slot. Cantidad 0 elimina la
    /// partida. Rechaza tipos que no correspondan al slot y memorias SODIMM
    /// (son para laptop, no para ensambles de escritorio).
    pub fn set_quantity(
        &mut self,
        slot: Slot,
        component: &Component,
        quantity: u32,
        supplier: Option<String>,
    ) -> Result<(), AssemblyError> {
        if is_laptop_ram(component) {
            return Err(AssemblyError::SodimmNotForDesktop);
        }
        if !slot.accepts(component.kind) {
            return Err(AssemblyError::KindDoesNotFitSlot {
                slot: slot.label().to_string(),
                kind: component.kind.label().to_string(),
            });
        }
        let component_id = component.id;
        let items = self.slots.entry(slot).or_default();
        items.retain(|i| i.component_id != component_id);
        if quantity > 0 {
            items.push(AssemblyItem {
                component_id,
                quantity,
                supplier,
            });
        }
        self.touch();
        Ok(())
    }

    pub fn clear_slot(&mut self, slot: Slot) {
        if let Some(items) = self.slots.get_mut(&slot) {
            items.clear();
        }
        self.touch();
    }

    pub fn items(&self, slot: Slot) -> &[AssemblyItem] {
        self.slots.get(&slot).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Cantidad total de unidades en el slot.
    pub fn total_quantity(&self, slot: Slot) -> u32 {
        self.items(slot).iter().map(|i| i.quantity).sum()
    }

    /// Indica si el slot está cubierto (al menos 1 unidad).
    pub fn is_filled(&self, slot: Slot) -> bool {
        self.total_quantity(slot) > 0
    }

    fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssemblyError {
    #[error("Un componente de tipo {kind} no puede ir en el slot {slot}")]
    KindDoesNotFitSlot { slot: String, kind: String },
    #[error("Las memorias SODIMM son para laptop y no pueden ir en un ensamble de escritorio")]
    SodimmNotForDesktop,
}

/// Las RAM SODIMM son para laptop: se excluyen de los ensambles.
pub fn is_laptop_ram(c: &Component) -> bool {
    c.kind == ComponentKind::Ram && c.specs.ram_form.as_deref() == Some("SODIMM")
}

// ---------------------------------------------------------------------------
// Validación
// ---------------------------------------------------------------------------

/// Acceso a componentes por id sin acoplar el dominio al catálogo concreto.
pub trait ComponentLookup {
    fn get_component(&self, id: &Uuid) -> Option<&Component>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
}

impl ValidationIssue {
    fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn warning(code: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn info(code: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// Cómo está cubierta la fuente de poder del ensamble.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PsuCoverage {
    /// Fuente dedicada seleccionada.
    Dedicated,
    /// Cubierta por la fuente incluida en el gabinete.
    IncludedInCase { detail: String },
    /// Sin fuente: el ensamble no es válido.
    Missing,
}

/// Estado del enfriamiento del ensamble.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoolingCoverage {
    /// Los procesadores traen disipador; enfriamiento adicional opcional.
    IncludedInCpu { detail: String },
    /// Cubierto por enfriadores de aire o líquidos seleccionados.
    Provided { detail: String },
    /// Se requiere enfriamiento y no hay ninguno.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// `true` si no hay ningún issue con severidad Error.
    pub is_valid: bool,
    pub issues: Vec<ValidationIssue>,
    pub psu: PsuCoverage,
    pub cooling: CoolingCoverage,
}

/// Valida el ensamble contra las reglas de negocio.
pub fn validate<L: ComponentLookup>(assembly: &Assembly, catalog: &L) -> ValidationReport {
    let mut issues = Vec::new();

    // --- Coherencia slot/tipo y componentes existentes ---
    for &slot in Slot::ALL.iter() {
        for item in assembly.items(slot) {
            match catalog.get_component(&item.component_id) {
                None => issues.push(ValidationIssue::error(
                    "unknown_component",
                    format!("El slot {} referencia un componente que ya no existe en el catálogo.", slot.label()),
                )),
                Some(c) if !slot.accepts(c.kind) => issues.push(ValidationIssue::error(
                    "wrong_kind",
                    format!(
                        "\"{}\" es de tipo {} y no puede ir en {}.",
                        short(&c.description),
                        c.kind.label(),
                        slot.label()
                    ),
                )),
                Some(c) if is_laptop_ram(c) => issues.push(ValidationIssue::error(
                    "sodimm_not_supported",
                    format!(
                        "\"{}\" es una memoria SODIMM (para laptop) y no puede ir en un ensamble de escritorio.",
                        short(&c.description)
                    ),
                )),
                Some(c) if c.condition == super::component::Condition::Usado => issues.push(
                    ValidationIssue::info(
                        "used_component",
                        format!("\"{}\" es un componente usado.", short(&c.description)),
                    ),
                ),
                Some(_) => {}
            }
        }
        // Slots de unidad única (CPU, tarjeta madre, gabinete, fuente).
        if !slot.multiple_allowed() && assembly.total_quantity(slot) > 1 {
            issues.push(ValidationIssue::warning(
                "multiple_single_slot",
                format!(
                    "El slot {} debería tener una sola unidad (tiene {}).",
                    slot.label(),
                    assembly.total_quantity(slot)
                ),
            ));
        }
    }

    // --- Mínimos obligatorios ---
    let minimums: [(Slot, &str); 5] = [
        (Slot::Cpu, "Falta el procesador."),
        (Slot::Motherboard, "Falta la tarjeta madre."),
        (Slot::Ram, "Falta la memoria RAM."),
        (Slot::Storage, "Falta el dispositivo de almacenamiento (SSD o HDD)."),
        (Slot::Case, "Falta el gabinete."),
    ];
    for (slot, msg) in minimums {
        if !assembly.is_filled(slot) {
            issues.push(ValidationIssue::error(format!("missing_{}", slot_key(slot)).as_str(), msg));
        }
    }

    // --- Compatibilidad CPU/MOBO/RAM/Gabinete ---
    validate_compatibility(assembly, catalog, &mut issues);

    // --- Fuente de poder: dedicada o incluida en gabinete ---
    let psu = psu_coverage(assembly, catalog, &mut issues);

    // --- Enfriamiento según disipador del procesador ---
    let cooling = cooling_coverage(assembly, catalog, &mut issues);

    let is_valid = !issues.iter().any(|i| i.severity == Severity::Error);
    ValidationReport {
        is_valid,
        issues,
        psu,
        cooling,
    }
}

/// Reglas de compatibilidad. Si faltan datos para comparar el gabinete se
/// emite una ADVERTENCIA de revisión manual; los enfrentamientos claros
/// (socket, DDR) son errores.
fn validate_compatibility<L: ComponentLookup>(
    assembly: &Assembly,
    catalog: &L,
    issues: &mut Vec<ValidationIssue>,
) {
    let first = |slot: Slot| -> Option<Component> {
        assembly
            .items(slot)
            .first()
            .and_then(|i| catalog.get_component(&i.component_id))
            .cloned()
    };
    let cpu = first(Slot::Cpu);
    let mobo = first(Slot::Motherboard);
    let case = first(Slot::Case);

    // 1) Socket de procesador vs tarjeta madre
    if let (Some(cpu), Some(mobo)) = (&cpu, &mobo) {
        match (&cpu.specs.socket, &mobo.specs.socket) {
            (Some(cs), Some(ms)) if cs != ms => issues.push(ValidationIssue::error(
                "socket_mismatch",
                format!(
                    "Socket incompatible: el procesador es {cs} y la tarjeta madre es {ms}."
                ),
            )),
            _ => {}
        }
    }

    // 2) DDR de la RAM vs tarjeta madre
    let rams: Vec<Component> = assembly
        .items(Slot::Ram)
        .iter()
        .filter_map(|i| catalog.get_component(&i.component_id))
        .cloned()
        .collect();
    if let Some(mobo) = &mobo {
        for ram in &rams {
            match (&ram.specs.ddr, &mobo.specs.ddr) {
                (Some(rd), Some(md)) if rd != md => issues.push(ValidationIssue::error(
                    "ddr_mismatch",
                    format!(
                        "Memoria incompatible: la RAM es {rd} y la tarjeta madre soporta {md}."
                    ),
                )),
                _ => {}
            }
        }
    }

    // 3) Factor de forma gabinete vs tarjeta madre. Sin datos: advertencia
    //    de revisión manual (no se puede afirmar incompatibilidad).
    if let (Some(case), Some(mobo)) = (&case, &mobo) {
        let case_ff = case.specs.form_factor.clone();
        let mobo_ff = mobo.specs.form_factor.clone();
        match (&case_ff, &mobo_ff) {
            (Some(cf), Some(mf)) => {
                let fits = match cf.as_str() {
                    "ATX" => true, // el gabinete ATX admite las tres placas
                    "MICRO ATX" => mf != "ATX",
                    "MINI ITX" => mf == "MINI ITX",
                    _ => true,
                };
                if !fits {
                    issues.push(ValidationIssue::error(
                        "form_factor_mismatch",
                        format!(
                            "El gabinete {cf} probablemente no admite una tarjeta madre {mf}: validar medidas."
                        ),
                    ));
                }
            }
            _ => issues.push(ValidationIssue::warning(
                "case_compatibility_unknown",
                format!(
                    "No hay datos de factor de forma del gabinete o de la tarjeta madre: valida manualmente que la placa quepa en el gabinete."
                ),
            )),
        }
    }
}

fn psu_coverage<L: ComponentLookup>(
    assembly: &Assembly,
    catalog: &L,
    issues: &mut Vec<ValidationIssue>,
) -> PsuCoverage {
    if assembly.is_filled(Slot::Psu) {
        // ¿Algún gabinete con fuente incluida? La dedicada la descarta.
        if let Some(spec) = case_included_psu(assembly, catalog) {
            issues.push(ValidationIssue::info(
                "psu_discarded",
                format!(
                    "El gabinete incluye fuente ({spec}); al elegir una dedicada, la incluida se descarta."
                ),
            ));
        }
        return PsuCoverage::Dedicated;
    }

    if let Some(spec) = case_included_psu(assembly, catalog) {
        issues.push(ValidationIssue::info(
            "psu_from_case",
            format!("La fuente de poder queda cubierta por la del gabinete: {spec}."),
        ));
        return PsuCoverage::IncludedInCase { detail: spec };
    }

    issues.push(ValidationIssue::error(
        "missing_psu",
        "Falta la fuente de poder: agrega una dedicada o elige un gabinete que la incluya.",
    ));
    PsuCoverage::Missing
}

fn case_included_psu<L: ComponentLookup>(assembly: &Assembly, catalog: &L) -> Option<String> {
    assembly
        .items(Slot::Case)
        .iter()
        .filter_map(|item| catalog.get_component(&item.component_id))
        .find_map(|c| c.included_psu_spec().map(|s| format!("{} — {s}", short(&c.description))))
}

fn cooling_coverage<L: ComponentLookup>(
    assembly: &Assembly,
    catalog: &L,
    issues: &mut Vec<ValidationIssue>,
) -> CoolingCoverage {
    let cpus: Vec<&Component> = assembly
        .items(Slot::Cpu)
        .iter()
        .filter_map(|i| catalog.get_component(&i.component_id))
        .filter(|c| c.kind == ComponentKind::Cpu)
        .collect();

    // ¿Todos los procesadores traen disipador? (None se trata como "no trae"
    // para no presupuestar un equipo sin enfriamiento.)
    let all_include_cooler = !cpus.is_empty()
        && cpus
            .iter()
            .all(|c| c.specs.includes_cooler == Some(true));

    let has_cooling = assembly.is_filled(Slot::Cooling);

    if all_include_cooler {
        if has_cooling {
            issues.push(ValidationIssue::info(
                "cooling_optional",
                "El procesador ya incluye disipador; el enfriamiento extra es opcional.",
            ));
        }
        let detail = cpus
            .iter()
            .map(|c| short(&c.description))
            .collect::<Vec<_>>()
            .join(", ");
        return CoolingCoverage::IncludedInCpu { detail };
    }

    if has_cooling {
        let detail = assembly
            .items(Slot::Cooling)
            .iter()
            .filter_map(|i| catalog.get_component(&i.component_id))
            .map(|c| short(&c.description))
            .collect::<Vec<_>>()
            .join(", ");
        return CoolingCoverage::Provided { detail };
    }

    issues.push(ValidationIssue::error(
        "missing_cooling",
        "El procesador no incluye disipador: agrega enfriamiento por aire o líquido.",
    ));
    CoolingCoverage::Missing
}

fn slot_key(slot: Slot) -> &'static str {
    match slot {
        Slot::Cpu => "cpu",
        Slot::Motherboard => "motherboard",
        Slot::Ram => "ram",
        Slot::Storage => "storage",
        Slot::Psu => "psu",
        Slot::Case => "case",
        Slot::Cooling => "cooling",
        Slot::Gpu => "gpu",
        Slot::Monitor => "monitor",
        Slot::Keyboard => "keyboard",
        Slot::Mouse => "mouse",
        Slot::ExpansionCard => "expansion_card",
    }
}

fn short(desc: &str) -> String {
    const MAX: usize = 60;
    if desc.len() <= MAX {
        desc.to_string()
    } else {
        let cut: String = desc.chars().take(MAX).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::component::{
        Component, ComponentKind, ComponentSource, Condition, Specs,
    };
    use crate::domain::money::{Currency, ExchangeRate, Money};
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    fn comp(id: Uuid, kind: ComponentKind, specs: Specs) -> Component {
        Component {
            id,
            sku: format!("SKU-{id}"),
            barcode: None,
            description: format!("COMPONENTE {kind:?} {id}"),
            brand: "TEST".into(),
            department: "TEST".into(),
            kind,
            condition: Condition::Nuevo,
            price: Money::new(dec!(100), Currency::Usd),
            on_promotion: false,
            offer_stocks: BTreeMap::new(),
            source: ComponentSource::Lista,
            supplier: "arroba".into(),
            stock_ensamble_total: 20,
            stock_individual_total: 20,
            cd_only_best: false,
            offers: vec![],
            specs,
        }
    }

    #[derive(Default)]
    struct FakeCatalog {
        map: HashMap<Uuid, Component>,
    }

    impl ComponentLookup for FakeCatalog {
        fn get_component(&self, id: &Uuid) -> Option<&Component> {
            self.map.get(id)
        }
    }

    fn full_setup(cooler_in_cpu: Option<bool>, case_includes_psu: bool) -> (FakeCatalog, Assembly) {
        let mut cat = FakeCatalog::default();
        let mut asm = Assembly::new("Equipo de prueba", QuoteType::Personalizado);

        let entries: Vec<(Slot, ComponentKind, Specs)> = vec![
            (Slot::Cpu, ComponentKind::Cpu, Specs { includes_cooler: cooler_in_cpu, ..Default::default() }),
            (Slot::Motherboard, ComponentKind::Motherboard, Specs::default()),
            (Slot::Ram, ComponentKind::Ram, Specs::default()),
            (Slot::Storage, ComponentKind::Storage, Specs::default()),
            (Slot::Case, ComponentKind::Case, Specs { includes_psu: Some(case_includes_psu), psu_watts: if case_includes_psu { Some(600) } else { None }, ..Default::default() }),
        ];
        for (slot, kind, specs) in entries {
            let c = comp(Uuid::new_v4(), kind, specs);
            asm.set_quantity(slot, &c, 1, None).unwrap();
            cat.map.insert(c.id, c);
        }
        (cat, asm)
    }

    #[test]
    fn empty_assembly_reports_all_missing_minimums() {
        let asm = Assembly::new("Vacío", QuoteType::Personalizado);
        let cat = FakeCatalog::default();
        let report = validate(&asm, &cat);
        assert!(!report.is_valid);
        for code in [
            "missing_cpu",
            "missing_motherboard",
            "missing_ram",
            "missing_storage",
            "missing_case",
            "missing_psu",
            "missing_cooling",
        ] {
            assert!(
                report.issues.iter().any(|i| i.code == code),
                "falta el código {code}"
            );
        }
    }

    #[test]
    fn complete_build_with_dedicated_psu_and_cpu_box_is_valid() {
        let (mut cat, mut asm) = full_setup(Some(true), false);
        // Fuente dedicada
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.map.insert(psu.id, psu);
        let report = validate(&asm, &cat);
        assert!(report.is_valid, "issues: {:?}", report.issues);
        assert_eq!(report.psu, PsuCoverage::Dedicated);
        assert!(matches!(report.cooling, CoolingCoverage::IncludedInCpu { .. }));
    }

    #[test]
    fn case_with_included_psu_covers_the_psu_slot() {
        // CPU sin disipador + enfriamiento; sin PSU dedicada pero el gabinete trae una.
        let (mut cat, mut asm) = full_setup(Some(false), true);
        let cooler = comp(Uuid::new_v4(), ComponentKind::CoolerAir, Specs::default());
        asm.set_quantity(Slot::Cooling, &cooler, 1, None)
            .unwrap();
        cat.map.insert(cooler.id, cooler);

        let report = validate(&asm, &cat);
        assert!(report.is_valid, "issues: {:?}", report.issues);
        match &report.psu {
            PsuCoverage::IncludedInCase { detail } => {
                assert!(detail.contains("600W"), "detalle: {detail}");
            }
            other => panic!("se esperaba IncludedInCase, vino {other:?}"),
        }
    }

    #[test]
    fn dedicated_psu_discards_included_one_with_info() {
        let (mut cat, mut asm) = full_setup(Some(true), true);
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.map.insert(psu.id, psu);
        let report = validate(&asm, &cat);
        assert!(report.is_valid);
        assert!(report.issues.iter().any(|i| i.code == "psu_discarded" && i.severity == Severity::Info));
    }

    #[test]
    fn cpu_without_cooler_requires_cooling() {
        let (mut cat, mut asm) = full_setup(Some(false), false);
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.map.insert(psu.id, psu);

        let report = validate(&asm, &cat);
        assert!(!report.is_valid);
        assert!(report.issues.iter().any(|i| i.code == "missing_cooling"));

        // Con enfriamiento líquido ya es válido
        let cooler = comp(Uuid::new_v4(), ComponentKind::CoolerLiquid, Specs::default());
        asm.set_quantity(Slot::Cooling, &cooler, 1, None)
            .unwrap();
        cat.map.insert(cooler.id, cooler);
        let report = validate(&asm, &cat);
        assert!(report.is_valid, "issues: {:?}", report.issues);
        assert!(matches!(report.cooling, CoolingCoverage::Provided { .. }));
    }

    #[test]
    fn unknown_cpu_cooler_spec_treated_as_no_cooler() {
        // includes_cooler = None → exige enfriamiento (caso conservador)
        let (mut cat, mut asm) = full_setup(None, false);
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.map.insert(psu.id, psu);
        let report = validate(&asm, &cat);
        assert!(report.issues.iter().any(|i| i.code == "missing_cooling"));
    }

    #[test]
    fn wrong_kind_in_slot_is_rejected() {
        let mut asm = Assembly::new("Tipos cruzados", QuoteType::Personalizado);
        let ram = comp(Uuid::new_v4(), ComponentKind::Ram, Specs::default());
        let err = asm.set_quantity(Slot::Cpu, &ram, 1, None);
        assert!(err.is_err());
    }

    #[test]
    fn sodimm_ram_is_rejected_as_laptop_memory() {
        let mut asm = Assembly::new("Con SODIMM", QuoteType::Personalizado);
        let sodimm = comp(
            Uuid::new_v4(),
            ComponentKind::Ram,
            Specs { ram_form: Some("SODIMM".into()), ..Default::default() },
        );
        // El alta directa la rechaza
        let err = asm.set_quantity(Slot::Ram, &sodimm, 1, None).unwrap_err();
        assert!(matches!(err, AssemblyError::SodimmNotForDesktop));
        // Y si ya quedó persistida en un ensamble, la validación la marca
        let mut cat = FakeCatalog::default();
        let items = vec![super::AssemblyItem { component_id: sodimm.id, quantity: 1, supplier: None }];
        asm.slots.insert(Slot::Ram, items);
        cat.map.insert(sodimm.id, sodimm);
        let report = validate(&asm, &cat);
        assert!(report.issues.iter().any(|i| i.code == "sodimm_not_supported"));

        // Las DIMM de escritorio siguen permitidas
        let dimm = comp(Uuid::new_v4(), ComponentKind::Ram, Specs { ram_form: Some("DIMM".into()), ..Default::default() });
        let mut asm2 = Assembly::new("Con DIMM", QuoteType::Personalizado);
        asm2.set_quantity(Slot::Ram, &dimm, 1, None).unwrap();
        assert!(asm2.is_filled(Slot::Ram));
    }

    #[test]
    fn kit_fills_keyboard_and_mouse_slots() {
        let mut asm = Assembly::new("Kit", QuoteType::Personalizado);
        assert!(Slot::Keyboard.accepts(ComponentKind::KeyboardMouseKit));
        assert!(Slot::Mouse.accepts(ComponentKind::KeyboardMouseKit));
        let kit = comp(Uuid::new_v4(), ComponentKind::KeyboardMouseKit, Specs::default());
        asm.set_quantity(Slot::Keyboard, &kit, 1, None).unwrap();
        assert!(asm.is_filled(Slot::Keyboard));
    }

    #[test]
    fn optional_slots_are_not_required() {
        // Ensamble válido sin GPU, monitor, teclado, mouse ni expansiones
        let (mut cat, mut asm) = full_setup(Some(true), false);
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.map.insert(psu.id, psu);
        let report = validate(&asm, &cat);
        assert!(report.is_valid);
        assert!(!report
            .issues
            .iter()
            .any(|i| i.code.starts_with("missing_") && i.code != "missing_cpu"));
    }

    #[test]
    fn preensamble_expires_first_day_of_next_month() {
        // Creada 15/oct/2026 → caduca el 01/nov/2026
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(
            expiry_date(QuoteType::Preensamble, d),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
        // Cruzando año: creada 05/dic → caduca 01/ene del siguiente año
        let d = chrono::NaiveDate::from_ymd_opt(2026, 12, 5).unwrap();
        assert_eq!(
            expiry_date(QuoteType::Preensamble, d),
            chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );
    }

    #[test]
    fn preensamble_last_day_of_month_gets_15_days() {
        // Creada 31/oct/2026 (último día del mes) → 15 días naturales
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(
            expiry_date(QuoteType::Preensamble, d),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
        );
        // Febrero (28 días): creada 28/feb/2027 → 15/mar/2027
        let d = chrono::NaiveDate::from_ymd_opt(2027, 2, 28).unwrap();
        assert_eq!(
            expiry_date(QuoteType::Preensamble, d),
            chrono::NaiveDate::from_ymd_opt(2027, 3, 15).unwrap()
        );
    }

    #[test]
    fn personalizado_expires_15_natural_days_after_creation() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(
            expiry_date(QuoteType::Personalizado, d),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()
        );
    }

    #[test]
    fn status_transitions_and_fabrication_blocks_expiry() {
        use chrono::NaiveDate;
        let mut asm = Assembly::new("reglas", QuoteType::Preensamble);
        asm.created_at = NaiveDate::from_ymd_opt(2026, 10, 15)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        // Vigente antes de la caducidad (01/nov)
        assert_eq!(asm.status(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()), QuoteStatus::Vigente);
        // Caducada desde el día de la caducidad
        assert_eq!(asm.status(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()), QuoteStatus::Caducada);
        assert!(!asm.can_issue_sale_note(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()));
        assert!(asm.can_issue_sale_note(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()));

        // En fabricación NO caduca: puede venderse aunque pasen los días
        asm.in_fabrication = true;
        assert_eq!(asm.status(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()), QuoteStatus::EnFabricacion);
        assert!(asm.can_issue_sale_note(NaiveDate::from_ymd_opt(2027, 5, 1).unwrap()));
    }

    #[test]
    fn compatibility_socket_ddr_and_case_warnings() {
        let mut cat = FakeCatalog::default();
        let mut asm = Assembly::new("Compat", QuoteType::Personalizado);

        let cpu = comp(Uuid::new_v4(), ComponentKind::Cpu, Specs {
            socket: Some("AM5".into()),
            ..Default::default()
        });
        let mobo_bad = comp(Uuid::new_v4(), ComponentKind::Motherboard, Specs {
            socket: Some("AM4".into()),
            ddr: Some("DDR4".into()),
            form_factor: Some("ATX".into()),
            ..Default::default()
        });
        let ram_bad = comp(Uuid::new_v4(), ComponentKind::Ram, Specs {
            ddr: Some("DDR5".into()),
            ram_form: Some("DIMM".into()),
            ..Default::default()
        });
        let case_no_ff = comp(Uuid::new_v4(), ComponentKind::Case, Specs::default());
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());

        for (slot, c) in [(Slot::Cpu, &cpu), (Slot::Motherboard, &mobo_bad),
                          (Slot::Ram, &ram_bad), (Slot::Case, &case_no_ff), (Slot::Psu, &psu)] {
            asm.set_quantity(slot, c, 1, None).unwrap();
            cat.map.insert(c.id, c.clone());
        }

        let report = validate(&asm, &cat);
        assert!(report.issues.iter().any(|i| i.code == "socket_mismatch" && i.severity == Severity::Error));
        assert!(report.issues.iter().any(|i| i.code == "ddr_mismatch" && i.severity == Severity::Error));
        assert!(report.issues.iter().any(|i| i.code == "case_compatibility_unknown" && i.severity == Severity::Warning),
            "gabinete sin factor de forma = advertencia manual");
    }

    #[test]
    fn quantity_zero_removes_item() {
        let mut asm = Assembly::new("Cantidades", QuoteType::Personalizado);
        let ram = comp(Uuid::new_v4(), ComponentKind::Ram, Specs::default());
        asm.set_quantity(Slot::Ram, &ram, 2, None).unwrap();
        assert_eq!(asm.total_quantity(Slot::Ram), 2);
        asm.set_quantity(Slot::Ram, &ram, 0, None).unwrap();
        assert_eq!(asm.total_quantity(Slot::Ram), 0);
    }

    /// El tipo de cambio efectivo debe reflejarse en los precios de componentes.
    #[test]
    fn component_price_mxp_uses_ceiled_rate() {
        let c = comp(Uuid::new_v4(), ComponentKind::Psu, Specs::default());
        let rate = ExchangeRate::new(dec!(18.52));
        let mxp = c.price_mxp(&rate); // 100 USD × 19
        assert_eq!(mxp.amount, dec!(1900));
        assert_eq!(mxp.currency, Currency::Mxp);
    }
}
