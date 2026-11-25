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
    /// `rulesets/<id>/<variant>.yaml`: shared rule-set variants by rule-set id, in file-name
    /// order (proposal 0002, Resolved). Read-only: `save` and `merge` leave them alone.
    pub rulesets: std::collections::BTreeMap<String, Vec<(String, Json)>>,
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

/// Shared rule sets: `<dir>/<id>/<variant>.yaml`, each a layer source of kind `ruleset` whose
/// `id` is its folder name and which has no `node`.
fn read_rulesets(
    dir: &Path,
) -> Result<std::collections::BTreeMap<String, Vec<(String, Json)>>, Error> {
    let mut out = std::collections::BTreeMap::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let list = |d: &Path| -> Result<Vec<PathBuf>, Error> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d)
            .map_err(|e| Error::Io(d.to_owned(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        v.sort();
        Ok(v)
    };
    for idir in list(dir)?.into_iter().filter(|p| p.is_dir()) {
        let id = idir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut variants = Vec::new();
        for f in list(&idir)?
            .into_iter()
            .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        {
            let v = read_yaml(&f)?.unwrap_or(Json::Null);
            let bad = |m: &str| Error::Source(format!("{}: {m}", f.display()));
            if v.get("kind").and_then(Json::as_str) != Some("ruleset") {
                return Err(bad("a shared rule set has kind ruleset"));
            }
            if v.get("id").and_then(Json::as_str) != Some(id.as_str()) {
                return Err(bad("its id must be the folder name"));
            }
            if v.get("node").is_some() {
                return Err(bad(
                    "a shared rule set has no node (ids are made per object)",
                ));
            }
            let variant = f
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            variants.push((variant, v));
        }
        out.insert(id, variants);
    }
    Ok(out)
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
            rulesets: read_rulesets(&root.join("rulesets"))?,
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

#[cfg(test)]
mod ruleset_tests {
    use super::*;

    #[test]
    fn shared_rulesets_load_and_are_checked() {
        let root = std::env::temp_dir().join(format!("opo-lib-rulesets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join("rulesets/stairs");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("uk.yaml"),
            "kind: \"ruleset\"\nid: \"stairs\"\nsource: \"s\"\nvalues:\n  riser_max: \"0.22\"\n",
        )
        .unwrap();
        let lib = Library::load(&root).unwrap();
        assert_eq!(lib.rulesets["stairs"][0].0, "uk");
        std::fs::write(
            d.join("us.yaml"),
            "kind: \"ruleset\"\nid: \"guards\"\nsource: \"s\"\nvalues: {}\n",
        )
        .unwrap();
        assert!(Library::load(&root).is_err());
        std::fs::write(
            d.join("us.yaml"),
            "kind: \"ruleset\"\nid: \"stairs\"\nnode: \"x\"\nsource: \"s\"\nvalues: {}\n",
        )
        .unwrap();
        assert!(Library::load(&root).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
