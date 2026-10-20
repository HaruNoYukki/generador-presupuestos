//! Formateo asistido de títulos con Z.ai (opcional, requiere clave API).
//!
//! Los proveedores formatean las descripciones de manera distinta; con una
//! clave de Z.ai guardada en Ajustes se pueden normalizar en lotes para
//! que el catálogo tenga un único estilo de título.

use serde_json::json;

/// Normaliza un lote de títulos: devuelve pares (sku, título nuevo).
/// Si una entrada falla, se omite sin abortar el lote.
pub fn normalize_titles_batch(
    api_key: &str,
    entries: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let lista: String = entries
        .iter()
        .map(|(sku, titulo)| format!("- {sku}: {titulo}"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = json!({
        "model": "glm-4.6",
        "messages": [
            {
                "role": "system",
                "content": "Eres un normalizador de catálogos de componentes de computadora. Recibes una lista de títulos crudos de proveedores y devuelves un título único, limpio y descriptivo por cada uno, en MAYÚSCULAS estables, sin el nombre del proveedor, sin duplicar el SKU al final, preservando marcas, capacidades y conectores. Responde SOLO un JSON: {\"titulos\":[{\"sku\":\"...\",\"titulo\":\"...\"}]}"
            },
            { "role": "user", "content": lista }
        ],
        "temperature": 0.2
    });

    let resp = ureq::post("https://api.z.ai/api/paas/v4/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .send_json(body)
        .map_err(|e| format!("Error llamando a Z.ai: {e}"))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Respuesta inválida de Z.ai: {e}"))?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => {
            // tolerar bloques de código ```json ... ```
            let limpio = content
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            serde_json::from_str(limpio).map_err(|e| format!("JSON no interpretable: {e}"))?
        }
    };
    let mut out = Vec::new();
    if let Some(arr) = parsed["titulos"].as_array() {
        for item in arr {
            if let (Some(sku), Some(titulo)) =
                (item["sku"].as_str(), item["titulo"].as_str())
            {
                out.push((sku.to_string(), titulo.trim().to_string()));
            }
        }
    }
    Ok(out)
}
