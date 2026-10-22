//! Consultas e importación al feed XML de CVA.
//!
//! La autenticación es por IP (dar de alta la IP del local en el portal de
//! CVA); la app solo necesita el NÚMERO DE CLIENTE. El servicio solo
//! responde en el horario 19:01–08:59 — fuera de él devuelve
//! `<articulos></articulos>` vacío sin error, así que un resultado vacío
//! fuera de horario se convierte en un mensaje explicativo.
//!
//! Parámetros usados: `marca`, `grupo`, `clave`, `codigo` (nuestro SKU),
//! `sucursales=1` (existencias por sucursal), `promos=1` (promociones) y
//! `subgpo=1` (subgrupo, para separar p. ej. fuentes de poder de UPS).
//! Wildcard: `%`.

use quick_xml::events::Event;
use quick_xml::Reader;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::domain::component::ComponentKind;
use crate::domain::money::Currency;
use crate::suppliers::ParsedProduct;

/// URL base del feed (los parámetros se agregan por consulta).
pub const FEED_BASE_URL: &str =
    "https://www.grupocva.com/catalogo_clientes_xml/lista_precios.xml";

/// Clave de la tabla `settings` con el número de cliente.
pub const SETTING_CLIENTE: &str = "cva_feed_cliente";

/// Grupos de CVA que nos interesan para ENSAMBLES y el tipo que les
/// corresponde. La consulta se hace con comodín (`grupo=%<nombre>%`) y la
/// coincidencia (prefijos ≥5 letras, sin acentos) tolera plurales —
/// «TARJETAS MADRE» casa con «tarjeta madre». Los conteos por grupo del
/// resultado de importación delatan un nombre que no cuaja.
pub const ASSEMBLY_GROUPS: &[(&str, ComponentKind)] = &[
    ("gabinetes", ComponentKind::Case),
    ("monitores", ComponentKind::Monitor),
    ("teclado y mouse", ComponentKind::KeyboardMouseKit),
    ("mouse", ComponentKind::Mouse),
    ("procesadores", ComponentKind::Cpu),
    ("tarjeta madre", ComponentKind::Motherboard),
    ("teclados", ComponentKind::Keyboard),
    ("video", ComponentKind::Gpu),
    ("memorias", ComponentKind::Ram),
    ("discos duros", ComponentKind::Storage),
    ("energia", ComponentKind::Psu),
];

/// Subgrupo/descripción dentro de memorias y energía que NO es de ensamble
/// (decisión autorizada: se importa como tipo «otro», vendible solo en
/// venta individual).
const NON_ASSEMBLY_KEYWORDS: &[&str] = &[
    // memorias
    "MEMORIA USB", "USB", "FLASH", "MICRO SD", "TARJETA SD", "SDXC", "SDHC",
    "LECTOR", "GRABADOR", "CFEXPRESS", "XQD",
    // energía
    "UPS", "NO-BREAK", "NOBREAK", "EXTENSION", "REGULADOR", "SUPRESOR",
    "PDU", "CARGADOR", "MULTIPACK", "APAGADOR",
];

