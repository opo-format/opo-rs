//! The object catalogue, generated from the `object.yaml` of every object in a library
//! (`objects/<id>/object.yaml`), including `planned` catalogue entries (decision 0018).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value as Json;

use crate::{Error, read_yaml};

/// One catalogue row.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Folder name.
    pub id: String,
    /// Catalogue number, e.g. `O-02`.
    pub catalogue_id: Option<String>,
    /// Display name.
    pub name: String,
    /// IFC class, with the predefined type when given (`IfcDoor · DOOR`).
    pub ifc: String,
    /// `P1`, `P2` or `P3`.
    pub priority: Option<String>,
    /// Lifecycle status (default `draft`).
    pub status: String,
    /// The sheet's scope, or the description.
    pub scope: String,
}

/// The catalogue of a library and the problems found in its object sources: an `id` that
/// differs from the folder, a planned entry without priority, IFC class or scope, an unknown
/// status or priority, a malformed or repeated catalogue number.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    /// Rows, ordered by catalogue number, then id.
    pub entries: Vec<Entry>,
    /// Problems (`<id>: …`).
    pub problems: Vec<String>,
}

const STATUSES: [&str; 5] = ["planned", "draft", "review", "stable", "deprecated"];
const PRIORITIES: [&str; 3] = ["P1", "P2", "P3"];

fn text(v: &Json, key: &str) -> Option<String> {
    v.get(key).and_then(Json::as_str).map(str::to_owned)
}

fn catalogue_id_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 4
        && b[0].is_ascii_uppercase()
        && b[1] == b'-'
        && b[2..].iter().all(u8::is_ascii_digit)
}

