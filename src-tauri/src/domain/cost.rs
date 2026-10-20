//! Cálculo de costo de un ensamble.
//!
//! Regla de negocio (fase 2): todos los precios se convierten a pesos
//! (USD→MXP con el tipo de cambio efectivo, redondeado hacia arriba al
//! entero superior), se suman para obtener el subtotal; sobre el subtotal
//! se calcula el IVA de 16%. El costo del ensamble es subtotal + IVA.

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::{Deserialize, Serialize};

use super::assembly::{Assembly, ComponentLookup};
use super::money::{ExchangeRate, Money};

/// IVA vigente en México: 16%.
pub const IVA_RATE: Decimal = rust_decimal_macros::dec!(0.16);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    /// Suma de precios de todas las partidas, en pesos.
    pub subtotal: Money,
    /// IVA de 16% calculado sobre el subtotal, en pesos.
    pub iva: Money,
    /// Costo del ensamble: subtotal + IVA, en pesos.
    pub total: Money,
}

/// Redondeo comercial a 2 decimales (mitad hacia arriba).
fn round2(d: Decimal) -> Decimal {
    d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

/// Suma el costo de todas las partidas del ensamble (cantidad × precio en
/// MXN), calcula el IVA y devuelve el desglose. Los componentes que ya no
/// existen en el catálogo se ignoran (el validador los reporta aparte).
pub fn compute_cost<L: ComponentLookup>(
    assembly: &Assembly,
    catalog: &L,
    rate: &ExchangeRate,
) -> CostBreakdown {
    let mut subtotal = Decimal::ZERO;
    for items in assembly.slots.values() {
        for item in items {
            if let Some(component) = catalog.get_component(&item.component_id) {
                subtotal += component.price_mxp(rate).amount * Decimal::from(item.quantity);
            }
        }
    }
    let subtotal = round2(subtotal);
    let iva = round2(subtotal * IVA_RATE);
    CostBreakdown {
        subtotal: Money::mxp(subtotal),
        iva: Money::mxp(iva),
        total: Money::mxp(subtotal + iva),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assembly::Slot;
    use crate::domain::component::{
        Component, ComponentKind, ComponentSource, Condition, Specs, Stock,
    };
    use rust_decimal_macros::dec;
    use std::collections::{BTreeMap, HashMap};
    use uuid::Uuid;

    fn comp(id: Uuid, kind: ComponentKind, amount: Decimal, currency: crate::domain::money::Currency) -> Component {
        Component {
            id,
            sku: format!("SKU-{id}"),
            barcode: None,
            description: format!("COMPONENTE {kind:?}"),
            brand: "TEST".into(),
            department: "TEST".into(),
            kind,
            condition: Condition::Nuevo,
            price: Money::new(amount, currency),
            on_promotion: false,
            offer_stocks: BTreeMap::new(),
            source: ComponentSource::Lista,
            supplier: "arroba".into(),
            stock_ensamble_total: 20,
            stock_individual_total: 20,
            cd_only_best: false,
            offers: vec![],
            specs: Specs::default(),
        }
    }

    struct FakeCatalog {
        map: HashMap<Uuid, Component>,
    }

    impl ComponentLookup for FakeCatalog {
        fn get_component(&self, id: &Uuid) -> Option<&Component> {
            self.map.get(id)
        }
    }

    fn rate() -> ExchangeRate {
        ExchangeRate::new(dec!(18.52))
    }

    #[test]
    fn empty_assembly_costs_zero() {
        let asm = Assembly::new("vacío", crate::domain::assembly::QuoteType::Personalizado);
        let cat = FakeCatalog { map: HashMap::new() };
        let cost = compute_cost(&asm, &cat, &rate());
        assert_eq!(cost.subtotal.amount, dec!(0));
        assert_eq!(cost.iva.amount, dec!(0));
        assert_eq!(cost.total.amount, dec!(0));
        assert_eq!(cost.total.currency, crate::domain::money::Currency::Mxp);
    }

    #[test]
    fn usd_prices_convert_with_ceiled_rate_before_summing() {
        // 54 USD × 19 = 1026 MXN (T.C 18.52 → efectivo 19)
        let mut asm = Assembly::new("test", crate::domain::assembly::QuoteType::Personalizado);
        let cat_map = HashMap::new();
        let cat = FakeCatalog { map: cat_map };
        let mut cat = cat;
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, dec!(54), crate::domain::money::Currency::Usd);
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.map.insert(psu.id, psu);

        let cost = compute_cost(&asm, &cat, &rate());
        assert_eq!(cost.subtotal.amount, dec!(1026.00));
        assert_eq!(cost.iva.amount, dec!(164.16));
        assert_eq!(cost.total.amount, dec!(1190.16));
    }

    #[test]
    fn mxp_prices_are_added_untouched() {
        let mut asm = Assembly::new("test", crate::domain::assembly::QuoteType::Personalizado);
        let mut cat = FakeCatalog { map: HashMap::new() };
        let case = comp(Uuid::new_v4(), ComponentKind::Case, dec!(41.60), crate::domain::money::Currency::Mxp);
        asm.set_quantity(Slot::Case, &case, 1, None).unwrap();
        cat.map.insert(case.id, case);

        let cost = compute_cost(&asm, &cat, &rate());
        assert_eq!(cost.subtotal.amount, dec!(41.60));
        // IVA de 41.60 = 6.656 → redondeado comercial 6.66
        assert_eq!(cost.iva.amount, dec!(6.66));
        assert_eq!(cost.total.amount, dec!(48.26));
    }

    #[test]
    fn mixed_currencies_and_quantities() {
        let mut asm = Assembly::new("test", crate::domain::assembly::QuoteType::Personalizado);
        let mut cat = FakeCatalog { map: HashMap::new() };

        // RAM USD 34.55 ×19 = 656.45, cantidad 2 → 1312.90
        let ram = comp(Uuid::new_v4(), ComponentKind::Ram, dec!(34.55), crate::domain::money::Currency::Usd);
        asm.set_quantity(Slot::Ram, &ram, 2, None).unwrap();
        cat.map.insert(ram.id, ram);

        // CPU MXP 5000.00, cantidad 1
        let cpu = comp(Uuid::new_v4(), ComponentKind::Cpu, dec!(5000), crate::domain::money::Currency::Mxp);
        asm.set_quantity(Slot::Cpu, &cpu, 1, None).unwrap();
        cat.map.insert(cpu.id, cpu);

        let cost = compute_cost(&asm, &cat, &rate());
        assert_eq!(cost.subtotal.amount, dec!(6312.90));
        // IVA: 6312.90 × 0.16 = 1010.064 → 1010.06
        assert_eq!(cost.iva.amount, dec!(1010.06));
        assert_eq!(cost.total.amount, dec!(7322.96));
        // Invariante de pantalla: subtotal + IVA = total
        assert_eq!(cost.subtotal.amount + cost.iva.amount, cost.total.amount);
    }

    #[test]
    fn missing_components_are_ignored_not_counted() {
        let mut asm = Assembly::new("test", crate::domain::assembly::QuoteType::Personalizado);
        let cat = FakeCatalog { map: HashMap::new() };
        let ghost = comp(Uuid::new_v4(), ComponentKind::Gpu, dec!(1), crate::domain::money::Currency::Usd);
        asm.set_quantity(Slot::Gpu, &ghost, 1, None).unwrap();
        let cost = compute_cost(&asm, &cat, &rate());
        assert_eq!(cost.total.amount, dec!(0));
    }
}