/// Etiquetas del `<item>` que NO son sucursales (normalizadas: minúsculas,
/// sin guiones bajos). Todo entero que aparezca con otra etiqueta se trata
/// como existencia de sucursal (RETAIL_CDMX, VENTAS_*, …).
const SCALAR_TAGS: &[&str] = &[
    "clave", "esproducto esd", "codigo fabricante", "codigo_fabricante",
    "descripcion", "solucion", "grupo", "subgrupo", "marca", "garantia",
    "clase", "disponible", "precio", "moneda", "ficha tecnica",
    "ficha comercial", "imagen", "disponiblecd", "totaldescuento",
    "monedadescuento", "preciodescuento", "monedapreciodescuento",
    "clavepromocion", "descripcionpromocion", "vencimientopromocion",
    "disponibleenpromocion", "tipoproducto", "iddepartamento", "departamento",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedConfig {
    /// Número de cliente asignado por CVA (la auth real es por IP).
    #[serde(default)]
    pub cliente: String,
}

impl FeedConfig {
    pub fn is_configured(&self) -> bool {
        !self.cliente.trim().is_empty()
    }
}

/// Promoción tal como viene en el `<item>`.
#[derive(Debug, Clone, Default)]
pub struct CvaPromo {
    pub clave: Option<String>,
    pub descripcion: Option<String>,
    pub vencimiento: Option<String>,
    /// `PrecioDescuento`: el precio OFERTA (no el monto del descuento).
    pub precio_descuento: Option<String>,
    pub moneda: Option<String>,
    pub disponible: Option<i64>,
}

/// Un `<item>` del feed, ya parseado. Las etiquetas numéricas que no son
/// escalares conocidas se acumulan como sucursales.
#[derive(Debug, Clone, Default)]
pub struct CvaItem {
    pub clave: String,
    pub codigo_fabricante: String,
    pub descripcion: String,
    pub grupo: String,
    pub subgrupo: String,
    pub solucion: String,
    pub marca: String,
    pub precio: String,
    pub moneda: String,
    pub promo: Option<CvaPromo>,
    /// (etiqueta original, cantidad) — p. ej. ("RETAIL_CDMX", 5).
    pub sucursales: Vec<(String, i64)>,
}

impl CvaItem {
    /// Almacén canónico para `warehouses`: retail_cdmx, cdm_mexico,
    /// ventas_guadalajara…
    pub fn warehouse_key(tag: &str) -> String {
        match tag.to_ascii_uppercase().as_str() {
            "RETAIL_CDMX" => "retail_cdmx".into(),
            "CENTRO_DE_DISTRIBUCION_MEXICO" => "cdm_mexico".into(),
            other => other.to_lowercase(),
        }
    }

    fn branch_qty(&self, tag: &str) -> u32 {
        self.sucursales
            .iter()
            .find(|(t, _)| t.eq_ignore_ascii_case(tag))
            .map(|(_, q)| (*q).max(0) as u32)
            .unwrap_or(0)
    }

    /// Existencia para ENSAMBLES: Retail CDMX + Centro de Distribución México.
    pub fn stock_ensamble(&self) -> u32 {
        self.branch_qty("RETAIL_CDMX") + self.branch_qty("CENTRO_DE_DISTRIBUCION_MEXICO")
    }

    /// Existencia para VENTA INDIVIDUAL: cualquier sucursal sirve.
    pub fn stock_individual(&self) -> u32 {
        self.sucursales.iter().map(|(_, q)| (*q).max(0) as u32).sum()
    }

    /// Sucursales como mapa de almacenes (solo cantidades > 0).
    pub fn offer_stocks(&self) -> std::collections::BTreeMap<String, u32> {
        self.sucursales
            .iter()
            .filter(|(_, q)| *q > 0)
            .map(|(tag, q)| (Self::warehouse_key(tag), (*q).max(0) as u32))
            .collect()
    }
}

/// Cotización para consultas puntuales (comando `cva_feed_query`).
#[derive(Debug, Clone, Serialize)]
pub struct FeedQuote {
    pub sku: String,
    pub description: String,
    pub group: String,
    pub brand: String,
    /// Precio efectivo: el de promoción cuando existe (decisión autorizada).
    pub price: String,
    pub currency: String,
    pub promo_price: Option<String>,
    pub promo_description: Option<String>,
    pub promo_vencimiento: Option<String>,
    pub branches: Vec<(String, i64)>,
}

pub fn get_config(conn: &rusqlite::Connection) -> Option<FeedConfig> {
    let cliente = db::get_setting(conn, SETTING_CLIENTE)?;
    Some(FeedConfig { cliente })
}

pub fn set_config(conn: &rusqlite::Connection, config: &FeedConfig) -> Result<(), String> {
    db::set_setting(conn, SETTING_CLIENTE, config.cliente.trim()).map_err(|e| e.to_string())
}

/// ¿Estamos dentro del horario de servicio (19:01–08:59, hora local)?
pub fn within_service_hours(now: chrono::DateTime<chrono::Local>) -> bool {
    use chrono::Timelike;
    let (h, m) = (now.hour(), now.minute());
    h > 19 || (h == 19 && m >= 1) || h < 8 || (h == 8 && m <= 59)
}

fn encode_param(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('&', "%26")
        .replace(' ', "%20")
}

/// Construye la URL del feed con los filtros dados (`None` = `%`).
pub fn build_url(
    cliente: &str,
    marca: Option<&str>,
    grupo: Option<&str>,
    clave: Option<&str>,
    codigo: Option<&str>,
) -> String {
    format!(
        "{}?cliente={}&marca={}&grupo={}&clave={}&codigo={}&sucursales=1&promos=1&subgpo=1",
        FEED_BASE_URL,
        encode_param(cliente),
        encode_param(marca.unwrap_or("%")),
        encode_param(grupo.unwrap_or("%")),
        encode_param(clave.unwrap_or("%")),
        encode_param(codigo.unwrap_or("%")),
    )
}

/// Descarga y parsea el feed para los filtros dados.
pub fn fetch_items(
    cliente: &str,
    marca: Option<&str>,
    grupo: Option<&str>,
    clave: Option<&str>,
    codigo: Option<&str>,
) -> Result<Vec<CvaItem>, String> {
    let url = build_url(cliente, marca, grupo, clave, codigo);
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(120))
        .call()
        .map_err(|e| format!("Error consultando CVA: {e}"))?;
    let body = resp
        .into_string()
        .map_err(|e| format!("Error leyendo la respuesta de CVA: {e}"))?;
    let items = parse_xml(&body)?;
    if items.is_empty() && !within_service_hours(chrono::Local::now()) {
        use chrono::Timelike;
        let now = chrono::Local::now();
        return Err(format!(
            "CVA devolvió 0 artículos y son las {:02}:{:02} — fuera del horario 19:01–08:59 el servicio siempre devuelve la lista vacía.",
            now.hour(),
            now.minute(),
        ));
    }
    Ok(items)
}

