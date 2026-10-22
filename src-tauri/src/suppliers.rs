//! Importación de listas de precios desde Excel, por proveedor.
//!
//! Estructura para soportar otros proveedores: cada uno implementa
//! [`SupplierAdapter`] (detección por firma de encabezados + remates de
//! formato) y se registra en [`adapters`]. La lectura del libro, el
//! mapeo de columnas por nombre y la clasificación de productos son
//! compartidos, así que un adaptador nuevo solo describe sus peculiaridades.
//!
//! Formato actual (adaptador NYKNIU): fila 1 vacía, fila 2 con fecha y
//! "T.C 18.52", fila 3 de encabezados, datos desde la fila 4.

use std::collections::HashMap;
use std::path::Path;

use calamine::{open_workbook, Data, Reader, Xlsx};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::component::{
    Component, ComponentKind, ComponentSource, Condition, Specs, Stock, StorageKind,
};
use std::collections::BTreeMap;
use crate::domain::money::{Currency, Money};

/// Mismo namespace que el script de Python para ids estables por SKU+desc.
const UUID_NS: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
    0xc8,
]);

#[derive(Debug, Clone)]
pub struct ParsedProduct {
    pub sku: String,
    pub barcode: Option<String>,
    pub description: String,
    pub brand: String,
    pub department: String,
    pub price_amount: Decimal,
    pub price_currency: Currency,
    pub on_promotion: bool,
    pub specs: Specs,
    /// Desglose por almacén para persistir en offer_stock.
    pub offer_stocks: BTreeMap<String, u32>,
    /// Proveedor de la oferta ('arroba' | 'cva' | 'manual').
    pub supplier: String,
    /// Existencia elegible para ENSAMBLES según el proveedor.
    pub stock_ensamble: u32,
    /// Existencia para VENTA INDIVIDUAL según el proveedor.
    pub stock_individual: u32,
    /// Solo disponible en CD (CVA): no usable en ensambles.
    pub cd_only: bool,
}

impl ParsedProduct {
    pub fn into_component(self) -> Component {
        // Departamento para clasificar: el del proveedor si clasifica,
        // si no se infiere de la descripción.
        let kind_department = if classify_kind(&self.department, &self.description, &self.sku)
            != ComponentKind::Other
            || self.department != "GENERAL"
        {
            self.department.clone()
        } else {
            sniff_department(&self.description)
        };
        let kind = classify_kind(&kind_department, &self.description, &self.sku);
        let supplier = self.supplier.clone();
        let stock_ensamble = self.stock_ensamble;
        let stock_individual = self.stock_individual;
        let cd_only = self.cd_only;
        let offer_stocks = self.offer_stocks.clone();
        Component {
            id: Uuid::new_v5(&UUID_NS, format!("{}|{}", self.sku, self.description).as_bytes()),
            sku: self.sku,
            barcode: self.barcode,
            description: self.description,
            brand: self.brand,
            department: self.department,
            kind,
            condition: Condition::Nuevo,
            price: Money::new(self.price_amount, self.price_currency),
            on_promotion: self.on_promotion,
            offer_stocks,
            source: ComponentSource::Lista,
            specs: self.specs.clone(),
            supplier: supplier.clone(),
            stock_ensamble_total: stock_ensamble,
            stock_individual_total: stock_individual,
            cd_only_best: cd_only,
            offers: vec![crate::domain::component::Offer {
                supplier,
                price: Money::new(self.price_amount, self.price_currency),
                on_promotion: self.on_promotion,
                stock_label: String::new(),
                stocks: self.offer_stocks.clone(),
                stock_ensamble,
                stock_individual,
                cd_only,
                price_overridden: false,
                stock_overridden: false,
                list_price: None,
                list_stock: None,
            }],
            overrides: Default::default(),
            list_kind: kind,
            list_specs: self.specs,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedCatalog {
    pub supplier: String,
    pub source_file: String,
    pub list_date: Option<String>,
    pub exchange_rate: Option<Decimal>,
    pub products: Vec<ParsedProduct>,
}

// ---------------------------------------------------------------------
// Adaptadores de proveedor
// ---------------------------------------------------------------------

pub trait SupplierAdapter {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// ¿Este libro corresponde al proveedor? `headers` son los encabezados
    /// normalizados de la fila detectada y `has_rate_row` indica si hay una
    /// fila superior con tipo de cambio.
    fn detect(&self, headers: &[String], has_rate_row: bool) -> bool;
    /// Moneda por defecto cuando la lista no trae columna de moneda.
    fn default_currency(&self) -> Currency {
        Currency::Mxp
    }
    /// Departamento por defecto cuando no hay columna de departamento.
    fn default_department(&self) -> String {
        "GENERAL".into()
    }
    /// Marca por defecto cuando no hay columna de marca.
    fn default_brand(&self) -> String {
        "SIN MARCA".into()
    }
}

/// Proveedor actual: lista completa con departamentos, moneda, promoción,
/// los cinco almacenes y fila de tipo de cambio.
pub struct NykniuAdapter;

impl SupplierAdapter for NykniuAdapter {
    fn id(&self) -> &'static str {
        // El proveedor actual es 'arroba' (formato de lista NYKNIU).
        "arroba"
    }
    fn name(&self) -> &'static str {
        "NYKNIU (formato de lista actual)"
    }
    fn detect(&self, headers: &[String], has_rate_row: bool) -> bool {
        has_rate_row
            && contains(headers, "DEPARTAMENTO")
            && contains(headers, "MONEDA")
            && contains(headers, "COSTO")
            && contains(headers, "PROMOC")
    }
}

/// CVA: lista en .xls con Clave/Marca/Grupo/Descripción/Precio/Moneda/
/// Promos/Disp/Disp CD/UPC. Disp es la existencia con prioridad de
/// ensambles; Disp CD solo suma para venta individual.
pub struct CvaAdapter;

impl SupplierAdapter for CvaAdapter {
    fn id(&self) -> &'static str {
        "cva"
    }
    fn name(&self) -> &'static str {
        "CVA (lista con Disp y Disp CD)"
    }
    fn detect(&self, headers: &[String], _has_rate_row: bool) -> bool {
        contains(headers, "CLAVE") && contains(headers, "GRUPO") && contains(headers, "DISP CD")
    }
    fn default_department(&self) -> String {
        "GENERAL".into()
    }
    fn default_brand(&self) -> String {
        "SIN MARCA".into()
    }
}

