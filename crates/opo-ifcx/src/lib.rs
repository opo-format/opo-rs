//! Read, write and compose IFCX documents (IFCX alpha, as pinned by OPO spec §5).
//!
//! An IFCX document has a `header`, `imports`, `schemas` and `data`: a flat list of nodes,
//! each with a `path` and optional `children`, `inherits` and `attributes` (spec §5.1).
//!
//! **Composition.** [`compose`] merges a stack of documents, weakest first. Nodes with the same
//! path combine entry by entry: for `children`, `inherits` and `attributes`, an entry from a
//! later node replaces the same entry from an earlier one, and a `null` value removes it.
//! Within one document, later nodes are stronger than earlier ones.
//!
//! **Inheritance.** [`Composition::resolved_attributes`] gives a node's attributes including
//! those it inherits: inherited attributes are weaker than the node's own, and of several
//! inherited nodes, the later one in the `inherits` map is stronger.
//!
//! **OPO objects.** [`load_object_dir`] reads an unpacked object folder: `object.ifcx`
//! first, then `layers/*.ifcx` in file-name order.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

/// The IFCX version OPO 0.1 pins.
pub const IFCX_VERSION: &str = "ifcx_alpha";

/// A map whose entries may be `null` (removal) in a layer.
pub type Links = Map<String, Json>;

/// An IFCX document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Header: id, ifcxVersion, dataVersion, author, timestamp.
    pub header: Header,
    /// Imported schema documents.
    #[serde(default)]
    pub imports: Vec<Import>,
    /// Attribute schemas declared by this document.
    #[serde(default)]
    pub schemas: Map<String, Json>,
    /// Nodes.
    pub data: Vec<Node>,
}

/// The document header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// Document id.
    pub id: String,
    /// IFCX version, e.g. `ifcx_alpha`.
    #[serde(rename = "ifcxVersion")]
    pub ifcx_version: String,
    /// Version of the data.
    #[serde(rename = "dataVersion")]
    pub data_version: String,
    /// Author.
    pub author: String,
    /// Timestamp.
    pub timestamp: String,
    /// Other header fields.
    #[serde(flatten)]
    pub extra: Map<String, Json>,
}

/// One import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Import {
    /// URI of the imported schema document.
    pub uri: String,
    /// Other fields (e.g. an integrity hash).
    #[serde(flatten)]
    pub extra: Map<String, Json>,
}

/// One node of `data`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Node path (in OPO: a UUID).
    pub path: String,
    /// Named children: name → path, or null to remove.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Links>,
    /// Named inherited nodes: name → path, or null to remove.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<Links>,
    /// Attributes: name → value, or null to remove.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Map<String, Json>>,
}

/// An error reading or composing documents.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read.
    Io(PathBuf, std::io::Error),
    /// A document is not valid IFCX JSON.
    Json(String, serde_json::Error),
    /// A `children` or `inherits` entry is not a path string or null.
    BadLink {
        /// The node path.
        path: String,
        /// The entry name.
        name: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Json(src, e) => write!(f, "{src}: {e}"),
            Error::BadLink { path, name } => {
                write!(f, "node {path}: link '{name}' is not a path or null")
            }
        }
    }
}

impl std::error::Error for Error {}

impl Document {
    /// Parse a document from JSON text. `source` names it in errors.
    pub fn from_json(text: &str, source: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|e| Error::Json(source.into(), e))
    }

    /// Read a document from a file.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::Io(path.into(), e))?;
        Self::from_json(&text, &path.display().to_string())
    }

    /// Write the document as pretty-printed JSON with a final newline.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("documents serialise");
        s.push('\n');
        s
    }
}

/// A node after composition: links and attributes with removals applied.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComposedNode {
    /// Named children → path.
    pub children: Map<String, Json>,
    /// Named inherited nodes → path.
    pub inherits: Map<String, Json>,
    /// Attributes.
    pub attributes: Map<String, Json>,
}

impl ComposedNode {
    /// Children as (name, path) pairs.
    pub fn child_paths(&self) -> impl Iterator<Item = (&str, &str)> {
        self.children
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|p| (k.as_str(), p)))
    }

    /// Inherited nodes as (name, path) pairs, weakest first.
    pub fn inherit_paths(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inherits
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|p| (k.as_str(), p)))
    }
}

/// The result of composing a stack of documents.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Composition {
    order: Vec<String>,
    composed: std::collections::HashMap<String, ComposedNode>,
}

fn merge(into: &mut Map<String, Json>, from: &Map<String, Json>) {
    for (k, v) in from {
        if v.is_null() {
            into.shift_remove(k);
        } else {
            into.insert(k.clone(), v.clone());
        }
    }
}

