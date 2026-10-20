//! Notas de venta (fase 5): ligadas a una cotización, con número propio
//! (DDMMAAAA-NC, p. ej. 30092024-1) y números de serie por unidad.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaleNoteLine {
    pub component_id: Uuid,
    pub sku: String,
    pub description: String,
    /// Tipo de componente (Procesador, Motherboard, RAM, …).
    pub kind: String,
    /// Número de serie capturado de la unidad.
    pub serial: String,
    /// Precio de la unidad (con IVA), igual al de la cotización.
    pub price: Decimal,
    /// Posición del renglón (persistencia normalizada).
    #[serde(default)]
    pub line_no: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaleNoteRecord {
    pub number: String,
    /// Número de cotización ligada (DDMMAA-NC). En ventas individuales no
    /// hay cotización: es NULL.
    pub quote_number: Option<String>,
    /// Ensamble de origen; NULL en ventas individuales.
    pub assembly_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub lines: Vec<SaleNoteLine>,
}