/// Fallback para otros proveedores: bastan SKU + descripción + costo. Los
/// campos ausentes toman valores por defecto y la clasificación de tipo se
/// hace por palabras clave de la descripción.
pub struct GenericAdapter;

impl SupplierAdapter for GenericAdapter {
    fn id(&self) -> &'static str {
        "generico"
    }
    fn name(&self) -> &'static str {
        "Genérico (otros proveedores)"
    }
    fn detect(&self, headers: &[String], _has_rate_row: bool) -> bool {
        contains(headers, "SKU") || contains(headers, "ARTICULO") || contains(headers, "CODIGO")
            || contains(headers, "DESCRIP")
    }
}

/// Registro de adaptadores: el orden importa (el más específico primero).
pub fn adapters() -> Vec<Box<dyn SupplierAdapter>> {
    vec![Box::new(NykniuAdapter), Box::new(CvaAdapter), Box::new(GenericAdapter)]
}

// ---------------------------------------------------------------------
// Lectura y mapeo del libro
// ---------------------------------------------------------------------

fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.to_uppercase().chars() {
        match ch {
            'Á' | 'À' | 'Ä' => out.push('A'),
            'É' | 'È' | 'Ë' => out.push('E'),
            'Í' | 'Ì' | 'Ï' => out.push('I'),
            'Ó' | 'Ò' | 'Ö' => out.push('O'),
            'Ú' | 'Ù' | 'Ü' => out.push('U'),
            'Ñ' => out.push('N'),
            other => out.push(other),
        }
    }
    out.trim().to_string()
}

fn contains(headers: &[String], needle: &str) -> bool {
    let n = normalize(needle);
    headers.iter().any(|h| h.contains(&n))
}

/// Convierte una celda a texto (los códigos de barras llegan como flotante).
fn cell_text(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::Int(i) => format!("{i}"),
        Data::Bool(b) => format!("{b}"),
        Data::DateTime(dt) => excel_serial_to_date(dt.as_f64()).unwrap_or_default(),
        Data::Error(_) => String::new(),
        Data::DateTimeIso(s) => s.trim().to_string(),
        Data::DurationIso(s) => s.trim().to_string(),
    }
}

/// Serial de Excel (días desde 1899-12-30) a fecha ISO.
fn excel_serial_to_date(serial: f64) -> Option<String> {
    use chrono::Datelike;
    if !(0.0..1e6).contains(&serial) {
        return None;
    }
    let days = serial.floor() as i64;
    let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 30)?;
    let d = base + chrono::Duration::days(days);
    Some(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
}

fn cell_decimal(d: &Data) -> Option<Decimal> {
    match d {
        Data::Float(f) => Decimal::try_from(*f).ok(),
        Data::Int(i) => Decimal::try_from(*i).ok(),
        Data::String(s) => rust_decimal::Decimal::from_str_exact(s.trim()).ok(),
        _ => None,
    }
}

struct ColumnMap {
    sku: usize,
    barcode: Option<usize>,
    description: usize,
    department: Option<usize>,
    currency: Option<usize>,
    brand: Option<usize>,
    promotion: Option<usize>,
    cost: usize,
    stocks: HashMap<&'static str, usize>,
}

