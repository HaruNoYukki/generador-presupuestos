//! Generador de Presupuestos — monolito modular en Rust + Tauri v2.
//!
//! Módulos:
//! - `db`: SQLite normalizado (3FN/BCNF) — maestro de productos + ofertas
//!   por proveedor + existencias por almacén.
//! - `domain`: componentes, dinero (regla del T.C), ensamble con validación
//!   de compatibilidad, costo, ganancias, carrito y notas de venta.
//! - `suppliers`: importación de listas de precios en Excel por proveedor.
//! - `catalog`: catálogo en memoria (mejor oferta por SKU).
//! - `store`: estado de la aplicación y persistencia.
//! - `commands`: frontera de comandos hacia la GUI.
//!
//! La semilla del catálogo (`data/catalog_seed.json`) es OPCIONAL y NO se
//! incluye en el repositorio (contiene precios de proveedores). Se busca en
//! este orden: directorio de datos de la app → recursos del paquete →
//! directorio de trabajo. Sin semilla, la app arranca con catálogo vacío y
//! se llena importando un Excel desde Ajustes.

mod ai;
mod catalog;
mod commands;
mod db;
mod domain;
mod store;
mod suppliers;

use tauri::Manager;
use std::path::PathBuf;

/// Rutas donde puede estar la semilla opcional del catálogo.
fn resolve_seed_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path().app_data_dir().ok()?.join("data/catalog_seed.json"),
        app.path().resource_dir().ok()?.join("data/catalog_seed.json"),
        PathBuf::from("data/catalog_seed.json"),
        PathBuf::from("../data/catalog_seed.json"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app
                .path()
                .app_data_dir()
                .expect("no se pudo resolver el directorio de datos de la app");
            let seed = resolve_seed_path(&app.handle())
                .and_then(|p| std::fs::read_to_string(p).ok());
            let state = store::AppState::load(seed.as_deref(), dir);
            app.manage(store::SharedState(std::sync::Mutex::new(state)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::catalog_overview,
            commands::list_components,
            commands::get_component,
            commands::add_manual_component,
            commands::remove_manual_component,
            commands::get_rate,
            commands::set_rate,
            commands::create_assembly,
            commands::duplicate_assembly,
            commands::set_fabrication,
            commands::assign_quote_number,
            commands::list_assemblies,
            commands::get_assembly,
            commands::rename_assembly,
            commands::delete_assembly,
            commands::set_assembly_item,
            commands::clear_assembly_slot,
            commands::validate_assembly,
            commands::assembly_cost,
            commands::confirm_assembly,
            commands::assembly_profit,
            commands::cart_profit,
            commands::import_catalog,
            commands::import_catalog_xlsx,
            commands::catalog_filters,
            commands::list_spec_options,
            commands::add_spec_option,
            commands::add_brand,
            commands::get_zai_key,
            commands::set_zai_key,
            commands::ai_normalize_titles,
            commands::get_component_by_sku,
            commands::next_quote_number,
            commands::next_sale_note_number,
            commands::record_sale_note,
            commands::list_sale_notes,
            commands::save_export,
            commands::convert_money,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
