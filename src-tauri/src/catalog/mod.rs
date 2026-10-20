//! Catálogo de componentes: semilla embebida de la lista de precios +
//! componentes capturados manualmente (p. ej. usados).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::assembly::ComponentLookup;
use crate::domain::component::{
    Component, ComponentKind, ComponentSource, Condition, Specs,
};
use crate::domain::money::{Currency, ExchangeRate, Money};
use std::collections::BTreeMap;

/// Estructura del archivo data/catalog_seed.json generado por
/// scripts/extract_catalog.py a partir del xlsx de la lista de precios.
#[derive(Debug, Deserialize)]
pub struct CatalogSeed {
    pub source_file: String,
    pub list_date: String,
    pub exchange_rate: String,
    #[serde(default)]
    pub exchange_rule: String,
    pub components: Vec<Component>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("Semilla del catálogo inválida: {0}")]
    InvalidSeed(String),
    #[error("Componente no encontrado: {0}")]
    NotFound(String),
    #[error("El precio es obligatorio y debe ser mayor o igual a cero")]
    InvalidPrice,
}

/// Entrada para dar de alta un componente manualmente (usado o nuevo).
#[derive(Debug, Deserialize)]
pub struct NewComponentInput {
    pub description: String,
    #[serde(default)]
    pub sku: Option<String>,
    #[serde(default)]
    pub brand: Option<String>,
    pub kind: ComponentKind,
    /// Precio tal como se capturó, con su moneda.
    pub price_amount: String,
    pub price_currency: Currency,
    /// Los componentes manuales suelen ser usados, pero se admite nuevo.
    #[serde(default = "default_condition")]
    pub condition: Condition,
    #[serde(default)]
    pub specs: Specs,
}

fn default_condition() -> Condition {
    Condition::Usado
}

/// Catálogo completo: componentes de la lista + manuales, con índice por id.
#[derive(Debug, Serialize)]
pub struct Catalog {
    pub source_file: String,
    pub list_date: String,
    pub exchange_rate: ExchangeRate,
    /// Texto de la regla de conversión tal como viene en la semilla.
    pub exchange_rule: String,
    pub components: Vec<Component>,
    #[serde(skip)]
    index: HashMap<Uuid, usize>,
}

impl Catalog {
    /// Construye el catálogo desde productos ya persistidos (SQLite).
    pub fn from_parts(
        mut products: Vec<Component>,
        exchange_rate: ExchangeRate,
        exchange_rule: &str,
        source_file: String,
        list_date: String,
    ) -> Self {
        products.sort_by(|a, b| {
            a.kind
                .label()
                .cmp(b.kind.label())
                .then(a.description.cmp(&b.description))
        });
        let mut catalog = Self {
            source_file,
            list_date,
            exchange_rate,
            exchange_rule: exchange_rule.to_string(),
            components: products,
            index: HashMap::new(),
        };
        catalog.reindex();
        catalog
    }

    pub fn from_seed_json(json: &str) -> Result<Self, CatalogError> {
        let seed: CatalogSeed = serde_json::from_str(json)
            .map_err(|e| CatalogError::InvalidSeed(e.to_string()))?;
        let rate: f64 = seed
            .exchange_rate
            .parse()
            .map_err(|_| CatalogError::InvalidSeed("T.C no numérico".into()))?;
        let rate = rust_decimal::Decimal::try_from(rate)
            .map_err(|e| CatalogError::InvalidSeed(format!("T.C fuera de rango: {e}")))?;

        let mut components = seed.components;
        components.sort_by(|a, b| a.kind.label().cmp(b.kind.label()).then(a.description.cmp(&b.description)));

        let mut catalog = Self {
            source_file: seed.source_file,
            list_date: seed.list_date,
            exchange_rate: ExchangeRate::new(rate),
            exchange_rule: seed.exchange_rule,
            components,
            index: HashMap::new(),
        };
        catalog.reindex();
        Ok(catalog)
    }

