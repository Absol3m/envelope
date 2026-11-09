//! Définitions d'outils MCP et validation des arguments d'un appel contre leur schéma.

use serde_json::{json, Value};

/// Définition d'un outil **en lecture seule** (temps 1). Les annotations servent aux clients qui
/// demandent confirmation ou approuvent automatiquement : elles ne doivent jamais en dire moins
/// que la vérité. Les outils d'écriture (temps 2) devront dire `readOnlyHint: false` et, pour
/// envoi/suppression, `destructiveHint` + `openWorldHint` : ajouter alors un paramètre d'effet.
pub fn tool(name: &str, title: &str, desc: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {
            "title": title,
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        },
    })
}

/// Schéma d'objet fermé : aucune clé hors de `props`.
pub fn object(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

pub fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

pub fn integer(desc: &str, min: i64, max: i64) -> Value {
    json!({"type": "integer", "description": desc, "minimum": min, "maximum": max})
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true,
    }
}

/// Vérifie `args` contre le schéma de l'outil `name` tel que listé : outil inconnu, argument
/// requis manquant ou inconnu, type, bornes. Un appel invalide n'atteint jamais l'outil.
pub fn check_arguments(defs: &[Value], name: &str, args: &Value) -> Result<(), String> {
    let def = defs
        .iter()
        .find(|d| d["name"] == name)
        .ok_or_else(|| format!("unknown tool `{name}`"))?;
    let schema = &def["inputSchema"];
    let Some(a) = args.as_object() else {
        return Err(format!("{name}: arguments must be an object"));
    };
    for r in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if a.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name}: missing required argument `{r}`"));
        }
    }
    let props = schema["properties"].as_object();
    for (k, v) in a {
        let Some(p) = props.and_then(|p| p.get(k)) else {
            let valid: Vec<&str> = props
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            let k: String = k.chars().take(64).collect();
            return Err(format!(
                "{name}: unknown argument `{k}`; its arguments are: {}",
                valid.join(", ")
            ));
        };
        if v.is_null() {
            continue;
        }
        if p["type"].as_str().is_some_and(|t| !type_matches(t, v)) {
            return Err(format!(
                "{name}: argument `{k}` must be of type {}",
                p["type"]
            ));
        }
        if let Some(n) = v.as_i64() {
            let below = p["minimum"].as_i64().is_some_and(|m| n < m);
            let above = p["maximum"].as_i64().is_some_and(|m| n > m);
            if below || above {
                return Err(format!(
                    "{name}: `{k}` must be between {} and {}",
                    p["minimum"], p["maximum"]
                ));
            }
        }
    }
    Ok(())
}

/// Résultat d'outil d'un seul bloc texte ; `is_error` pour que le modèle lise l'erreur et se corrige.
pub fn text_result(text: impl Into<String>, is_error: bool) -> Value {
    let mut r = json!({"content": [{"type": "text", "text": text.into()}]});
    if is_error {
        r["isError"] = json!(true);
    }
    r
}