/// Compose documents, weakest first (see the crate documentation).
pub fn compose(documents: &[Document]) -> Result<Composition, Error> {
    let mut c = Composition::default();
    for doc in documents {
        for n in &doc.data {
            for links in [&n.children, &n.inherits] {
                for (k, v) in links.iter().flatten() {
                    if !(v.is_string() || v.is_null()) {
                        return Err(Error::BadLink {
                            path: n.path.clone(),
                            name: k.clone(),
                        });
                    }
                }
            }
            let node = c.composed.entry(n.path.clone()).or_insert_with(|| {
                c.order.push(n.path.clone());
                ComposedNode::default()
            });
            if let Some(m) = &n.children {
                merge(&mut node.children, m);
            }
            if let Some(m) = &n.inherits {
                merge(&mut node.inherits, m);
            }
            if let Some(m) = &n.attributes {
                merge(&mut node.attributes, m);
            }
        }
    }
    Ok(c)
}

impl Composition {
    /// Node paths in the order they first appear.
    pub fn paths(&self) -> &[String] {
        &self.order
    }

    /// The composed node at a path.
    pub fn node(&self, path: &str) -> Option<&ComposedNode> {
        self.composed.get(path)
    }

    /// Paths referenced by `children` or `inherits` that have no node.
    pub fn dangling(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for p in &self.order {
            let n = &self.composed[p];
            for (_, target) in n.child_paths().chain(n.inherit_paths()) {
                if !self.composed.contains_key(target) {
                    out.push((p.clone(), target.to_owned()));
                }
            }
        }
        out
    }

    /// A node's attributes including inherited ones. Inherited attributes are weaker than the
    /// node's own; a later entry of `inherits` is stronger than an earlier one. Cycles are cut.
    pub fn resolved_attributes(&self, path: &str) -> Map<String, Json> {
        let mut seen = HashSet::new();
        self.resolve(path, &mut seen)
    }

    fn resolve(&self, path: &str, seen: &mut HashSet<String>) -> Map<String, Json> {
        let mut out = Map::new();
        if !seen.insert(path.to_owned()) {
            return out;
        }
        if let Some(n) = self.composed.get(path) {
            for (_, parent) in n.inherit_paths() {
                let inherited = self.resolve(parent, seen);
                merge(&mut out, &inherited);
            }
            merge(&mut out, &n.attributes);
        }
        seen.remove(path);
        out
    }

    /// An object-valued attribute merged key by key along the inheritance chain: inherited
    /// keys are weaker than the node's own, and a `null` key removes an inherited key. OPO uses
    /// this for `opo::values`, so a size variant adds to the values of the system node it
    /// inherits (spec §4.14). Non-object values of the attribute are ignored.
    pub fn merged_attribute(&self, path: &str, name: &str) -> Map<String, Json> {
        let mut seen = HashSet::new();
        self.merge_attr(path, name, &mut seen)
    }

    fn merge_attr(&self, path: &str, name: &str, seen: &mut HashSet<String>) -> Map<String, Json> {
        let mut out = Map::new();
        if !seen.insert(path.to_owned()) {
            return out;
        }
        if let Some(n) = self.composed.get(path) {
            for (_, parent) in n.inherit_paths() {
                let inherited = self.merge_attr(parent, name, seen);
                merge(&mut out, &inherited);
            }
            if let Some(Json::Object(own)) = n.attributes.get(name) {
                merge(&mut out, own);
            }
        }
        seen.remove(path);
        out
    }

    /// The paths of nodes that carry an attribute, in order.
    pub fn with_attribute<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.order
            .iter()
            .filter(move |p| self.composed[*p].attributes.contains_key(name))
            .map(String::as_str)
    }

    /// The OPO type node: the node that carries `opo::object` (spec §5.1).
    pub fn type_node(&self) -> Option<&str> {
        self.with_attribute("opo::object").next()
    }
}

/// Two layers set the same attribute of the same node (not allowed, decision 0020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap {
    /// Node path.
    pub path: String,
    /// Attribute name.
    pub attribute: String,
    /// The two layers, in order.
    pub layers: (String, String),
}

/// Find attributes that more than one layer sets on the same node. Layers may add nodes and
/// add entries to `children` and `inherits`, but must not set the same attribute (spec §5.2).
pub fn layer_overlaps(layers: &[(String, Document)]) -> Vec<Overlap> {
    let mut first = std::collections::HashMap::<(String, String), String>::new();
    let mut out = Vec::new();
    for (name, doc) in layers {
        let mut here = HashSet::new();
        for n in &doc.data {
            for k in n.attributes.iter().flat_map(|a| a.keys()) {
                let key = (n.path.clone(), k.clone());
                if !here.insert(key.clone()) {
                    continue;
                }
                match first.get(&key) {
                    Some(other) => out.push(Overlap {
                        path: key.0.clone(),
                        attribute: key.1.clone(),
                        layers: (other.clone(), name.clone()),
                    }),
                    None => {
                        first.insert(key, name.clone());
                    }
                }
            }
        }
    }
    out
}

/// An unpacked OPO object folder: the base document and its layers.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDir {
    /// The folder.
    pub dir: PathBuf,
    /// `object.ifcx`.
    pub base: Document,
    /// `layers/*.ifcx` in file-name order, with their file names.
    pub layers: Vec<(String, Document)>,
}

