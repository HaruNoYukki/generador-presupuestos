//! Cálculo de ganancias y precio de venta de un ensamble.
//!
//! Reglas (fase 3):
//! - Ganancia de cada componente = costo sin IVA × porcentaje.
//!   Presets básicos: 10%, 20% y 30%; se admite cualquier porcentaje.
//! - El IVA se aplica UNA sola vez por renglón sobre su base gravable:
//!   (costo + ganancia final) × 16%. Con precio objetivo el IVA SE
//!   RECALCULA sobre la base ajustada; así Subtotal + IVA = Total siempre
//!   coincide.
//! - Precio del componente = costo + ganancia final + IVA.
//! - Precio objetivo: D = objetivo − total actual; D, positivo o negativo,
//!   se reparte en partes iguales entre los componentes ajustando la
//!   ganancia. El residuo del redondeo cae en el último renglón.

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::assembly::{Assembly, ComponentLookup};
use super::money::ExchangeRate;

/// IVA vigente en México: 16% (compartido con el módulo de costo).
pub const IVA_RATE: Decimal = rust_decimal_macros::dec!(0.16);

/// Presets básicos de ganancia.
pub const PRESETS: [Decimal; 3] = [
    rust_decimal_macros::dec!(10),
    rust_decimal_macros::dec!(20),
    rust_decimal_macros::dec!(30),
];