/// Encuentra la fila de encabezados y mapea las columnas por nombre
/// (insensible a acentos, mayúsculas y orden de columnas).
fn locate_headers(rows: &[Vec<Data>]) -> Option<(usize, ColumnMap)> {
    fn pos(headers: &[String], needle: &str) -> Option<usize> {
        let n = normalize(needle);
        headers.iter().position(|h| h.contains(&n))
    }
    for (idx, row) in rows.iter().enumerate().take(20) {
        let headers: Vec<String> = row.iter().map(|c| normalize(&cell_text(c))).collect();
        // SKU: Clave (CVA) / Número de artículo o SKU (otros). CLAVE primero
        // evita confundir "Descripción del artículo" con la columna de SKU.
        let sku = pos(&headers, "CLAVE")
            .or_else(|| pos(&headers, "SKU"))
            .or_else(|| pos(&headers, "ARTICULO"));
        let desc = pos(&headers, "DESCRIP");
        let cost = headers.iter().position(|h| {
            h == "COSTO"
                || h.starts_with("COSTO ")
                || h.contains("COSTO UNIT")
                || h == "PRECIO"
                || h.starts_with("PRECIO ")
        });
        if sku.is_none() || desc.is_none() || cost.is_none() {
            continue;
        }
        let barcode = pos(&headers, "BARRAS").or_else(|| pos(&headers, "UPC"));
        let department = pos(&headers, "DEPART").or_else(|| pos(&headers, "GRUPO"));
        let currency = pos(&headers, "MONEDA");
        let brand = pos(&headers, "MARCA");
        let promotion = pos(&headers, "PROMO");
        let mut stocks = HashMap::new();
        for (key, label) in [
            ("a_cen", "A-CEN"),
            ("ave_2", "AVE-2"),
            ("cedis", "CEDIS"),
            ("cmal", "CMAL"),
            ("gdl", "GDL"),
        ] {
            if let Some(p) = pos(&headers, label) {
                stocks.insert(key, p);
            }
        }
        return Some((
            idx,
            ColumnMap {
                sku: sku.unwrap(),
                barcode,
                description: desc.unwrap(),
                department,
                currency,
                brand,
                promotion,
                cost: cost.unwrap(),
                stocks,
            },
        ));
    }
    None
}

/// Extrae el tipo de cambio ("T.C 18.52") y la fecha de la lista de las
/// filas previas a los encabezados.
fn extract_meta_above(rows: &[Vec<Data>], header_idx: usize) -> (Option<Decimal>, Option<String>) {
    let mut rate = None;
    let mut date = None;
    for row in rows.iter().take(header_idx) {
        for cell in row {
            match cell {
                Data::DateTime(dt) => {
                    if date.is_none() {
                        if let Some(d) = excel_serial_to_date(dt.as_f64()) {
                            date = Some(d);
                        }
                    }
                }
                _ => {
                    let text = normalize(&cell_text(cell));
                    if rate.is_none() && (text.contains("T.C") || text.contains("TC ")) {
                        // T.C 18.52 / TC 19 / T.C=19.5
                        let digits: String = text
                            .chars()
                            .skip_while(|c| !c.is_ascii_digit())
                            .take_while(|c| c.is_ascii_digit() || *c == '.')
                            .collect();
                        if let Ok(v) = rust_decimal::Decimal::from_str_exact(&digits) {
                            rate = Some(v);
                        }
                    }
                }
            }
        }
    }
    (rate, date)
}

// ---------------------------------------------------------------------
// Clasificación de productos (misma lógica que scripts/extract_catalog.py)
// ---------------------------------------------------------------------

fn regex_first(text: &str, pattern: &str) -> Option<String> {
    // Regex mínima por patrones fijos usados aquí (sin crate de regex).
    simple_match(text, pattern)
}