/// Read an unpacked object folder (spec §5.2).
pub fn load_object_dir(dir: &Path) -> Result<ObjectDir, Error> {
    let base = Document::read(&dir.join("object.ifcx"))?;
    let mut layers = Vec::new();
    let ldir = dir.join("layers");
    if ldir.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&ldir)
            .map_err(|e| Error::Io(ldir.clone(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ifcx"))
            .collect();
        files.sort();
        for f in files {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            layers.push((name, Document::read(&f)?));
        }
    }
    Ok(ObjectDir {
        dir: dir.into(),
        base,
        layers,
    })
}

impl ObjectDir {
    /// Compose the base with the named layers (in the order given). Use an empty list for the
    /// base alone.
    pub fn compose_with(&self, layer_names: &[&str]) -> Result<Composition, Error> {
        let mut docs = vec![self.base.clone()];
        for name in layer_names {
            if let Some((_, d)) = self.layers.iter().find(|(n, _)| n == name) {
                docs.push(d.clone());
            }
        }
        compose(&docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(data: Json) -> Document {
        serde_json::from_value(json!({
            "header": {"id": "t", "ifcxVersion": "ifcx_alpha", "dataVersion": "0.1.0", "author": "a", "timestamp": "t"},
            "imports": [], "schemas": {}, "data": data
        }))
        .unwrap()
    }

    #[test]
    fn later_layer_wins_and_null_removes() {
        let a = doc(json!([{"path": "p", "attributes": {"x": 1, "y": 2}, "children": {"c": "q"}}]));
        let b =
            doc(json!([{"path": "p", "attributes": {"x": 3, "y": null}, "children": {"c": null}}]));
        let c = compose(&[a, b]).unwrap();
        let n = c.node("p").unwrap();
        assert_eq!(n.attributes, json!({"x": 3}).as_object().unwrap().clone());
        assert!(n.children.is_empty());
    }

    #[test]
    fn inherited_attributes_are_weaker() {
        let d = doc(json!([
            {"path": "type", "attributes": {"w": 1, "h": 2}},
            {"path": "sys", "inherits": {"type": "type"}, "attributes": {"h": 3}},
            {"path": "var", "inherits": {"system": "sys"}, "attributes": {"w": 4}}
        ]));
        let c = compose(&[d]).unwrap();
        assert_eq!(
            c.resolved_attributes("var"),
            json!({"w": 4, "h": 3}).as_object().unwrap().clone()
        );
    }

    #[test]
    fn values_merge_key_by_key_through_inheritance() {
        let d = doc(json!([
            {"path": "sys", "attributes": {"opo::values": {"frame": 1, "gap": 2}}},
            {"path": "var", "inherits": {"system": "sys"}, "attributes": {"opo::values": {"width": 3, "gap": null}}}
        ]));
        let c = compose(&[d]).unwrap();
        assert_eq!(
            c.merged_attribute("var", "opo::values"),
            json!({"frame": 1, "width": 3}).as_object().unwrap().clone()
        );
        // Plain resolution replaces the attribute whole, as IFCX does.
        assert_eq!(
            c.resolved_attributes("var")["opo::values"],
            json!({"width": 3, "gap": null})
        );
    }

    #[test]
    fn inheritance_cycles_are_cut() {
        let d = doc(json!([
            {"path": "a", "inherits": {"b": "b"}, "attributes": {"x": 1}},
            {"path": "b", "inherits": {"a": "a"}, "attributes": {"y": 2}}
        ]));
        let c = compose(&[d]).unwrap();
        assert_eq!(c.resolved_attributes("a").len(), 2);
    }

    #[test]
    fn dangling_links_are_reported() {
        let c = compose(&[doc(json!([{"path": "a", "children": {"x": "missing"}}]))]).unwrap();
        assert_eq!(c.dangling(), vec![("a".to_owned(), "missing".to_owned())]);
    }

    #[test]
    fn bad_link_is_an_error() {
        assert!(matches!(
            compose(&[doc(json!([{"path": "a", "children": {"x": 1}}]))]),
            Err(Error::BadLink { .. })
        ));
    }

    #[test]
    fn overlapping_layers_are_reported() {
        let a = doc(json!([{"path": "t", "attributes": {"x": 1}, "children": {"c": "p"}}]));
        let b = doc(
            json!([{"path": "t", "attributes": {"x": 2}, "children": {"d": "q"}}, {"path": "n", "attributes": {"x": 3}}]),
        );
        let o = layer_overlaps(&[("a".into(), a), ("b".into(), b)]);
        assert_eq!(
            o,
            vec![Overlap {
                path: "t".into(),
                attribute: "x".into(),
                layers: ("a".into(), "b".into())
            }]
        );
    }

    #[test]
    fn type_node_is_found() {
        let c = compose(&[doc(
            json!([{"path": "a"}, {"path": "t", "attributes": {"opo::object": {}}}]),
        )])
        .unwrap();
        assert_eq!(c.type_node(), Some("t"));
    }
}
