//! Componentes de cómputo del catálogo.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use super::money::{ExchangeRate, Money};

/// Tipo de componente. Distingue los roles que puede jugar dentro de un
/// ensamble (slot) y los subtipos que la lista de precios maneja por
/// departamento + descripción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Cpu,
    Motherboard,
    Ram,
    Storage,
    Psu,
    Case,
    CoolerAir,
    CoolerLiquid,
    Gpu,
    Monitor,
    Keyboard,
    Mouse,
    KeyboardMouseKit,
    ExpansionCard,
    Other,
}

impl ComponentKind {
    /// Etiqueta en español para la interfaz.
    pub fn label(&self) -> &'static str {
        match self {
            ComponentKind::Cpu => "Procesador",
            ComponentKind::Motherboard => "Tarjeta madre",
            ComponentKind::Ram => "Memoria RAM",
            ComponentKind::Storage => "Almacenamiento",
            ComponentKind::Psu => "Fuente de poder",
            ComponentKind::Case => "Gabinete",
            ComponentKind::CoolerAir => "Enfriamiento por aire",
            ComponentKind::CoolerLiquid => "Enfriamiento líquido",
            ComponentKind::Gpu => "Tarjeta de video",
            ComponentKind::Monitor => "Monitor",
            ComponentKind::Keyboard => "Teclado",
            ComponentKind::Mouse => "Mouse",
            ComponentKind::KeyboardMouseKit => "Kit teclado + mouse",
            ComponentKind::ExpansionCard => "Tarjeta de expansión",
            ComponentKind::Other => "Otro",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }

    pub fn is_cooler(&self) -> bool {
        matches!(self, ComponentKind::CoolerAir | ComponentKind::CoolerLiquid)
    }
}

/// Condición del componente: la lista de precios trae productos nuevos;
/// los usados se capturan manualmente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    Nuevo,
    Usado,
}

impl Condition {
    /// Etiqueta en español (la usará el módulo de reportes, fase 2).
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Condition::Nuevo => "Nuevo",
            Condition::Usado => "Usado",
        }
    }
}

/// Origen del componente dentro del catálogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentSource {
    Lista,
    Manual,
}

/// Existencias por almacén (columnas de la lista de precios).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stock {
    #[serde(default)]
    pub a_cen: u32,
    #[serde(default)]
    pub ave_2: u32,
    #[serde(default)]
    pub cedis: u32,
    #[serde(default)]
    pub cmal: u32,
    #[serde(default)]
    pub gdl: u32,
}

impl Stock {
    /// Inventario relevante para vender: únicamente CEN (A-CEN), AVE-2 y
    /// CEDIS. Los demás almacenes (CMAL, GDL) no cuentan como disponibles.
    pub fn sellable(&self) -> u32 {
        self.a_cen + self.ave_2 + self.cedis
    }

    /// Total de unidades en todos los almacenes (informativo).
    #[allow(dead_code)]
    pub fn total(&self) -> u32 {
        self.a_cen + self.ave_2 + self.cedis + self.cmal + self.gdl
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    Ssd,
    Hdd,
}

/// Especificaciones por tipo de componente. Es una estructura plana con
/// campos opcionales: cada tipo usa los que le corresponden.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Specs {
    // CPU
    pub socket: Option<String>,
    /// `Some(true)`: el procesador trae disipador (BOX).
    /// `Some(false)`: "SIN DISIPADOR" / "NO INCLUYE DISIPADOR" / SKU WOF.
    /// `None`: la descripción no lo aclara.
    pub includes_cooler: Option<bool>,

    // Tarjeta madre
    pub form_factor: Option<String>,

    // RAM
    pub ddr: Option<String>,
    pub capacity_gb: Option<u16>,
    /// DIMM o SODIMM.
    pub ram_form: Option<String>,

    // Fuente de poder
    pub wattage: Option<u16>,

    // Gabinete
    /// El gabinete incluye fuente de poder.
    pub includes_psu: Option<bool>,
    pub psu_watts: Option<u16>,

    // Almacenamiento
    pub storage_kind: Option<StorageKind>,
    pub internal: Option<bool>,
    /// Capacidad tal como aparece en la descripción, p.ej. "480GB", "2TB".
    pub capacity_raw: Option<String>,
    // capacity_gb compartido con RAM

    // GPU
    pub vram: Option<String>,

    // Monitor
    pub inches: Option<String>,

    // Tarjeta de expansión
    pub card_type: Option<String>,
}

/// Una oferta de un SKU por parte de un proveedor. El precio y la
/// disponibilidad viven aquí, no en el maestro: varios proveedores pueden
/// ofrecer el mismo SKU a distintos precios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub supplier: String,
    pub price: Money,
    pub on_promotion: bool,
    /// Existencias por almacén del proveedor (3FN: tabla offer_stock).
    /// Claves: 'a_cen','ave_2','cedis','cmal','gdl' (arroba) o
    /// 'disp','disp_cd' (CVA).
    pub stocks: BTreeMap<String, u32>,
    /// Etiqueta legible del desglose, construida por la base.
    pub stock_label: String,
    /// Existencia elegible para ENSAMBLES (arroba: CEN+AVE-2+CEDIS; CVA: Disp).
    pub stock_ensamble: u32,
    /// Existencia para VENTA INDIVIDUAL (arroba: igual que ensamble;
    /// CVA: Disp + Disp CD).
    pub stock_individual: u32,
    /// Solo tiene disponibilidad en CD: no usable en ensambles.
    pub cd_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub id: Uuid,
    pub sku: String,
    pub barcode: Option<String>,
    pub description: String,
    pub brand: String,
    pub department: String,
    pub kind: ComponentKind,
    pub condition: Condition,
    /// Precio de la MEJOR oferta (menor precio con disponibilidad).
    pub price: Money,
    pub on_promotion: bool,
    /// Desglose por almacén de la oferta ganadora.
    #[serde(default)]
    pub offer_stocks: BTreeMap<String, u32>,
    pub source: ComponentSource,
    #[serde(default)]
    pub specs: Specs,
    /// Proveedor de la oferta ganadora: 'arroba' | 'cva' | 'manual'.
    #[serde(default)]
    pub supplier: String,
    /// Existencia total para ensambles (suma de ofertas).
    #[serde(default)]
    pub stock_ensamble_total: u32,
    /// Existencia total para venta individual (suma de ofertas).
    #[serde(default)]
    pub stock_individual_total: u32,
    /// La mejor oferta es solo-CD (marca obligatoria en la selección).
    #[serde(default)]
    pub cd_only_best: bool,
    /// Desglose de inventario por proveedor.
    #[serde(default)]
    pub offers: Vec<Offer>,
}

impl Component {
    /// Precio del componente en pesos según el tipo de cambio dado.
    pub fn price_mxp(&self, rate: &ExchangeRate) -> Money {
        self.price.to_mxp(rate)
    }

    /// Descripción de la fuente incluida, si el gabinete trae una.
    pub fn included_psu_spec(&self) -> Option<String> {
        if self.kind != ComponentKind::Case || self.specs.includes_psu != Some(true) {
            return None;
        }
        Some(match self.specs.psu_watts {
            Some(w) => format!("Fuente incluida de {w}W"),
            None => "Fuente incluida (watts no especificados)".to_string(),
        })
    }
}
