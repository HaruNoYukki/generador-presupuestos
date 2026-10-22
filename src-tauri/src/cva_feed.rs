//! Consultas al servicio XML de CVA (esqueleto).
//!
//! ESTRUCTURA LISTA, DETALLES PENDIENTES: el endpoint real, el esquema XML
//! de respuesta y cualquier parámetro extra del servicio se definirán con
//! el negocio. Este módulo ya deja listo el almacenamiento de la
//! configuración (tabla `settings`) y la frontera de comandos; `query`
//! devuelve error mientras el formato no esté definido.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db;

/// Claves de la tabla `settings`.
pub const SETTING_ENDPOINT: &str = "cva_feed_endpoint";
pub const SETTING_CLIENT_KEY: &str = "cva_feed_client_key";

/// Configuración del servicio de consultas de CVA.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedConfig {
    /// URL del servicio XML (por definir con el negocio).
    #[serde(default)]
    pub endpoint: String,
    /// Clave de cliente que asigna CVA.
    #[serde(default)]
    pub client_key: String,
}

impl FeedConfig {
    pub fn is_configured(&self) -> bool {
        !self.endpoint.trim().is_empty() && !self.client_key.trim().is_empty()
    }
}

/// Respuesta esperada de una consulta de precio/existencia. Los campos
/// podrían ajustarse cuando se conozca el esquema XML real.
#[derive(Debug, Clone, Serialize)]
pub struct FeedQuote {
    pub sku: String,
    pub description: String,
    pub price: String,
    pub currency: String,
    pub stock: i64,
    /// XML crudo de la respuesta, para diagnóstico.
    pub raw_xml: String,
}

pub fn get_config(conn: &Connection) -> Option<FeedConfig> {
    let endpoint = db::get_setting(conn, SETTING_ENDPOINT)?;
    let client_key = db::get_setting(conn, SETTING_CLIENT_KEY).unwrap_or_default();
    Some(FeedConfig {
        endpoint,
        client_key,
    })
}

pub fn set_config(conn: &Connection, config: &FeedConfig) -> Result<(), String> {
    db::set_setting(conn, SETTING_ENDPOINT, config.endpoint.trim()).map_err(|e| e.to_string())?;
    db::set_setting(conn, SETTING_CLIENT_KEY, config.client_key.trim())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Consulta de precio/existencia de un SKU contra el servicio de CVA.
/// PENDIENTE: se implementará cuando se definan los detalles del servicio
/// (endpoint, autenticación y esquema XML de respuesta). Con la
/// configuración guardada, este es el punto donde irá la llamada HTTP y el
/// parseo del XML.
pub fn query(conn: &Connection, sku: &str) -> Result<FeedQuote, String> {
    let _ = sku.trim();
    let config = get_config(conn).unwrap_or_default();
    if !config.is_configured() {
        return Err("Configura el endpoint y la clave de cliente de CVA en Ajustes.".into());
    }
    Err("El servicio XML de CVA aún no está implementado: faltan los detalles del formato.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_conn() -> (Connection, PathBuf) {
        let dir = std::env::temp_dir().join(format!("cva-feed-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = crate::db::open(&dir).expect("abrir base temporal");
        (conn, dir)
    }

    #[test]
    fn feed_config_roundtrip() {
        let (conn, dir) = temp_conn();
        assert!(get_config(&conn).is_none(), "sin configurar");

        set_config(&conn, &FeedConfig {
            endpoint: "https://ejemplo.cva/feed.xml".into(),
            client_key: "CLAVE-123".into(),
        })
        .unwrap();
        let cfg = get_config(&conn).expect("config guardada");
        assert!(cfg.is_configured());
        assert_eq!(cfg.endpoint, "https://ejemplo.cva/feed.xml");
        assert_eq!(cfg.client_key, "CLAVE-123");

        // Reescribir reemplaza (settings es key/value).
        set_config(&conn, &FeedConfig {
            endpoint: "https://otra.cva/xml".into(),
            client_key: String::new(),
        })
        .unwrap();
        let cfg = get_config(&conn).unwrap();
        assert_eq!(cfg.endpoint, "https://otra.cva/xml");
        assert!(!cfg.is_configured(), "sin clave no está configurado");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn query_requires_config_and_is_not_implemented_yet() {
        let (conn, dir) = temp_conn();
        let err = query(&conn, "CPU-1").unwrap_err();
        assert!(err.contains("Configura"), "{err}");

        set_config(&conn, &FeedConfig {
            endpoint: "https://ejemplo.cva/feed.xml".into(),
            client_key: "CLAVE-123".into(),
        })
        .unwrap();
        let err = query(&conn, "CPU-1").unwrap_err();
        assert!(err.contains("no está implementado"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
