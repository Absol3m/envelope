//! Adresses e-mail telles que stockées en base : colonne JSON `[{"name":..,"address":..}]`.
//! Un seul type pour l'écriture (application) et la lecture (MCP) : changer le format casse la compilation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Addr {
    pub name: Option<String>,
    pub address: String,
}

/// `Nom <adresse>`, ou la seule adresse sans nom.
pub fn mailbox(name: Option<&str>, address: &str) -> String {
    match name {
        Some(n) if !n.is_empty() => format!("{n} <{address}>"),
        _ => address.to_string(),
    }
}

impl Addr {
    pub fn display(&self) -> String {
        mailbox(self.name.as_deref(), &self.address)
    }
}

pub fn encode(list: &[Addr]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".into())
}

/// Liste vide si la colonne est illisible (donnée écrite par une version inconnue).
pub fn decode(json: &str) -> Vec<Addr> {
    serde_json::from_str(json).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_display() {
        let l = vec![
            Addr {
                name: Some("Alice".into()),
                address: "a@x.fr".into(),
            },
            Addr {
                name: None,
                address: "b@x.fr".into(),
            },
        ];
        assert_eq!(decode(&encode(&l)), l);
        assert_eq!(l[0].display(), "Alice <a@x.fr>");
        assert_eq!(l[1].display(), "b@x.fr");
        assert!(decode("pas du json").is_empty());
    }
}