/// Implementación liviana de los tres patrones que usamos.
fn simple_match(text: &str, pattern: &str) -> Option<String> {
    match pattern {
        "socket" => {
            for key in ["AM5", "AM4", "LGA1700", "LGA1851", "LGA1200"] {
                if text.contains(key) {
                    return Some(key.to_string());
                }
            }
            // LGA con 3-4 dígitos genéricos
            for token in text.split_whitespace() {
                if token.starts_with("LGA") {
                    let digits: String = token.chars().skip(3).take_while(|c| c.is_ascii_digit()).collect();
                    if digits.len() >= 3 {
                        return Some(format!("LGA{digits}"));
                    }
                }
            }
            None
        }
        "ddr" => {
            for key in ["DDR5", "DDR4", "DDR3"] {
                if text.contains(key) {
                    return Some(key.to_string());
                }
            }
            None
        }
        "wattage" => {
            // (\d{3,4})W con espacio opcional antes de la W
            let chars: Vec<char> = text.chars().collect();
            for i in 0..chars.len() {
                if chars[i] == 'W' {
                    let start = chars[..i]
                        .iter()
                        .rposition(|c| !c.is_ascii_digit() && *c != ' ')
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    let digits: String = chars[start..i].iter().collect();
                    let digits = digits.trim().to_string();
                    if (3..=4).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
                    {
                        return Some(digits);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn capacity_from_desc(desc: &str) -> Option<(String, i64)> {
    let upper = desc.to_uppercase();
    for unit in ["TB", "GB"] {
        let chars: Vec<char> = upper.chars().collect();
        for i in 0..chars.len() {
            if chars[i..].starts_with(&[unit.chars().next().unwrap(), unit.chars().nth(1).unwrap()])
            {
                // dígitos (con punto decimal opcional) antes de la unidad
                let start = chars[..i]
                    .iter()
                    .rposition(|c| !c.is_ascii_digit() && *c != '.')
                    .map(|p| p + 1)
                    .unwrap_or(0);
                let digits: String = chars[start..i].iter().collect();
                if !digits.is_empty()
                    && digits
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == '.')
                {
                    let raw = format!("{digits}{unit}");
                    let gb: f64 = digits.parse().unwrap_or(0.0);
                    let gb = if unit == "TB" { (gb * 1000.0) as i64 } else { gb as i64 };
                    return Some((raw, gb));
                }
            }
        }
    }
    None
}

/// Departamento inferido desde la descripción (para proveedores sin esa
/// columna): palabras clave del catálogo actual.
fn sniff_department(desc: &str) -> String {
    let d = normalize(desc);
    if d.contains("PROCESADOR") {
        "PROCESADORES".into()
    } else if d.contains("MOTHERBOARD") || d.contains("MOTHER BOARD") || (d.contains("MB ") && d.contains("ATX")) {
        "MOTHER BOARD".into()
    } else if d.contains("MEMORIA RAM") || d.contains("MEMORIA DIMM") || d.contains("MEMORIA SODIMM") {
        "MEMORIA RAM".into()
    } else if d.contains("TARJETA DE VIDEO") || d.contains("VGA ") {
        "TARJETA DE VIDEO".into()
    } else if d.contains("MONITOR") {
        "MONITORES".into()
    } else if d.contains("SSD") {
        "SSD INTERNO".into()
    } else if d.contains("DISCO DURO") || d.contains("HDD") {
        "DD INTERNO".into()
    } else if d.contains("ENFRIAMIENTO") || d.contains("VENTILADOR") || d.contains("COOLER") {
        "ENFRIAMIENTO".into()
    } else if d.contains("FUENTE DE PODER") {
        "COMPONENTES".into()
    } else if d.contains("GABINETE") {
        "COMPONENTES".into()
    } else if d.contains("TECLADO") && d.contains("MOUSE") {
        "PERIFERICOS".into()
    } else if d.contains("TECLADO") {
        "PERIFERICOS".into()
    } else if d.contains("MOUSE") {
        "PERIFERICOS".into()
    } else {
        "GENERAL".into()
    }
}

/// Clasificación (kind + specs) idéntica a la del script de Python.
pub fn classify_kind(department: &str, desc: &str, sku: &str) -> ComponentKind {
    let (kind, _) = classify_full(department, desc, sku);
    kind
}

fn classify_full(department: &str, desc: &str, sku: &str) -> (ComponentKind, Specs) {
    let d = desc.to_uppercase();
    let sku_up = sku.to_uppercase();
    let mut specs = Specs::default();

    match department {
        "PROCESADORES" => {
            if let Some(s) = regex_first(&d, "socket") {
                specs.socket = Some(s);
            }
            if d.contains("SIN DISIPADOR") || d.contains("NO INCLUYE DISIPADOR") {
                specs.includes_cooler = Some(false);
            } else if sku_up.ends_with("WOF") {
                specs.includes_cooler = Some(false);
            } else if d.contains("CON DISIPADOR") || d.contains("INCLUYE DISIPADOR") || sku_up.ends_with("BOX") {
                specs.includes_cooler = Some(true);
            }
            (ComponentKind::Cpu, specs)
        }
        "MOTHER BOARD" => {
            if let Some(s) = regex_first(&d, "socket") {
                specs.socket = Some(s);
            }
            specs.form_factor = if d.contains("MINI ITX") {
                Some("MINI ITX".into())
            } else if d.contains("MICRO ATX") || d.contains("MATX") {
                Some("MICRO ATX".into())
            } else if d.contains(" ATX") {
                Some("ATX".into())
            } else {
                None
            };
            (ComponentKind::Motherboard, specs)
        }
        "MEMORIA RAM" => {
            if let Some(s) = regex_first(&d, "ddr") {
                specs.ddr = Some(s);
            }
            if let Some((raw, gb)) = capacity_from_desc(&d) {
                specs.capacity_gb = Some(gb.min(i64::from(u16::MAX)) as u16);
                if specs.capacity_raw.is_none() {
                    specs.capacity_raw = Some(raw);
                }
            }
            specs.ram_form = Some(if d.contains("SODIMM") { "SODIMM".into() } else { "DIMM".into() });
            (ComponentKind::Ram, specs)
        }
        "TARJETA DE VIDEO" => {
            if let Some((raw, _)) = capacity_from_desc(&d) {
                specs.capacity_raw = Some(raw);
            }
            specs.vram = capacity_from_desc(&d).map(|(raw, _)| raw);
            (ComponentKind::Gpu, specs)
        }
        "MONITORES" => {
            // pulgadas: \d\d(\.\d)? seguido de IN o P
            let chars: Vec<char> = d.chars().collect();
            for i in 0..chars.len().saturating_sub(1) {
                if chars[i].is_ascii_digit()
                    && chars[i + 1].is_ascii_digit()
                    && (chars.get(i + 2) == Some(&'I') && chars.get(i + 3) == Some(&'N')
                        || chars.get(i + 2) == Some(&'P'))
                {
                    specs.inches = Some(chars[i..=i + 1].iter().collect());
                    break;
                }
            }
            (ComponentKind::Monitor, specs)
        }
        "SSD INTERNO" => {
            specs.storage_kind = Some(StorageKind::Ssd);
            specs.internal = Some(true);
            (ComponentKind::Storage, specs)
        }
        "DD INTERNO" | "DISCO DURO" => {
            specs.storage_kind = Some(StorageKind::Hdd);
            specs.internal = Some(true);
            (ComponentKind::Storage, specs)
        }
        "ENFRIAMIENTO" => {
            if d.contains("LIQUIDO") || d.contains("LIQUIDA") || d.contains(" AIO") {
                (ComponentKind::CoolerLiquid, specs)
            } else if d.contains("BASE ENFRIADORA") || d.contains("LAPTOP") {
                (ComponentKind::Other, specs)
            } else {
                (ComponentKind::CoolerAir, specs)
            }
        }
        "PERIFERICOS" => {
            let has_mouse = d.contains("MOUSE");
            let has_kb = d.contains("TECLADO");
            if has_mouse && has_kb {
                (ComponentKind::KeyboardMouseKit, specs)
            } else if has_mouse {
                (ComponentKind::Mouse, specs)
            } else if has_kb {
                (ComponentKind::Keyboard, specs)
            } else {
                (ComponentKind::Other, specs)
            }
        }
        "REDES" => {
            specs.card_type = Some("red".into());
            (ComponentKind::ExpansionCard, specs)
        }
        "COMPONENTES" => {
            if d.contains("FUENTE DE PODER") {
                if let Some(w) = regex_first(&d, "wattage") {
                    specs.wattage = w.parse().ok();
                }
                (ComponentKind::Psu, specs)
            } else if d.contains("GABINETE") || d.contains("CASE") {
                if d.contains("SIN FUENTE") {
                    specs.includes_psu = Some(false);
                } else if d.contains("CON FUENTE") {
                    specs.includes_psu = Some(true);
                    let needle = "FUENTE";
                    if let Some(pos) = d.find(needle) {
                        if let Some(w) = regex_first(&d[pos..], "wattage") {
                            specs.psu_watts = w.parse().ok();
                        }
                    }
                } else {
                    specs.includes_psu = Some(false);
                }
                (ComponentKind::Case, specs)
            } else {
                (ComponentKind::Other, specs)
            }
        }
        "ACCESORIOS" => {
            if d.contains("TARJETA DE RED") && (d.contains("PCI") || d.contains("PCIE")) {
                specs.card_type = Some("red".into());
                (ComponentKind::ExpansionCard, specs)
            } else {
                (ComponentKind::Other, specs)
            }
        }
        _ => (ComponentKind::Other, specs),
    }
}

// ---------------------------------------------------------------------
// Importación
// ---------------------------------------------------------------------

/// Lee un xlsx, detecta el proveedor y devuelve el catálogo parseado.
/// Abre .xls (BIFF, CVA) o .xlsx/.xlsm según la extensión.
fn open_any(path: &Path) -> Result<Vec<Vec<Data>>, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "xls" => {
            let mut wb: calamine::Xls<_> =
                open_workbook(path).map_err(|e| format!("No se pudo abrir el .xls: {e}"))?;
            let sheet = wb
                .sheet_names()
                .first()
                .cloned()
                .ok_or_else(|| "El Excel no tiene hojas".to_string())?;
            let range = wb
                .worksheet_range(&sheet)
                .map_err(|e| format!("No se pudo leer la hoja '{sheet}': {e}"))?;
            Ok(range.rows().map(|r| r.to_vec()).collect())
        }
        _ => {
            let mut wb: Xlsx<_> =
                open_workbook(path).map_err(|e| format!("No se pudo abrir el Excel: {e}"))?;
            let sheet = wb
                .sheet_names()
                .first()
                .cloned()
                .ok_or_else(|| "El Excel no tiene hojas".to_string())?;
            let range = wb
                .worksheet_range(&sheet)
                .map_err(|e| format!("No se pudo leer la hoja '{sheet}': {e}"))?;
            Ok(range.rows().map(|r| r.to_vec()).collect())
        }
    }
}

pub fn import_xlsx(path: &Path) -> Result<ParsedCatalog, String> {
    let source_file = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let rows: Vec<Vec<Data>> = open_any(path)?;
    if rows.is_empty() {
        return Err("El Excel está vacío".into());
    }

    let (header_idx, map) =
        locate_headers(&rows)
        .ok_or_else(|| String::from("No se encontraron encabezados (SKU/Descripción/Costo)"))?;
    let headers: Vec<String> = rows[header_idx].iter().map(|c| normalize(&cell_text(c))).collect();
    let (rate, list_date) = extract_meta_above(&rows, header_idx);
    let has_rate_row = rate.is_some();

    let registry = adapters();
    let adapter = registry
        .iter()
        .find(|a| a.detect(&headers, has_rate_row))
        .ok_or_else(|| {
            String::from("Formato de lista no reconocido por ningún proveedor registrado")
        })?;
    let supplier = format!("{} [{id}]", adapter.name(), id = adapter.id());

    let mut products = Vec::new();
    for row in rows.iter().skip(header_idx + 1) {
        let get = |col: Option<usize>| col.map(|i| cell_text(&row[i])).unwrap_or_default();
        let mut sku = get(Some(map.sku));
        let description = get(Some(map.description));
        // CVA: el SKU real es el "Codigo de Fabricante" (la Clave es su
        // código interno). El SKU es índice obligatorio: filas sin él, fuera.
        if adapter.id() == "cva" {
            let headers: Vec<String> = rows[header_idx]
                .iter()
                .map(|c| normalize(&cell_text(c)))
                .collect();
            if let Some(pos) = headers.iter().position(|h| h.contains("CODIGO DE FABRICANTE")) {
                sku = get(Some(pos));
            }
        }
        if sku.is_empty() || description.is_empty() {
            continue;
        }
        let cost = cell_decimal(&row[map.cost]).unwrap_or(Decimal::ZERO);
        let currency = map
            .currency
            .map(|i| match normalize(&cell_text(&row[i])).as_str() {
                "USD" | "DOLARES" | "DOLAR" | "US" => Currency::Usd,
                _ => Currency::Mxp,
            })
            .unwrap_or_else(|| adapter.default_currency());
        let mut stock = Stock::default();
        for (key, col) in &map.stocks {
            let qty = cell_decimal(&row[*col]).unwrap_or(Decimal::ZERO);
            let qty = u32::try_from(qty.max(Decimal::ZERO).round()).unwrap_or(0);
            match *key {
                "a_cen" => stock.a_cen = qty,
                "ave_2" => stock.ave_2 = qty,
                "cedis" => stock.cedis = qty,
                "cmal" => stock.cmal = qty,
                "gdl" => stock.gdl = qty,
                _ => {}
            }
        }
        let department = map
            .department
            .map(|i| cell_text(&row[i]))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                let sniffed = sniff_department(&description);
                if sniffed == "GENERAL" {
                    adapter.default_department()
                } else {
                    sniffed
                }
            });
        // CVA: existencias por flujo desde Disp y Disp CD.
        let mut stock_ensamble = stock.sellable();
        let mut stock_individual = stock.sellable();
        let mut cd_only = false;
        if adapter.id() == "cva" {
            let disp = headers
                .iter()
                .position(|h| h.contains("DISP"))
                .and_then(|i| cell_decimal(&row[i]).map(|d| u32::try_from(d.max(Decimal::ZERO).round()).unwrap_or(0)))
                .unwrap_or(0);
            let disp_cd = headers
                .iter()
                .position(|h| h.contains("DISP CD"))
                .and_then(|i| cell_decimal(&row[i]).map(|d| u32::try_from(d.max(Decimal::ZERO).round()).unwrap_or(0)))
                .unwrap_or(0);
            stock_ensamble = disp;
            stock_individual = disp + disp_cd;
            cd_only = disp == 0 && disp_cd > 0;
        }
        let (_kind, mut specs) = classify_full(&department, &description, &sku);
        // Capacidades generales (SSD/HDD y RAM ya las cubren sus ramas)
        if specs.capacity_raw.is_none() {
            if let Some((raw, gb)) = capacity_from_desc(&d_upper(&description)) {
                specs.capacity_raw = Some(raw);
                if specs.capacity_gb.is_none() {
                    specs.capacity_gb = Some(gb.min(i64::from(u16::MAX)) as u16);
                }
            }
        }
        products.push(ParsedProduct {
            sku,
            barcode: map.barcode.map(|i| get(Some(i))).filter(|s| !s.is_empty()),
            description,
            brand: map
                .brand
                .map(|i| get(Some(i)))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| adapter.default_brand()),
            department,
            price_amount: cost,
            price_currency: currency,
            on_promotion: map
                .promotion
                .map(|i| matches!(normalize(&cell_text(&row[i])).as_str(), "SI" | "S" | "YES" | "1" | "TRUE" | "P"))
                .unwrap_or(false),
            specs,
            supplier: adapter.id().to_string(),
            // Regla por proveedor: 'arroba' vende por CEN+AVE-2+CEDIS (ambos
            // flujos). 'cva' usa Disp para ensambles y Disp+Disp CD para
            // venta individual; solo-CD se marca para no usarlo en ensambles.
            // Otros proveedores: sin stock conocido (0).
            offer_stocks: if adapter.id() == "cva" {
                let mut m = BTreeMap::new();
                if stock_ensamble > 0 {
                    m.insert("disp".into(), stock_ensamble);
                }
                if stock_individual > stock_ensamble {
                    m.insert("disp_cd".into(), stock_individual - stock_ensamble);
                }
                m
            } else {
                let mut m = BTreeMap::new();
                for (w, q) in [
                    ("a_cen", stock.a_cen),
                    ("ave_2", stock.ave_2),
                    ("cedis", stock.cedis),
                    ("cmal", stock.cmal),
                    ("gdl", stock.gdl),
                ] {
                    if q > 0 {
                        m.insert(w.into(), q);
                    }
                }
                m
            },
            stock_ensamble,
            stock_individual,
            cd_only,
        });
    }

    if products.is_empty() {
        return Err(String::from("El Excel no contiene productos legibles"));
    }

    Ok(ParsedCatalog {
        supplier,
        source_file,
        list_date,
        exchange_rate: rate,
        products,
    })
}