    fn reindex(&mut self) {
        self.index = self
            .components
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i))
            .collect();
    }

    /// Reemplaza el conjunto de componentes manuales por los dados
    /// (los de la lista de precios permanecen intactos).
    pub fn merge_manual(&mut self, manual: Vec<Component>) {
        self.components.retain(|c| c.source != ComponentSource::Manual);
        self.components.extend(manual);
        self.components
            .sort_by(|a, b| a.kind.label().cmp(b.kind.label()).then(a.description.cmp(&b.description)));
        self.reindex();
    }

    /// Da de alta un componente manual y lo devuelve.
    pub fn add_manual(&mut self, input: NewComponentInput) -> Result<Component, CatalogError> {
        if input.description.trim().is_empty() {
            return Err(CatalogError::NotFound("La descripción es obligatoria".into()));
        }
        let amount = rust_decimal::Decimal::from_str_exact(&input.price_amount)
            .map_err(|_| CatalogError::InvalidPrice)?;
        if amount < rust_decimal::Decimal::ZERO {
            return Err(CatalogError::InvalidPrice);
        }

        let component = Component {
            id: Uuid::new_v4(),
            sku: input.sku.unwrap_or_else(|| format!("MANUAL-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"))),
            barcode: None,
            description: input.description.trim().to_string(),
            brand: input.brand.unwrap_or_else(|| "SIN MARCA".into()),
            department: "CAPTURA MANUAL".into(),
            kind: input.kind,
            condition: input.condition,
            price: Money::new(amount, input.price_currency),
            on_promotion: false,
            offer_stocks: BTreeMap::new(),
            source: ComponentSource::Manual,
            specs: input.specs,
            supplier: String::new(),
            stock_ensamble_total: 0,
            stock_individual_total: 0,
            cd_only_best: false,
            offers: Vec::new(),
        };
        self.components.push(component.clone());
        self.reindex();
        Ok(component)
    }

    pub fn remove_manual(&mut self, id: Uuid) -> bool {
        let before = self.components.len();
        self.components
            .retain(|c| !(c.id == id && c.source == ComponentSource::Manual));
        let removed = before != self.components.len();
        if removed {
            self.reindex();
        }
        removed
    }

    /// Componentes manuales (para persistencia).
    pub fn manual_components(&self) -> Vec<Component> {
        self.components
            .iter()
            .filter(|c| c.source == ComponentSource::Manual)
            .cloned()
            .collect()
    }

    /// Búsqueda por texto libre (SKU, descripción y marca) y opcionalmente
    /// por tipo. Los de tipo Other solo aparecen si se piden explícitamente.
    pub fn search(&self, kind: Option<ComponentKind>, query: &str) -> Vec<&Component> {
        let q = query.trim().to_uppercase();
        self.components
            .iter()
            .filter(|c| {
                let kind_ok = match kind {
                    None => c.kind != ComponentKind::Other,
                    Some(k) => c.kind == k,
                };
                if !kind_ok {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                c.sku.to_uppercase().contains(&q)
                    || c.description.to_uppercase().contains(&q)
                    || c.brand.to_uppercase().contains(&q)
            })
            .collect()
    }

    /// Conteo por tipo de componente.
    pub fn counts_by_kind(&self) -> HashMap<String, usize> {
        let mut map = HashMap::new();
        for c in &self.components {
            let key = serde_json::to_value(c.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            *map.entry(key).or_default() += 1;
        }
        map
    }

    pub fn set_exchange_rate(&mut self, rate: ExchangeRate) {
        self.exchange_rate = rate;
    }
}

impl ComponentLookup for Catalog {
    fn get_component(&self, id: &Uuid) -> Option<&Component> {
        self.index.get(id).map(|&i| &self.components[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const MINIMAL_SEED: &str = r#"{
        "source_file": "TEST.xlsx",
        "list_date": "2026-10-15",
        "exchange_rate": "18.52",
        "components": [
            {
                "id": "4a1f9da7-5ee6-5dde-b50c-533573c64423",
                "sku": "CPU-1",
                "barcode": null,
                "description": "PROCESADOR TEST",
                "brand": "AMD",
                "department": "PROCESADORES",
                "kind": "cpu",
                "condition": "nuevo",
                "price": {"amount": "217.36", "currency": "USD"},
                "on_promotion": false,
                "stock": {"a_cen": 10, "ave_2": 0, "cedis": 0, "cmal": 0, "gdl": 0},
                "source": "lista",
                "specs": {"includes_cooler": false, "socket": "AM4"}
            },
            {
                "id": "0e2d0f5e-9ad1-5c65-b06b-a3032ac2f001",
                "sku": "CASE-1",
                "barcode": null,
                "description": "GABINETE CON FUENTE 600W",
                "brand": "EVOTEC",
                "department": "COMPONENTES",
                "kind": "case",
                "condition": "nuevo",
                "price": {"amount": "41.60", "currency": "MXP"},
                "on_promotion": false,
                "stock": {"a_cen": 0, "ave_2": 0, "cedis": 0, "cmal": 0, "gdl": 0},
                "source": "lista",
                "specs": {"includes_psu": true, "psu_watts": 600}
            }
        ]
    }"#;

    #[test]
    fn loads_seed_and_ceils_rate() {
        let cat = Catalog::from_seed_json(MINIMAL_SEED).unwrap();
        assert_eq!(cat.components.len(), 2);
        assert_eq!(cat.exchange_rate.rate, dec!(18.52));
        assert_eq!(cat.exchange_rate.effective(), dec!(19));
        assert_eq!(cat.list_date, "2026-10-15");
    }

    #[test]
    fn lookup_by_id_and_search() {
        let cat = Catalog::from_seed_json(MINIMAL_SEED).unwrap();
        let id = Uuid::parse_str("4a1f9da7-5ee6-5dde-b50c-533573c64423").unwrap();
        let c = cat.get_component(&id).expect("debe existir");
        assert_eq!(c.kind, ComponentKind::Cpu);
        assert_eq!(c.specs.includes_cooler, Some(false));

        let found = cat.search(Some(ComponentKind::Case), "fuente");
        assert_eq!(found.len(), 1);
        let none = cat.search(Some(ComponentKind::Cpu), "evotec");
        assert!(none.is_empty());
    }

    #[test]
    fn add_manual_used_component() {
        let mut cat = Catalog::from_seed_json(MINIMAL_SEED).unwrap();
        let before = cat.components.len();
        let c = cat
            .add_manual(NewComponentInput {
                description: "RTX 3060 usada".into(),
                sku: Some("USADA-1".into()),
                brand: Some("ASUS".into()),
                kind: ComponentKind::Gpu,
                price_amount: "3500".into(),
                price_currency: Currency::Mxp,
                condition: Condition::Usado,
                specs: Specs::default(),
            })
            .unwrap();
        assert_eq!(cat.components.len(), before + 1);
        assert_eq!(c.condition, Condition::Usado);
        assert_eq!(c.source, ComponentSource::Manual);
        assert!(cat.get_component(&c.id).is_some());

        assert!(cat.remove_manual(c.id));
        assert!(cat.get_component(&c.id).is_none());
    }

    #[test]
    fn merge_manual_roundtrip() {
        let mut cat = Catalog::from_seed_json(MINIMAL_SEED).unwrap();
        let c = cat
            .add_manual(NewComponentInput {
                description: "Fuente usada".into(),
                sku: None,
                brand: None,
                kind: ComponentKind::Psu,
                price_amount: "500".into(),
                price_currency: Currency::Mxp,
                condition: Condition::Usado,
                specs: Specs::default(),
            })
            .unwrap();
        let manual = cat.manual_components();
        assert_eq!(manual.len(), 1);

        // Recargar: semilla + merge de manuales
        let mut reloaded = Catalog::from_seed_json(MINIMAL_SEED).unwrap();
        assert_eq!(reloaded.components.len(), 2);
        reloaded.merge_manual(manual);
        assert_eq!(reloaded.components.len(), 3);
        assert!(reloaded.get_component(&c.id).is_some());
    }

    #[test]
    fn counts_by_kind_uses_snake_keys() {
        let cat = Catalog::from_seed_json(MINIMAL_SEED).unwrap();
        let counts = cat.counts_by_kind();
        assert_eq!(counts.get("cpu"), Some(&1));
        assert_eq!(counts.get("case"), Some(&1));
    }

    /// La semilla real (data/catalog_seed.json, generada por el script de
    /// Python) debe deserializar sin errores y cumplir lo esperado.
    #[test]
    fn real_embedded_seed_loads() {
        let json = include_str!("../../../data/catalog_seed.json");
        let cat = Catalog::from_seed_json(json).expect("la semilla real debe cargar");
        assert!(cat.components.len() >= 600, "componentes: {}", cat.components.len());
        assert_eq!(cat.exchange_rate.rate, dec!(18.52));
        assert_eq!(cat.exchange_rate.effective(), dec!(19));
        assert_eq!(cat.list_date, "2026-10-15");

        // Hay al menos un gabinete con fuente incluida (EV-1015, 600W)
        let with_psu: Vec<_> = cat
            .components
            .iter()
            .filter(|c| c.kind == ComponentKind::Case && c.specs.includes_psu == Some(true))
            .collect();
        assert!(!with_psu.is_empty());
        assert!(with_psu[0].specs.psu_watts.is_some());

        // Todos los componentes de lista son nuevos
        assert!(cat
            .components
            .iter()
            .all(|c| c.condition == Condition::Nuevo));

        // Las búsquedas por tipo funcionan sobre el catálogo real
        assert_eq!(cat.search(Some(ComponentKind::Cpu), "").len(), 24);
        assert!(cat.search(None, "RYZEN").len() >= 5);
    }
}
