use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json};

use crate::{Error, read_yaml, yaml};

/// The registries of a library (proposal 0002 §3): references, materials and hatches, by id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    /// `references.yaml`: id → `{title, kind, edition | version, licence, url, note}`.
    pub references: Map<String, Json>,
    /// `styles/materials.yaml`: id → `opo::materials` entry.
    pub materials: Map<String, Json>,
    /// `styles/hatches.yaml`: id → `opo::hatches` entry.
    pub hatches: Map<String, Json>,
}

fn as_map(v: Option<Json>, what: &Path) -> Result<Map<String, Json>, Error> {
    match v {
        None | Some(Json::Null) => Ok(Map::new()),
        Some(Json::Object(m)) => Ok(m),
        Some(_) => Err(Error::Source(format!(
            "{}: must be a map of ids",
            what.display()
        ))),
    }
}

impl Library {
    /// The registry files under a library root.
    pub fn files(root: &Path) -> [PathBuf; 3] {
        [
            root.join("references.yaml"),
            root.join("styles/materials.yaml"),
            root.join("styles/hatches.yaml"),
        ]
    }

    /// Read the registries under a library root; missing files are empty.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let [r, m, h] = Self::files(root);
        Ok(Library {
            references: as_map(read_yaml(&r)?, &r)?,
            materials: as_map(read_yaml(&m)?, &m)?,
            hatches: as_map(read_yaml(&h)?, &h)?,
        })
    }

    /// Write the registries under a library root (sorted by id).
    pub fn save(&self, root: &Path) -> Result<(), Error> {
        let [r, m, h] = Self::files(root);
        for (path, map) in [
            (r, &self.references),
            (m, &self.materials),
            (h, &self.hatches),
        ] {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.into(), e))?;
            }
            let mut sorted: Vec<_> = map.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            let m: Map<String, Json> = sorted
                .into_iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            std::fs::write(&path, yaml::write(&Json::Object(m)))
                .map_err(|e| Error::Io(path.clone(), e))?;
        }
        Ok(())
    }

    /// Add entries from another library. The same id with a different definition is an error.
    pub fn merge(&mut self, other: &Library) -> Result<(), Error> {
        for (name, mine, theirs) in [
            ("reference", &mut self.references, &other.references),
            ("material", &mut self.materials, &other.materials),
            ("hatch", &mut self.hatches, &other.hatches),
        ] {
            for (k, v) in theirs {
                match mine.get(k) {
                    Some(existing) if existing != v => {
                        return Err(Error::Source(format!(
                            "{name} '{k}' is defined twice with different content"
                        )));
                    }
                    Some(_) => {}
                    None => {
                        mine.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_detects_conflicts() {
        let mut a = Library::default();
        a.materials
            .insert("steel".into(), json!({"label": "Steel"}));
        let mut b = Library::default();
        b.materials
            .insert("steel".into(), json!({"label": "Steel"}));
        b.hatches
            .insert("solid".into(), json!({"label": "Solid", "kind": "solid"}));
        a.merge(&b).unwrap();
        assert_eq!(a.hatches.len(), 1);
        b.materials
            .insert("steel".into(), json!({"label": "Stainless"}));
        assert!(a.merge(&b).is_err());
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("opo-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Library::default();
        a.references.insert(
            "iso-21542-2021".into(),
            json!({"title": "ISO 21542:2021", "kind": "norm"}),
        );
        a.save(&dir).unwrap();
        assert_eq!(Library::load(&dir).unwrap(), a);
        assert_eq!(
            Library::load(&dir.join("missing")).unwrap(),
            Library::default()
        );
    }
}
