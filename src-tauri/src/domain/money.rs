//! Dinero y monedas.
//!
//! Regla de negocio: los precios pueden estar en dólares o pesos (MXP).
//! Para convertir USD a MXN se usa el tipo de cambio de la lista redondeado
//! hacia arriba al entero superior: 18.1 → 19, 18.6 → 19, 18.52 → 19.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Currency {
    #[serde(rename = "USD")]
    Usd,
    #[serde(rename = "MXP")]
    Mxp,
}

impl Currency {
    pub fn code(&self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Mxp => "MXP",
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Money {
    pub amount: Decimal,
    pub currency: Currency,
}

impl Money {
    pub fn new(amount: Decimal, currency: Currency) -> Self {
        Self { amount, currency }
    }

    /// Lo usará el módulo de costos (fase 2).
    #[allow(dead_code)]
    pub fn usd(amount: Decimal) -> Self {
        Self::new(amount, Currency::Usd)
    }

    pub fn mxp(amount: Decimal) -> Self {
        Self::new(amount, Currency::Mxp)
    }

    /// Lo usará el módulo de costos para validar partidas (fase 2).
    #[allow(dead_code)]
    pub fn is_zero(&self) -> bool {
        self.amount.is_zero()
    }

    /// Convierte a pesos. Los precios en MXP se devuelven tal cual;
    /// los USD se multiplican por el tipo de cambio efectivo (redondeado
    /// hacia arriba al entero superior).
    pub fn to_mxp(&self, rate: &ExchangeRate) -> Money {
        match self.currency {
            Currency::Mxp => self.clone(),
            Currency::Usd => Money::mxp(rate.convert_usd(self.amount)),
        }
    }
}

/// Tipo de cambio pesos por dólar tomado de la lista de precios.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExchangeRate {
    #[serde(rename = "pesos_per_dollar")]
    pub rate: Decimal,
}

impl ExchangeRate {
    pub fn new(rate: Decimal) -> Self {
        Self { rate }
    }

    /// Tipo de cambio efectivo: redondeado hacia arriba al entero superior.
    /// 18.1 → 19, 18.6 → 19, 18.52 → 19, 19.0 → 19.
    pub fn effective(&self) -> Decimal {
        self.rate.ceil()
    }

    /// Convierte un monto en dólares a pesos usando el tipo efectivo.
    pub fn convert_usd(&self, usd: Decimal) -> Decimal {
        usd * self.effective()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn effective_rate_rounds_up_to_integer() {
        assert_eq!(ExchangeRate::new(dec!(18.1)).effective(), dec!(19));
        assert_eq!(ExchangeRate::new(dec!(18.6)).effective(), dec!(19));
        assert_eq!(ExchangeRate::new(dec!(18.52)).effective(), dec!(19));
        assert_eq!(ExchangeRate::new(dec!(19)).effective(), dec!(19));
        assert_eq!(ExchangeRate::new(dec!(19.01)).effective(), dec!(20));
        assert_eq!(ExchangeRate::new(dec!(20.99)).effective(), dec!(21));
    }

    #[test]
    fn usd_converts_with_effective_rate() {
        let rate = ExchangeRate::new(dec!(18.52));
        // 54 USD × 19 = 1026 MXP
        assert_eq!(rate.convert_usd(dec!(54)), dec!(1026));
        assert_eq!(rate.convert_usd(dec!(40.80)), dec!(775.20));
    }

    #[test]
    fn mxp_money_stays_untouched() {
        let rate = ExchangeRate::new(dec!(18.52));
        let mxp = Money::mxp(dec!(41.60));
        assert_eq!(mxp.to_mxp(&rate), mxp);
    }

    #[test]
    fn usd_money_converts_to_mxp() {
        let rate = ExchangeRate::new(dec!(18.52));
        let usd = Money::usd(dec!(34.55));
        let mxp = usd.to_mxp(&rate);
        assert_eq!(mxp.currency, Currency::Mxp);
        assert_eq!(mxp.amount, dec!(656.45));
    }
}