fn round2(d: Decimal) -> Decimal {
    d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfitLine {
    pub component_id: Uuid,
    pub description: String,
    pub sku: String,
    pub quantity: u32,
    /// Precio unitario en MXN sin IVA.
    pub unit_cost: Decimal,
    /// Costo del renglón: unitario × cantidad, sin IVA.
    pub cost: Decimal,
    /// Ganancia base = costo × porcentaje.
    pub base_profit: Decimal,
    /// Ajuste del precio objetivo (positivo suma, negativo resta).
    pub adjustment: Decimal,
    /// Ganancia final = base + ajuste.
    pub final_profit: Decimal,
    /// IVA aplicado UNA vez sobre la base gravable (costo + ganancia
    /// final) × 16%; se recalcula cuando el objetivo ajusta la ganancia.
    pub iva: Decimal,
    /// Precio = costo + ganancia final + IVA.
    pub price: Decimal,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfitTotals {
    pub cost: Decimal,
    pub base_profit: Decimal,
    pub adjustment: Decimal,
    pub final_profit: Decimal,
    pub iva: Decimal,
    pub price: Decimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfitReport {
    /// Porcentaje aplicado (10/20/30 o capturado).
    pub percent: Decimal,
    /// Precio objetivo capturado, si lo hay.
    pub target: Option<Decimal>,
    /// objetivo − total antes del ajuste.
    pub difference: Option<Decimal>,
    /// Ajuste por componente (partes iguales; el último absorbe el residuo).
    pub adjustment_per_line: Option<Decimal>,
    pub lines: Vec<ProfitLine>,
    pub totals: ProfitTotals,
}

/// Construye el renglón de ganancia de una partida (componente + cantidad).
/// Lo reutilizan los dos flujos: ensambles y ventas individuales.
pub fn build_profit_line(
    c: &crate::domain::component::Component,
    quantity: u32,
    rate: &ExchangeRate,
    percent: Decimal,
) -> ProfitLine {
    let factor = percent / Decimal::from(100);
    let unit_cost = c.price_mxp(rate).amount;
    let cost = round2(unit_cost * Decimal::from(quantity));
    let base_profit = round2(cost * factor);
    let iva = round2((cost + base_profit) * IVA_RATE);
    ProfitLine {
        component_id: c.id,
        description: c.description.clone(),
        sku: c.sku.clone(),
        quantity,
        unit_cost,
        cost,
        base_profit,
        adjustment: Decimal::ZERO,
        final_profit: base_profit,
        iva,
        price: cost + base_profit + iva,
    }
}

/// Calcula la ganancia por componente con el porcentaje dado. Sin objetivo:
/// ajuste 0 y ganancia final = base.
pub fn compute_profit<L: ComponentLookup>(
    assembly: &Assembly,
    catalog: &L,
    rate: &ExchangeRate,
    percent: Decimal,
) -> ProfitReport {
    let mut lines = Vec::new();
    for items in assembly.slots.values() {
        for item in items {
            if let Some(c) = catalog.get_component(&item.component_id) {
                lines.push(build_profit_line(c, item.quantity, rate, percent));
            }
        }
    }
    let mut report = ProfitReport {
        percent,
        target: None,
        difference: None,
        adjustment_per_line: None,
        lines,
        totals: ProfitTotals::default(),
    };
    recompute_totals(&mut report);
    report
}

/// Renglones de ganancia a partir de pares (componente, cantidad): el
/// carrito de la venta individual usa esto.
pub fn profit_lines_from_pairs(
    pairs: &[(crate::domain::component::Component, u32)],
    rate: &ExchangeRate,
    percent: Decimal,
) -> Vec<ProfitLine> {
    pairs
        .iter()
        .map(|(c, qty)| build_profit_line(c, *qty, rate, percent))
        .collect()
}

/// Arma un ProfitReport a partir de renglones ya calculados.
pub fn report_from_lines(percent: Decimal, lines: Vec<ProfitLine>) -> ProfitReport {
    let mut report = ProfitReport {
        percent,
        target: None,
        difference: None,
        adjustment_per_line: None,
        lines,
        totals: ProfitTotals::default(),
    };
    recompute_totals(&mut report);
    report
}

/// Aplica un precio objetivo (total CON IVA): la diferencia contra el total
/// actual se reparte equitativamente entre los renglones ajustando la
/// ganancia y el IVA de cada renglón SE RECALCULA sobre su nueva base
/// gravable (costo + ganancia final). Así la suma Subtotal + IVA = Total
/// coincide y el total queda a un centavo del objetivo como máximo (por el
/// redondeo de IVA por renglón, absorbido también en el último renglón).
pub fn apply_target(report: &mut ProfitReport, target: Decimal) {
    let n = report.lines.len();
    if n == 0 {
        report.target = Some(target);
        return;
    }
    let current: Decimal = report.lines.iter().map(|l| l.price).sum();
    let difference = round2(target - current);

    // La diferencia se reparte sobre el subtotal (sin IVA): para mover el
    // total con IVA en `difference`, el subtotal debe moverse en
    // difference / (1 + IVA).
    let subtotal_now: Decimal = report.lines.iter().map(|l| l.cost + l.final_profit).sum();
    let delta_subtotal = round2(difference / (Decimal::ONE + IVA_RATE));
    let per = round2(delta_subtotal / Decimal::from(n));

    for (i, line) in report.lines.iter_mut().enumerate() {
        // El último renglón absorbe el residuo del redondeo para que la
        // suma de ajustes sea exactamente la diferencia sin IVA.
        line.adjustment = if i == n - 1 {
            round2(delta_subtotal - per * Decimal::from(n - 1))
        } else {
            per
        };
        line.final_profit = round2(line.base_profit + line.adjustment);
        // IVA recalculado sobre la nueva base gravable.
        line.iva = round2((line.cost + line.final_profit) * IVA_RATE);
        line.price = line.cost + line.final_profit + line.iva;
    }
    let _ = subtotal_now;

    // El redondeo de IVA por renglón puede dejar el total a ±centavos del
    // objetivo: ese residuo se absorbe en el IVA del último renglón para
    // que el total cierre EXACTO en el objetivo.
    let total_now: Decimal = report.lines.iter().map(|l| l.price).sum();
    let residual = round2(target - total_now);
    if residual != Decimal::ZERO {
        if let Some(last) = report.lines.last_mut() {
            last.iva = round2(last.iva + residual);
            last.price = last.cost + last.final_profit + last.iva;
        }
    }

    report.target = Some(target);
    report.difference = Some(difference);
    report.adjustment_per_line = Some(per);
    recompute_totals(report);
}

fn recompute_totals(report: &mut ProfitReport) {
    let mut t = ProfitTotals::default();
    for l in &report.lines {
        t.cost += l.cost;
        t.base_profit += l.base_profit;
        t.adjustment += l.adjustment;
        t.final_profit += l.final_profit;
        t.iva += l.iva;
        t.price += l.price;
    }
    report.totals = t;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assembly::Slot;
    use crate::domain::component::{
        Component, ComponentKind, ComponentSource, Condition, Specs, Stock,
    };
    use crate::domain::money::{Currency, Money};
    use rust_decimal_macros::dec;
    use std::collections::{BTreeMap, HashMap};

    fn comp(id: Uuid, kind: ComponentKind, amount: Decimal, currency: Currency) -> Component {
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

    struct FakeCatalog(HashMap<Uuid, Component>);
    impl ComponentLookup for FakeCatalog {
        fn get_component(&self, id: &Uuid) -> Option<&Component> {
            self.0.get(id)
        }
    }

    fn two_line_assembly() -> (Assembly, FakeCatalog) {
        let mut asm = Assembly::new("cotización", crate::domain::assembly::QuoteType::Personalizado);
        let mut cat = FakeCatalog(HashMap::new());
        // PSU 54 USD × 19 = 1026.00 MXN
        let psu = comp(Uuid::new_v4(), ComponentKind::Psu, dec!(54), Currency::Usd);
        asm.set_quantity(Slot::Psu, &psu, 1, None).unwrap();
        cat.0.insert(psu.id, psu);
        // RAM 34.55 USD × 19 = 656.45 × 2 = 1312.90 MXN
        let ram = comp(
            Uuid::new_v4(),
            ComponentKind::Ram,
            dec!(34.55),
            Currency::Usd,
        );
        asm.set_quantity(Slot::Ram, &ram, 2, None).unwrap();
        cat.0.insert(ram.id, ram);
        (asm, cat)
    }

    fn rate() -> ExchangeRate {
        ExchangeRate::new(dec!(18.52))
    }

    #[test]
    fn preset_profit_and_iva_per_line() {
        // 10% sobre la PSU: costo 1026, ganancia 102.60, IVA (1026+102.60)×16% = 180.58
        let (asm, cat) = two_line_assembly();
        let r = compute_profit(&asm, &cat, &rate(), dec!(10));
        assert_eq!(r.lines.len(), 2);
        let psu = r.lines.iter().find(|l| l.cost == dec!(1026.00)).unwrap();
        assert_eq!(psu.base_profit, dec!(102.60));
        assert_eq!(psu.iva, dec!(180.58));
        assert_eq!(psu.price, dec!(1309.18));

        // RAM ×2: costo 1312.90, ganancia 131.29, IVA (1444.19×16%) = 231.07
        let ram = r.lines.iter().find(|l| l.cost == dec!(1312.90)).unwrap();
        assert_eq!(ram.base_profit, dec!(131.29));
        assert_eq!(ram.iva, dec!(231.07));

        // Sumas de columnas
        assert_eq!(r.totals.cost, dec!(2338.90));
        assert_eq!(r.totals.base_profit, dec!(233.89));
        assert_eq!(r.totals.iva, dec!(411.65));
        assert_eq!(r.totals.price, dec!(2984.44));
    }

    #[test]
    fn presets_20_and_30_scale_profit() {
        let (asm, cat) = two_line_assembly();
        let r20 = compute_profit(&asm, &cat, &rate(), dec!(20));
        assert_eq!(r20.totals.base_profit, dec!(467.78));
        let r30 = compute_profit(&asm, &cat, &rate(), dec!(30));
        assert_eq!(r30.totals.base_profit, dec!(701.67));
    }

    #[test]
    fn iva_is_applied_once_not_on_totals_compound() {
        // totals.iva es la suma de los IVAs por renglón, NO el 16% del total
        // (que ya incluye IVA). total = costo + ganancia + IVA una sola vez.
        let (asm, cat) = two_line_assembly();
        let r = compute_profit(&asm, &cat, &rate(), dec!(10));
        assert_eq!(r.totals.price, r.totals.cost + r.totals.base_profit + r.totals.iva);
        let compound = round2(r.totals.price * IVA_RATE);
        assert_ne!(r.totals.iva, compound);
    }

    #[test]
    fn target_up_distributes_equally_recalcs_iva_and_closes_exact() {
        let (asm, cat) = two_line_assembly();
        let mut r = compute_profit(&asm, &cat, &rate(), dec!(10));
        let iva_base = r.totals.iva; // 411.65
        apply_target(&mut r, dec!(4000));
        // Diferencia 4000 − 2984.44 = 1015.56; reparto SIN IVA:
        // 1015.56/1.16 = 875.48 → 437.74 por renglón
        assert_eq!(r.difference, Some(dec!(1015.56)));
        assert_eq!(r.adjustment_per_line, Some(dec!(437.74)));
        assert_eq!(r.lines[0].adjustment, dec!(437.74));
        assert_eq!(r.lines[1].adjustment, dec!(437.74));
        // IVA RECALCULADO (lines[0]=RAM, lines[1]=PSU por orden de slots):
        // RAM base 1312.90+569.03=1881.93×16%=301.11;
        // PSU base 1026+540.34=1566.34×16%=250.61
        assert_eq!(r.lines[0].final_profit, dec!(569.03));
        assert_eq!(r.lines[0].iva, dec!(301.11));
        assert_eq!(r.lines[1].final_profit, dec!(540.34));
        // El IVA de la última línea absorbe 1 centavo de redondeo para
        // cerrar exacto en 4000 (250.6144 → 250.61 + 0.01 residual)
        assert_eq!(r.lines[1].iva, dec!(250.62));
        // El total cierra EXACTO en el objetivo y Subtotal+IVA=Total
        assert_eq!(r.totals.price, dec!(4000.00));
        assert_eq!(r.totals.cost + r.totals.final_profit + r.totals.iva, r.totals.price);
        // El IVA creció porque la base gravable creció
        assert!(r.totals.iva > iva_base);
    }

    #[test]
    fn target_down_can_make_negative_profit() {
        // Objetivo por debajo del costo: la ganancia absorbe el descuento
        // y puede volver negativa; el total cierra igual en el objetivo.
        let (asm, cat) = two_line_assembly();
        let mut r = compute_profit(&asm, &cat, &rate(), dec!(10));
        apply_target(&mut r, dec!(2000));
        assert_eq!(r.difference, Some(dec!(-984.44)));
        assert_eq!(r.totals.price, dec!(2000.00));
        assert_eq!(r.totals.cost + r.totals.final_profit + r.totals.iva, r.totals.price);
        assert!(r.totals.final_profit < Decimal::ZERO);
    }

    /// Invariante de la cotización NYKNIU: con cualquier objetivo,
    /// Subtotal + IVA = Total exacto (suma de columnas).
    #[test]
    fn quote_sums_always_match_with_target() {
        let (asm, cat) = two_line_assembly();
        for target in [dec!(2500), dec!(3500.55), dec!(6000), dec!(2984.44)] {
            let mut r = compute_profit(&asm, &cat, &rate(), dec!(20));
            apply_target(&mut r, target);
            let subtotal: Decimal = r.lines.iter().map(|l| l.cost + l.final_profit).sum();
            assert_eq!(r.totals.cost + r.totals.final_profit, subtotal);
            assert_eq!(subtotal + r.totals.iva, r.totals.price);
            assert_eq!(r.totals.price, target);
        }
    }

    #[test]
    fn target_equal_to_current_is_noop() {
        let (asm, cat) = two_line_assembly();
        let mut r = compute_profit(&asm, &cat, &rate(), dec!(10));
        let total = r.totals.price;
        apply_target(&mut r, total);
        assert_eq!(r.difference, Some(Decimal::ZERO));
        assert_eq!(r.totals.adjustment, Decimal::ZERO);
        assert_eq!(r.totals.price, total);
    }

    #[test]
    fn empty_assembly_is_zero() {
        let asm = Assembly::new("vacío", crate::domain::assembly::QuoteType::Personalizado);
        let cat = FakeCatalog(HashMap::new());
        let r = compute_profit(&asm, &cat, &rate(), dec!(20));
        assert!(r.lines.is_empty());
        assert_eq!(r.totals.price, Decimal::ZERO);
    }
}