fn d_upper(s: &str) -> String {
    s.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Escribe un fixture embebido a un archivo temporal y lo importa.
    fn import_fixture(bytes: &[u8]) -> Result<ParsedCatalog, String> {
        let mut path = std::env::temp_dir();
        // XLSX empieza con "PK" (zip); el .xls de CVA es OLE2 (D0 CF).
        let ext = if bytes.starts_with(b"PK") { "xlsx" } else { "xls" };
        path.push(format!("lista-{}.{ext}", Uuid::new_v4()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(bytes).unwrap();
        }
        let result = import_xlsx(&path);
        std::fs::remove_file(&path).ok();
        result
    }

    const NYKNIU: &[u8] = include_bytes!("../tests/fixtures/lista_nykniu.xlsx");
    const GENERICO: &[u8] = include_bytes!("../tests/fixtures/lista_generico.xlsx");
    const CVA: &[u8] = include_bytes!("../tests/fixtures/lista_cva_sample.xls");
    const CVA_XLSX: &[u8] = include_bytes!("../tests/fixtures/lista_cva.xlsx");

    /// El portal de CVA también publica .xlsx: el mismo adaptador debe
    /// reconocerlo por sus encabezados sin importar el formato del libro.
    #[test]
    fn cva_adapter_parses_xlsx_upload() {
        let cat = import_fixture(CVA_XLSX).expect("parsear CVA en xlsx");
        assert!(cat.supplier.contains("cva"), "supplier={}", cat.supplier);
        assert_eq!(cat.products.len(), 4);

        // Reglas de flujo: ensamble usa Disp; venta individual Disp+Disp CD
        let solo_cd = cat.products.iter().find(|p| p.sku == "COOL-FAB-1").unwrap();
        assert_eq!(solo_cd.stock_ensamble, 0);
        assert_eq!(solo_cd.stock_individual, 2);
        assert!(solo_cd.cd_only, "solo-CD marcado para el proveedor CVA");

        let normal = cat.products.iter().find(|p| p.sku == "KB-FAB-1").unwrap();
        assert_eq!(normal.stock_ensamble, 15);
        assert_eq!(normal.stock_individual, 18);
        assert!(!normal.cd_only);
        assert!(normal.on_promotion, "Promos 'P' = promoción");

        // Monedas de texto CVA
        let usd = cat.products.iter().find(|p| p.sku == "HDMI-FAB-1").unwrap();
        assert_eq!(usd.price_currency, Currency::Usd);
        let mxn = cat.products.iter().find(|p| p.sku == "KB-FAB-1").unwrap();
        assert_eq!(mxn.price_currency, Currency::Mxp);

        // UPC como código de barras
        let barcode = cat.products.iter().find(|p| p.sku == "HDMI-FAB-1").unwrap();
        assert_eq!(barcode.barcode.as_deref(), Some("750349871001"));
    }

    /// El lector BIFF (.xls) con el fixture sintético de CVA (los datos
    /// reales del proveedor no se publican en el repositorio).
    #[test]
    fn cva_adapter_parses_disp_and_cd() {
        let cat = import_fixture(CVA).expect("parsear lista CVA en .xls");
        assert!(cat.supplier.contains("cva"), "supplier={}", cat.supplier);
        assert_eq!(cat.exchange_rate, None, "CVA no trae T.C");
        assert_eq!(cat.products.len(), 4);

        // Reglas de flujo: ensamble usa Disp; venta individual Disp+Disp CD
        let solo_cd = cat.products.iter().find(|p| p.sku == "COOL-FAB-1").unwrap();
        assert_eq!(solo_cd.stock_ensamble, 0);
        assert_eq!(solo_cd.stock_individual, 2);
        assert!(solo_cd.cd_only, "solo-CD marcado para el proveedor CVA");

        let normal = cat.products.iter().find(|p| p.sku == "KB-FAB-1").unwrap();
        assert_eq!(normal.stock_ensamble, 15);
        assert_eq!(normal.stock_individual, 18);
        assert!(!normal.cd_only);
        assert!(normal.on_promotion, "Promos 'P' = promoción");

        // Monedas de texto CVA
        let usd = cat.products.iter().find(|p| p.sku == "HDMI-FAB-1").unwrap();
        assert_eq!(usd.price_currency, Currency::Usd);
        let mxn = cat.products.iter().find(|p| p.sku == "KB-FAB-1").unwrap();
        assert_eq!(mxn.price_currency, Currency::Mxp);

        // UPC como código de barras
        let barcode = cat.products.iter().find(|p| p.sku == "HDMI-FAB-1").unwrap();
        assert_eq!(barcode.barcode.as_deref(), Some("750349871001"));
    }

    #[test]
    fn nykniu_adapter_parses_format_and_specs() {
        let cat = import_fixture(NYKNIU).expect("parsear lista NYKNIU");
        assert!(cat.supplier.contains("arroba"));
        assert_eq!(cat.exchange_rate, Some(rust_decimal_macros::dec!(18.75)));
        assert_eq!(cat.list_date.as_deref(), Some("2026-10-20"));
        assert_eq!(cat.products.len(), 4);

        let cpu = cat.products.iter().find(|p| p.sku == "100-100001582WOF").unwrap();
        let kind = classify_kind(&cpu.department, &cpu.description, &cpu.sku);
        assert!(matches!(kind, ComponentKind::Cpu));
        assert_eq!(cpu.specs.includes_cooler, Some(false));
        assert_eq!(cpu.price_currency, Currency::Usd);

        let case = cat.products.iter().find(|p| p.sku == "EV-1015").unwrap();
        let (kind, specs) = classify_full(&case.department, &case.description, &case.sku);
        assert!(matches!(kind, ComponentKind::Case));
        assert_eq!(specs.includes_psu, Some(true));
        assert_eq!(specs.psu_watts, Some(700));
        assert_eq!(case.offer_stocks.get("cedis"), Some(&2));
        assert_eq!(case.offer_stocks.get("cmal"), Some(&9));
        assert!(case.on_promotion);

        let cooler = cat.products.iter().find(|p| p.sku == "ENF-LQ-240").unwrap();
        let kind = classify_kind(&cooler.department, &cooler.description, &cooler.sku);
        assert!(matches!(kind, ComponentKind::CoolerLiquid));
    }

    #[test]
    fn generic_adapter_sniffs_kind_from_description() {
        let cat = import_fixture(GENERICO).expect("parsear lista genérica");
        assert!(cat.supplier.contains("generico"));
        assert_eq!(cat.exchange_rate, None, "el proveedor genérico no trae T.C");
        assert_eq!(cat.products.len(), 3);

        let psu = cat.products.iter().find(|p| p.sku == "GEN-PSU-750").unwrap();
        let kind = classify_kind(&psu.department, &psu.description, &psu.sku);
        assert!(matches!(kind, ComponentKind::Psu));
        assert_eq!(psu.specs.wattage, Some(750));
        assert_eq!(psu.price_currency, Currency::Mxp, "moneda por defecto");

        let sodimm = cat.products.iter().find(|p| p.sku == "GEN-RAM-8").unwrap();
        let (kind, specs) = classify_full(&sodimm.department, &sodimm.description, &sodimm.sku);
        assert!(matches!(kind, ComponentKind::Ram));
        assert_eq!(specs.ram_form.as_deref(), Some("SODIMM"));
        assert_eq!(specs.capacity_gb, Some(8));
    }
}
