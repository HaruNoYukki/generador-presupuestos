//! Venta individual (flujo 2): carrito de cualquier producto del catálogo.
//!
//! A diferencia de un ensamble, no hay slots mínimos ni confirmación: el
//! carrito es la sesión de venta directa. Las reglas económicas son las
//! mismas (ganancia %, IVA recalculado con el objetivo) vía profit.rs.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::component::{Component, Stock};
use crate::domain::money::ExchangeRate;
use crate::domain::profit::{self, ProfitLine, ProfitReport};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CartItem {
    pub component_id: Uuid,
    pub quantity: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cart {
    pub items: Vec<CartItem>,
}

impl Cart {
    pub fn set_quantity(&mut self, component_id: Uuid, quantity: u32) {
        self.items.retain(|i| i.component_id != component_id);
        if quantity > 0 {
            self.items.push(CartItem {
                component_id,
                quantity,
            });
        }
    }

    pub fn quantity_of(&self, component_id: Uuid) -> u32 {
        self.items
            .iter()
            .find(|i| i.component_id == component_id)
            .map(|i| i.quantity)
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Disponibilidad para vender (flujo individual) de un producto:
/// totales entre proveedores, ya calculados por la base.
pub fn sellable_stock(c: &Component) -> u32 {
    c.stock_individual_total
}

/// Calcula el reporte de ganancia del carrito. Los componentes faltantes
/// (eliminados del catálogo) se ignoran.
pub fn compute_cart_profit(
    items: &[CartItem],
    lookup: &dyn Fn(&Uuid) -> Option<Component>,
    rate: &ExchangeRate,
    percent: Decimal,
) -> ProfitReport {
    let mut pairs: Vec<(Component, u32)> = Vec::new();
    for item in items {
        if let Some(c) = lookup(&item.component_id) {
            pairs.push((c, item.quantity));
        }
    }
    let lines: Vec<ProfitLine> =
        profit::profit_lines_from_pairs(&pairs, rate, percent);
    profit::report_from_lines(percent, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::component::{
        ComponentKind, ComponentSource, Condition, Specs,
    };
    use crate::domain::money::{Currency, Money};
    use rust_decimal_macros::dec;
    use std::collections::{BTreeMap, HashMap};

    fn product(sku: &str, a_cen: u32, ave_2: u32, cedis: u32, cmal: u32, gdl: u32) -> Component {
        Component {
            id: Uuid::new_v4(),
            sku: sku.into(),
            barcode: None,
            description: format!("PRODUCTO {sku}"),
            brand: "TEST".into(),
            department: "TEST".into(),
            kind: ComponentKind::Ram,
            condition: Condition::Nuevo,
            price: Money::new(dec!(100), Currency::Mxp),
            on_promotion: false,
            offer_stocks: {
                let mut m = BTreeMap::new();
                for (w, q) in [("a_cen", a_cen), ("ave_2", ave_2), ("cedis", cedis), ("cmal", cmal), ("gdl", gdl)] {
                    if q > 0 {
                        m.insert(w.into(), q);
                    }
                }
                m
            },
            source: ComponentSource::Lista,
            supplier: "arroba".into(),
            stock_ensamble_total: a_cen + ave_2 + cedis,
            stock_individual_total: a_cen + ave_2 + cedis,
            cd_only_best: false,
            offers: vec![],
            specs: Specs::default(),
        }
    }

    fn rate() -> ExchangeRate {
        ExchangeRate::new(dec!(18.52))
    }

    #[test]
    fn sellable_stock_only_counts_cen_ave2_cedis() {
        let p = product("X", 5, 3, 7, 99, 99);
        assert_eq!(sellable_stock(&p), 15, "CMAL y GDL no cuentan");
        assert_eq!(p.stock_individual_total, 15);
    }

    #[test]
    fn cart_profit_matches_component_rules() {
        let mut catalog = HashMap::new();
        let ram = product("RAM-1", 10, 0, 0, 0, 0);
        catalog.insert(ram.id, ram.clone());
        let lookup = |id: &Uuid| catalog.get(id).cloned();

        let mut cart = Cart::default();
        cart.set_quantity(ram.id, 2);
        let mut report = compute_cart_profit(&cart.items, &lookup, &rate(), dec!(20));
        // costo 100×2 = 200; ganancia 20% = 40; IVA = 240×16% = 38.40
        assert_eq!(report.lines.len(), 1);
        assert_eq!(report.totals.cost, dec!(200.00));
        assert_eq!(report.totals.base_profit, dec!(40.00));
        assert_eq!(report.totals.iva, dec!(38.40));
        assert_eq!(report.totals.price, dec!(278.40));

        // Con precio objetivo el total cierra y Subtotal+IVA=Total
        profit::apply_target(&mut report, dec!(300));
        assert_eq!(report.totals.price, dec!(300.00));
        assert_eq!(
            report.totals.cost + report.totals.final_profit + report.totals.iva,
            report.totals.price
        );
    }

    #[test]
    fn cart_quantity_zero_removes_item() {
        let mut cart = Cart::default();
        let id = Uuid::new_v4();
        cart.set_quantity(id, 3);
        assert_eq!(cart.quantity_of(id), 3);
        cart.set_quantity(id, 0);
        assert!(cart.is_empty());
    }
}