/// Parser tolerante del feed: acumula el texto de cada etiqueta hija de
/// `<item>`; las etiquetas numéricas no escalares son sucursales.
pub fn parse_xml(xml: &str) -> Result<Vec<CvaItem>, String> {
    const PROMO_TAGS: &[&str] = &[
        "TotalDescuento",
        "MonedaDescuento",
        "PrecioDescuento",
        "MonedaPrecioDescuento",
        "ClavePromocion",
        "DescripcionPromocion",
        "VencimientoPromocion",
        "DisponibleEnPromocion",
    ];

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut items = Vec::new();
    let mut in_item = false;
    let mut current_tag: Option<String> = None;
    let mut promo_tag: Option<String> = None;
    let mut buf = String::new();
    let mut item = CvaItem::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                if name == "item" {
                    in_item = true;
                    item = CvaItem::default();
                    current_tag = None;
                    promo_tag = None;
                } else if in_item {
                    buf.clear();
                    if PROMO_TAGS.contains(&name.as_str()) {
                        promo_tag = Some(name);
                    } else {
                        current_tag = Some(name);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if in_item && (current_tag.is_some() || promo_tag.is_some()) {
                    buf.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_string();
                if name == "item" {
                    items.push(std::mem::take(&mut item));
                    in_item = false;
                } else if in_item {
                    let text = buf.trim().to_string();
                    if let Some(pt) = promo_tag.take() {
                        assign_promo(&mut item, &pt, &text);
                    } else if let Some(ct) = current_tag.take() {
                        assign_scalar_or_branch(&mut item, &ct, &text);
                    }
                    buf.clear();
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML de CVA inválido: {e}")),
            _ => {}
        }
    }
    Ok(items
        .into_iter()
        .filter(|i| !i.codigo_fabricante.is_empty())
        .collect())
}

fn assign_scalar(item: &mut CvaItem, tag: &str, text: &str) {
    match tag.to_ascii_lowercase().as_str() {
        "clave" => item.clave = text.to_string(),
        "codigo_fabricante" | "codigo fabricante" => item.codigo_fabricante = text.to_string(),
        "descripcion" => item.descripcion = text.to_string(),
        "grupo" => item.grupo = text.to_string(),
        "subgrupo" => item.subgrupo = text.to_string(),
        "solucion" => item.solucion = text.to_string(),
        "marca" => item.marca = text.to_string(),
        "precio" => item.precio = text.to_string(),
        "moneda" => item.moneda = text.to_string(),
        _ => {}
    }
}

fn assign_scalar_or_branch(item: &mut CvaItem, tag: &str, text: &str) {
    let norm = tag.to_ascii_lowercase().replace('_', " ");
    if SCALAR_TAGS.contains(&norm.as_str()) {
        assign_scalar(item, &norm, &text);
    } else if let Ok(qty) = text.parse::<i64>() {
        item.sucursales.push((tag.to_string(), qty));
    }
}

fn assign_promo(item: &mut CvaItem, tag: &str, text: &str) {
    let promo = item.promo.get_or_insert_with(CvaPromo::default);
    let value = (!text.is_empty()).then(|| text.to_string());
    match tag {
        "ClavePromocion" => promo.clave = value,
        "DescripcionPromocion" => promo.descripcion = value,
        "VencimientoPromocion" => promo.vencimiento = value,
        "PrecioDescuento" => promo.precio_descuento = value,
        "MonedaPrecioDescuento" => promo.moneda = value,
        "DisponibleEnPromocion" => promo.disponible = text.parse::<i64>().ok(),
        _ => {}
    }
}

/// Marca canónica: recorta todo lo que va entre paréntesis («ASUS (OEM)» →
/// «ASUS») y colapsa espacios (decisión autorizada; sin tabla de alias).
pub fn canonicalize_brand(brand: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in brand.trim().chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize(text: &str) -> String {
    crate::suppliers::normalize(text)
}

/// Tipo de componente según el grupo de CVA; `None` si no es de los 11 de
/// ensamble. Coincidencia por prefijos ≥5 letras tolera plurales.
pub fn kind_for_group(grupo: &str) -> Option<ComponentKind> {
    let g_tokens: Vec<String> = normalize(grupo).split_whitespace().map(String::from).collect();
    if g_tokens.is_empty() {
        return None;
    }
    for (name, kind) in ASSEMBLY_GROUPS {
        let matched = normalize(name).split_whitespace().all(|p| {
            if p.chars().count() < 5 {
                return g_tokens.iter().any(|t| t == p);
            }
            g_tokens.iter().any(|t| t.starts_with(p) || p.starts_with(t))
        });
        if matched {
            return Some(*kind);
        }
    }
    None
}

/// ¿El subgrupo/descripción corresponde a algo que NO es de ensamble
/// (memorias USB, flash, UPS, extensiones…)? → el ítem se importa como
/// tipo «otro».
pub fn is_non_assembly(item: &CvaItem) -> bool {
    let haystack = format!(
        "{} {} {}",
        normalize(&item.subgrupo),
        normalize(&item.descripcion),
        normalize(&item.solucion),
    );
    NON_ASSEMBLY_KEYWORDS
        .iter()
        .any(|k| haystack.contains(&normalize(k)))
}

/// Moneda del feed → Currency. "Dolares" → USD, "Pesos" → MXP.
pub fn parse_moneda(moneda: &str) -> Currency {
    let m = normalize(moneda);
    if m.contains("DOLAR") || m.contains("USD") {
        Currency::Usd
    } else {
        Currency::Mxp
    }
}

/// Departamento equivalente al taxonomía de arroba: reutiliza TODA la
/// extracción de specs de `classify_full` (socket, DDR, watts, CON/SIN
/// FUENTE, capacidades…) y normaliza el departamento mostrado.
pub fn department_for_group(grupo: &str) -> &'static str {
    match kind_for_group(grupo) {
        Some(ComponentKind::Cpu) => "PROCESADORES",
        Some(ComponentKind::Motherboard) => "MOTHER BOARD",
        Some(ComponentKind::Ram) => "MEMORIA RAM",
        Some(ComponentKind::Storage) => "DISCO DURO",
        Some(ComponentKind::Monitor) => "MONITORES",
        Some(ComponentKind::Gpu) => "TARJETA DE VIDEO",
        Some(ComponentKind::Mouse) | Some(ComponentKind::Keyboard) => "PERIFERICOS",
        // Gabinetes y fuentes viven en COMPONENTES en la lista de arroba;
        // el tipo explícito manda vía kind_override.
        _ => "COMPONENTES",
    }
}

/// Convierte un `<item>` en un producto del pipeline de importación.
/// Precio efectivo = promo si hay (decisión 2); lo no-ensamble de
/// memorias/energía cae en «otro» (decisión 1); la marca se canoniza
/// (decisión 3).
pub fn to_parsed_product(item: &CvaItem) -> Result<ParsedProduct, String> {
    let group_kind = kind_for_group(&item.grupo);
    let kind = match group_kind {
        Some(k) if !is_non_assembly(item) => k,
        _ => ComponentKind::Other,
    };
    let promo_price = item
        .promo
        .as_ref()
        .and_then(|p| p.precio_descuento.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let price_raw = promo_price.unwrap_or(item.precio.trim());
    let price_amount = Decimal::from_str_exact(price_raw)
        .map_err(|_| format!("Precio inválido para {}: '{price_raw}'", item.codigo_fabricante))?;
    let currency = parse_moneda(
        if promo_price.is_some() {
            item.promo
                .as_ref()
                .and_then(|p| p.moneda.as_deref())
                .filter(|m| !m.trim().is_empty())
        } else {
            None
        }
        .unwrap_or(item.moneda.as_str()),
    );

    // Specs vía la clasificación existente sobre el departamento equivalente.
    let dept = department_for_group(&item.grupo);
    let (_kind_classified, specs) =
        crate::suppliers::classify_full(dept, &item.descripcion, &item.codigo_fabricante);

    let brand = {
        let b = canonicalize_brand(&item.marca);
        if b.is_empty() {
            "SIN MARCA".to_string()
        } else {
            b
        }
    };

    Ok(ParsedProduct {
        sku: item.codigo_fabricante.trim().to_string(),
        barcode: None,
        description: item.descripcion.trim().to_string(),
        brand,
        // Taxonomía de arroba: el maestro muestra el mismo catálogo de
        // departamentos sin importar el proveedor.
        department: dept.to_string(),
        price_amount,
        price_currency: currency,
        on_promotion: item.promo.is_some(),
        specs,
        supplier: "cva".to_string(),
        offer_stocks: item.offer_stocks(),
        stock_ensamble: item.stock_ensamble(),
        stock_individual: item.stock_individual(),
        cd_only: false,
        // El tipo del grupo de CVA manda sobre la clasificación por texto.
        kind_override: Some(kind),
    })
}

/// Consulta puntual de un SKU contra el feed.
pub fn query(conn: &rusqlite::Connection, sku: &str) -> Result<Vec<FeedQuote>, String> {
    let config = get_config(conn).unwrap_or_default();
    if !config.is_configured() {
        return Err(
            "Configura tu número de cliente de CVA en Ajustes (y da de alta tu IP en su portal)."
                .into(),
        );
    }
    let items = fetch_items(&config.cliente, None, None, None, Some(sku.trim()))?;
    Ok(items
        .iter()
        .map(|i| FeedQuote {
            sku: i.codigo_fabricante.clone(),
            description: i.descripcion.clone(),
            group: i.grupo.clone(),
            brand: i.marca.clone(),
            price: i
                .promo
                .as_ref()
                .and_then(|p| p.precio_descuento.clone())
                .unwrap_or_else(|| i.precio.clone()),
            currency: i.moneda.clone(),
            promo_price: i.promo.as_ref().and_then(|p| p.precio_descuento.clone()),
            promo_description: i.promo.as_ref().and_then(|p| p.descripcion.clone()),
            promo_vencimiento: i.promo.as_ref().and_then(|p| p.vencimiento.clone()),
            branches: i.sucursales.clone(),
        })
        .collect())
}

/// Fixture XML sintético con los dos ejemplos reales del feed que dio
/// el usuario (procesador con promo + fuente con subgrupo) más una
/// memoria USB (no-ensamble → «otro»). Solo para pruebas.
#[cfg(test)]
pub(crate) const SAMPLE_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <articulos>
        <item>
            <clave>CP-1283</clave>
            <EsProductoESD>false</EsProductoESD>
            <codigo_fabricante>100-100000457BOX</codigo_fabricante>
            <descripcion>PROCESADOR AMD RYZEN 5 5500 S-AM4 5A GEN / 3.6 - 4.2 GHZ / CACHE 16MB / 6 NUCLEOS / SIN GRAFICOS / CON DISIPADOR / GAMER MEDIO</descripcion>
            <solucion>COMPONENTES ( OEM )</solucion>
            <grupo>PROCESADORES</grupo>
            <marca>AMD</marca>
            <garantia>1 ANIO</garantia>
            <clase>AC</clase>
            <disponible>0</disponible>
            <precio>133.73</precio>
            <moneda>Dolares</moneda>
            <ficha_tecnica></ficha_tecnica>
            <ficha_comercial></ficha_comercial>
            <imagen>http://www.grupocva.com/detalle_articulo/10312566.jpg</imagen>
            <disponibleCD>0</disponibleCD>
            <TotalDescuento></TotalDescuento>
            <MonedaDescuento>Dolares</MonedaDescuento>
            <PrecioDescuento>72.63</PrecioDescuento>
            <MonedaPrecioDescuento>Dolares</MonedaPrecioDescuento>
            <ClavePromocion>759764</ClavePromocion>
            <DescripcionPromocion>PRECIO ESPECIAL EN PROCESADOR AMD RYZEN 5 5500 AM4</DescripcionPromocion>
            <VencimientoPromocion>Por Cantidad</VencimientoPromocion>
            <DisponibleEnPromocion>101</DisponibleEnPromocion>
            <TipoProducto>NORMAL</TipoProducto>
            <IdDepartamento>823</IdDepartamento>
            <Departamento>C</Departamento>
            <CENTRO_DE_DISTRIBUCION_MEXICO>856</CENTRO_DE_DISTRIBUCION_MEXICO>
            <CENTRO_DE_DISTRIBUCION_MONTERREY>0</CENTRO_DE_DISTRIBUCION_MONTERREY>
            <RETAIL_CDMX>0</RETAIL_CDMX>
            <VENTAS_GUADALAJARA>7</VENTAS_GUADALAJARA>
        </item>
        <item>
            <clave>CSP-518</clave>
            <codigo_fabricante>GP-P450B</codigo_fabricante>
            <descripcion>FUENTE DE PODER GIGABYTE 450W /80 PLUS BRONZE/NEGRO/ATX/NO MODULAR</descripcion>
            <solucion>COMPONENTES ( OEM )</solucion>
            <grupo>ENERGIA</grupo>
            <subgrupo>FUENTES DE PODER</subgrupo>
            <marca>GIGABYTE (OEM)</marca>
            <garantia>1 ANIO</garantia>
            <clase>AC</clase>
            <disponible>0</disponible>
            <precio>29.34</precio>
            <moneda>Dolares</moneda>
            <disponibleCD>0</disponibleCD>
            <PrecioDescuento>29.34</PrecioDescuento>
            <ClavePromocion>720684</ClavePromocion>
            <DescripcionPromocion>LIMITADA A UNIDADES</DescripcionPromocion>
            <VencimientoPromocion>Por Cantidad</VencimientoPromocion>
            <DisponibleEnPromocion>2</DisponibleEnPromocion>
            <TipoProducto>NORMAL</TipoProducto>
            <IdDepartamento>823</IdDepartamento>
            <Departamento>C</Departamento>
            <RETAIL_CDMX>4</RETAIL_CDMX>
        </item>
        <item>
            <clave>MEM-USB</clave>
            <codigo_fabricante>USB-128</codigo_fabricante>
            <descripcion>MEMORIA USB KINGSTON 128GB DATA TRAVELER</descripcion>
            <grupo>MEMORIAS</grupo>
            <subgrupo>MEMORIAS USB</subgrupo>
            <marca>KINGSTON</marca>
            <precio>10.00</precio>
            <moneda>Dolares</moneda>
            <RETAIL_CDMX>50</RETAIL_CDMX>
        </item>
    </articulos>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    use crate::domain::component::Specs;

    #[test]
    fn parses_items_promos_and_branches() {
        let items = parse_xml(SAMPLE_FEED).unwrap();
        assert_eq!(items.len(), 3);

        let cpu = &items[0];
        assert_eq!(cpu.codigo_fabricante, "100-100000457BOX");
        assert_eq!(cpu.grupo, "PROCESADORES");
        assert_eq!(cpu.precio, "133.73");
        let promo = cpu.promo.as_ref().unwrap();
        assert_eq!(promo.precio_descuento.as_deref(), Some("72.63"));
        assert_eq!(promo.disponible, Some(101));
        assert_eq!(promo.vencimiento.as_deref(), Some("Por Cantidad"));
        assert_eq!(cpu.stock_ensamble(), 856, "retail_cdmx 0 + cdm 856");
        assert_eq!(cpu.stock_individual(), 856 + 7, "todas las sucursales");
        // Etiquetas escalares numéricas NO son sucursales.
        assert!(!cpu
            .sucursales
            .iter()
            .any(|(t, _)| t == "IdDepartamento" || t == "DisponibleEnPromocion" || t == "disponible"));
    }

    #[test]
    fn classification_and_conversion() {
        let items = parse_xml(SAMPLE_FEED).unwrap();

        let cpu = to_parsed_product(&items[0]).unwrap();
        assert_eq!(cpu.kind_override, Some(ComponentKind::Cpu));
        assert_eq!(cpu.brand, "AMD");
        // Precio efectivo = promo (decisión 2).
        assert_eq!(cpu.price_amount, Decimal::from_str_exact("72.63").unwrap());
        assert!(cpu.on_promotion);
        assert_eq!(cpu.stock_ensamble, 856);
        assert_eq!(cpu.stock_individual, 863);
        assert_eq!(cpu.offer_stocks.get("cdm_mexico"), Some(&856));
        assert_eq!(cpu.offer_stocks.get("ventas_guadalajara"), Some(&7));
        assert_eq!(cpu.department, "PROCESADORES");

        let psu = to_parsed_product(&items[1]).unwrap();
        assert_eq!(psu.kind_override, Some(ComponentKind::Psu), "energia con subgrupo fuente");
        // Marca canónica: sin «(OEM)» (decisión 3).
        assert_eq!(psu.brand, "GIGABYTE");
        assert_eq!(psu.specs.wattage, Some(450), "watts extraídos de la descripción");
        assert_eq!(psu.offer_stocks.get("retail_cdmx"), Some(&4));

        // Memoria USB: importada como «otro» (decisión 1).
        let usb = to_parsed_product(&items[2]).unwrap();
        assert_eq!(usb.kind_override, Some(ComponentKind::Other));
    }

    #[test]
    fn brand_rule_and_helpers() {
        assert_eq!(canonicalize_brand("ASUS (OEM)"), "ASUS");
        assert_eq!(canonicalize_brand("  GIGABYTE  ( OEM ) "), "GIGABYTE");
        assert_eq!(canonicalize_brand("AMD"), "AMD");
        assert_eq!(canonicalize_brand(""), "");

        assert!(within_service_hours(chrono::Local::now().with_hour(20).unwrap()));
        assert!(!within_service_hours(chrono::Local::now().with_hour(13).unwrap()));

        assert_eq!(parse_moneda("Dolares"), Currency::Usd);
        assert_eq!(parse_moneda("Pesos"), Currency::Mxp);
    }

    #[test]
    fn group_matching_tolerates_plurals() {
        assert_eq!(kind_for_group("PROCESADORES"), Some(ComponentKind::Cpu));
        assert_eq!(kind_for_group("Video"), Some(ComponentKind::Gpu));
        assert_eq!(kind_for_group("TARJETAS MADRE"), Some(ComponentKind::Motherboard));
        assert_eq!(kind_for_group("TARJETA MADRE"), Some(ComponentKind::Motherboard));
        assert_eq!(kind_for_group("TECLADO Y MOUSE"), Some(ComponentKind::KeyboardMouseKit));
        assert_eq!(kind_for_group("TECLADOS"), Some(ComponentKind::Keyboard));
        assert_eq!(kind_for_group("GABINETES"), Some(ComponentKind::Case));
        assert_eq!(kind_for_group("MEMORIAS"), Some(ComponentKind::Ram));
        assert_eq!(kind_for_group("DISCOS DUROS"), Some(ComponentKind::Storage));
        assert_eq!(kind_for_group("ENERGIA"), Some(ComponentKind::Psu));
        // «impresoras» no es grupo de ensamble.
        assert_eq!(kind_for_group("IMPRESORAS"), None);
    }

    #[test]
    fn url_building() {
        let url = build_url(
            "74247",
            Some("HP"),
            Some("%impresora%"),
            None,
            Some("LS22F350FHLXZX"),
        );
        assert!(url.starts_with(FEED_BASE_URL));
        assert!(url.contains("cliente=74247"));
        assert!(url.contains("marca=HP"));
        assert!(url.contains("grupo=%25impresora%25"));
        assert!(url.contains("clave=%25"));
        assert!(url.contains("codigo=LS22F350FHLXZX"));
        assert!(url.ends_with("&sucursales=1&promos=1&subgpo=1"));
    }

    #[test]
    fn empty_feed_parses_to_zero_items() {
        let empty = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><articulos></articulos>";
        assert!(parse_xml(empty).unwrap().is_empty());
        let _ = Specs::default();
    }
}