/// Read `objects/*/object.yaml` under `root`.
pub fn catalogue(root: &Path) -> Result<Catalogue, Error> {
    let dir = root.join("objects");
    let mut folders: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(Error::Io(dir, e)),
    };
    folders.sort();
    let mut cat = Catalogue::default();
    let mut numbers: BTreeMap<String, String> = BTreeMap::new();
    for f in folders {
        let folder = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some(v) = read_yaml(&f.join("object.yaml"))? else {
            cat.problems.push(format!("{folder}: no object.yaml"));
            continue;
        };
        let mut problem = |m: String| cat.problems.push(format!("{folder}: {m}"));
        let id = text(&v, "id").unwrap_or_default();
        if id != folder {
            problem(format!("id '{id}' is not the folder name"));
        }
        let status = text(&v, "status").unwrap_or_else(|| "draft".into());
        if !STATUSES.contains(&status.as_str()) {
            problem(format!("unknown status '{status}'"));
        }
        let priority = text(&v, "priority");
        if let Some(p) = &priority {
            if !PRIORITIES.contains(&p.as_str()) {
                problem(format!("unknown priority '{p}'"));
            }
        }
        let catalogue_id = text(&v, "catalogue_id");
        if let Some(c) = &catalogue_id {
            if !catalogue_id_ok(c) {
                problem(format!("catalogue_id '{c}' is not like O-02"));
            } else if let Some(other) = numbers.insert(c.clone(), folder.clone()) {
                problem(format!("catalogue_id {c} is also used by {other}"));
            }
        }
        let class = v.pointer("/ifc/class").and_then(Json::as_str);
        let scope = v.pointer("/sheet/scope").and_then(Json::as_str);
        if status == "planned" {
            for (what, missing) in [
                ("priority", priority.is_none()),
                ("ifc.class", class.is_none()),
                ("sheet.scope", scope.is_none()),
            ] {
                if missing {
                    problem(format!("a planned object needs {what}"));
                }
            }
        }
        if status == "deprecated" && text(&v, "replaced_by").is_none() {
            problem("a deprecated object needs replaced_by".into());
        }
        let ifc = match (
            class,
            v.pointer("/ifc/predefined_type").and_then(Json::as_str),
        ) {
            (Some(c), Some(t)) => format!("{c} · {t}"),
            (Some(c), None) => c.to_owned(),
            _ => String::new(),
        };
        cat.entries.push(Entry {
            id: folder.clone(),
            catalogue_id,
            name: text(&v, "name").unwrap_or_default(),
            ifc,
            priority,
            status,
            scope: scope
                .map(str::to_owned)
                .or_else(|| text(&v, "description"))
                .unwrap_or_default(),
        });
    }
    cat.entries.sort_by(|a, b| {
        let key = |e: &Entry| {
            (
                e.catalogue_id.is_none(),
                e.catalogue_id.clone(),
                e.id.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(cat)
}

impl Catalogue {
    /// The catalogue as a Markdown table (generated; not to be committed, decision 0016).
    pub fn to_markdown(&self) -> String {
        let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
        let mut out = String::from(
            "# Object catalogue\n\nGenerated by `opo catalogue` from `objects/*/object.yaml`; do not edit.\n\n\
             | No. | Object | Id | IFC class | Pri. | Status | Scope |\n|---|---|---|---|---|---|---|\n",
        );
        for e in &self.entries {
            out += &format!(
                "| {} | {} | `{}` | {} | {} | {} | {} |\n",
                e.catalogue_id.as_deref().unwrap_or(""),
                cell(&e.name),
                e.id,
                cell(&e.ifc),
                e.priority.as_deref().unwrap_or(""),
                e.status,
                cell(&e.scope)
            );
        }
        let count = |s: &str| self.entries.iter().filter(|e| e.status == s).count();
        out += &format!(
            "\n{} objects: {} planned, {} draft, {} review, {} stable, {} deprecated.\n",
            self.entries.len(),
            count("planned"),
            count("draft"),
            count("review"),
            count("stable"),
            count("deprecated")
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, id: &str, yaml: &str) {
        let d = root.join("objects").join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("object.yaml"), yaml).unwrap();
    }

    #[test]
    fn rows_and_problems() {
        let root = std::env::temp_dir().join(format!("opo-catalogue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(
            &root,
            "door-double-swing",
            "id: \"door-double-swing\"\nname: \"Double swing door\"\nstatus: \"planned\"\npriority: \"P2\"\ncatalogue_id: \"O-02\"\nifc:\n  class: \"IfcDoor\"\n  predefined_type: \"DOOR\"\nsheet:\n  scope: \"Two leaves | one frame.\"\n",
        );
        write(
            &root,
            "door-single-swing",
            "id: \"door-single-swing\"\nname: \"Single swing door\"\ncatalogue_id: \"O-01\"\ndescription: \"A door.\"\nifc:\n  class: \"IfcDoor\"\n",
        );
        write(
            &root,
            "bad",
            "id: \"other\"\nname: \"Bad\"\nstatus: \"planned\"\ncatalogue_id: \"O-01\"\n",
        );
        let cat = catalogue(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let ids: Vec<_> = cat.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["bad", "door-single-swing", "door-double-swing"]);
        assert_eq!(cat.entries[1].status, "draft");
        assert_eq!(cat.entries[2].ifc, "IfcDoor · DOOR");
        let md = cat.to_markdown();
        assert!(md.contains("| O-02 | Double swing door | `door-double-swing` | IfcDoor · DOOR | P2 | planned | Two leaves \\| one frame. |"));
        assert!(md.contains("3 objects: 2 planned, 1 draft"));
        for want in [
            "bad: id 'other' is not the folder name",
            "door-single-swing: catalogue_id O-01 is also used by bad",
            "bad: a planned object needs priority",
            "bad: a planned object needs ifc.class",
            "bad: a planned object needs sheet.scope",
        ] {
            assert!(
                cat.problems.iter().any(|p| p.starts_with(want)),
                "{want}: {:?}",
                cat.problems
            );
        }
        assert_eq!(cat.problems.len(), 5, "{:?}", cat.problems);
    }
}
