//! Read the IFC 4.3 element classes and property set templates from buildingSMART's UML
//! sources (the `schemas/*.uml` files of the IFC4.x-development repository).
//!
//! IFC is published under CC BY-ND 4.0, so no IFC data is committed to the library: it is
//! read from a pinned checkout at build time. Only names, value types, enumeration values
//! and applicability are kept; definitions are not read.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use serde_json::{Value as Json, json};

use crate::Error;

/// A reference to a type: the id it points at and the UML kind of the target.
#[derive(Debug, Clone, Default)]
struct TypeRef {
    id: String,
    kind: String,
}

#[derive(Debug, Clone, Default)]
struct Attribute {
    name: String,
    ty: TypeRef,
    upper: String,
    comments: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct Class {
    name: String,
    is_abstract: bool,
    generals: Vec<String>,
    attributes: Vec<Attribute>,
}

#[derive(Debug, Default)]
struct Model {
    classes: HashMap<String, Class>,
    enums: HashMap<String, (String, Vec<String>)>,
    /// (client, supplier) of every dependency.
    dependencies: Vec<(String, String)>,
}

/// The id a reference points at: an `href` ends in `#id`.
fn fragment(s: &str) -> String {
    s.rsplit('#').next().unwrap_or(s).to_owned()
}

fn attr(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// What the parser is inside of.
#[derive(Debug)]
enum Frame {
    Class(String),
    Enum(String),
    Dependency(usize),
    Attribute,
    Generalization,
    Comment,
    Body,
    Other,
}

fn read_file(text: &str, model: &mut Model) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Frame> = Vec::new();
    let mut attribute: Option<Attribute> = None;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let (e, empty) = match &event {
            Event::Start(e) => (e.clone(), false),
            Event::Empty(e) => (e.clone(), true),
            Event::Text(t) => {
                if matches!(stack.last(), Some(Frame::Body)) {
                    let raw = t.decode().map_err(|e| e.to_string())?;
                    let body = quick_xml::escape::unescape(&raw)
                        .map_err(|e| e.to_string())?
                        .trim()
                        .to_owned();
                    if let Some(a) = attribute.as_mut() {
                        a.comments.push(body);
                    }
                }
                continue;
            }
            Event::End(_) => {
                if let Some(Frame::Attribute) = stack.pop() {
                    if let (Some(a), Some(id)) = (attribute.take(), current_class(&stack)) {
                        if let Some(c) = model.classes.get_mut(&id) {
                            c.attributes.push(a);
                        }
                    }
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let tag = e.local_name();
        let xmi_type = attr(&e, b"xmi:type").unwrap_or_default();
        let frame = match (tag.as_ref(), xmi_type.as_str()) {
            (b"packagedElement", "uml:Class") => {
                let id = attr(&e, b"xmi:id").unwrap_or_default();
                model.classes.insert(
                    id.clone(),
                    Class {
                        name: attr(&e, b"name").unwrap_or_default(),
                        is_abstract: attr(&e, b"isAbstract").as_deref() == Some("true"),
                        ..Class::default()
                    },
                );
                Frame::Class(id)
            }
            (b"packagedElement", "uml:Enumeration") => {
                let id = attr(&e, b"xmi:id").unwrap_or_default();
                model.enums.insert(
                    id.clone(),
                    (attr(&e, b"name").unwrap_or_default(), Vec::new()),
                );
                Frame::Enum(id)
            }
            (b"packagedElement", "uml:Dependency") => {
                model.dependencies.push((
                    attr(&e, b"client")
                        .map(|s| fragment(&s))
                        .unwrap_or_default(),
                    attr(&e, b"supplier")
                        .map(|s| fragment(&s))
                        .unwrap_or_default(),
                ));
                Frame::Dependency(model.dependencies.len() - 1)
            }
            (b"client" | b"supplier", _) => {
                if let (Some(Frame::Dependency(i)), Some(href)) = (stack.last(), attr(&e, b"href"))
                {
                    let d = &mut model.dependencies[*i];
                    if tag.as_ref() == b"client" {
                        d.0 = fragment(&href);
                    } else {
                        d.1 = fragment(&href);
                    }
                }
                Frame::Other
            }
            (b"ownedLiteral", _) => {
                if let Some(Frame::Enum(id)) = stack.last() {
                    if let Some(en) = model.enums.get_mut(id) {
                        en.1.push(attr(&e, b"name").unwrap_or_default());
                    }
                }
                Frame::Other
            }
            (b"generalization", _) => {
                if let (Some(Frame::Class(id)), Some(g)) = (stack.last(), attr(&e, b"general")) {
                    if let Some(c) = model.classes.get_mut(id) {
                        c.generals.push(fragment(&g));
                    }
                }
                Frame::Generalization
            }
            (b"general", _) => {
                if let (Some(Frame::Generalization), Some(href)) = (stack.last(), attr(&e, b"href"))
                {
                    if let Some(id) = current_class(&stack) {
                        if let Some(c) = model.classes.get_mut(&id) {
                            c.generals.push(fragment(&href));
                        }
                    }
                }
                Frame::Other
            }
            (b"ownedAttribute", _) if matches!(stack.last(), Some(Frame::Class(_))) => {
                attribute = Some(Attribute {
                    name: attr(&e, b"name").unwrap_or_default(),
                    ty: TypeRef {
                        id: attr(&e, b"type").map(|s| fragment(&s)).unwrap_or_default(),
                        kind: String::new(),
                    },
                    upper: "1".into(),
                    comments: Vec::new(),
                });
                Frame::Attribute
            }
            (b"type", _) if matches!(stack.last(), Some(Frame::Attribute)) => {
                if let (Some(a), Some(href)) = (attribute.as_mut(), attr(&e, b"href")) {
                    a.ty = TypeRef {
                        id: fragment(&href),
                        kind: xmi_type.trim_start_matches("uml:").to_owned(),
                    };
                }
                Frame::Other
            }
            (b"upperValue", _) if matches!(stack.last(), Some(Frame::Attribute)) => {
                if let Some(a) = attribute.as_mut() {
                    a.upper = attr(&e, b"value").unwrap_or_else(|| "0".into());
                }
                Frame::Other
            }
            (b"ownedComment", _) => Frame::Comment,
            (b"body", _) if matches!(stack.last(), Some(Frame::Comment)) => Frame::Body,
            _ => Frame::Other,
        };
        if empty {
            if let Frame::Attribute = frame {
                if let (Some(a), Some(id)) = (attribute.take(), current_class(&stack)) {
                    if let Some(c) = model.classes.get_mut(&id) {
                        c.attributes.push(a);
                    }
                }
            }
        } else {
            stack.push(frame);
        }
    }
    Ok(())
}

fn current_class(stack: &[Frame]) -> Option<String> {
    stack.iter().rev().find_map(|f| match f {
        Frame::Class(id) => Some(id.clone()),
        _ => None,
    })
}

const QUANTITIES: [(&str, &str); 8] = [
    ("Length", "Q_LENGTH"),
    ("Area", "Q_AREA"),
    ("Volume", "Q_VOLUME"),
    ("Mass", "Q_WEIGHT"),
    ("Weight", "Q_WEIGHT"),
    ("Count", "Q_COUNT"),
    ("Time", "Q_TIME"),
    ("Number", "Q_NUMBER"),
];

/// The name of the type an id points at (`dt_IfcLabel` → `IfcLabel`).
fn type_name(model: &Model, id: &str) -> String {
    model
        .classes
        .get(id)
        .map(|c| c.name.clone())
        .or_else(|| model.enums.get(id).map(|e| e.0.clone()))
        .unwrap_or_else(|| id.split_once('_').map_or(id, |(_, n)| n).to_owned())
}

impl Model {
    fn property(&self, a: &Attribute, qto: bool) -> Json {
        let target = type_name(self, &a.ty.id);
        let (template, measure, enumeration) = if let Some((_, items)) =
            self.enums.get(&a.ty.id).filter(|_| a.ty.kind != "DataType")
        {
            (
                "P_ENUMERATEDVALUE".to_owned(),
                Some("IfcLabel".to_owned()),
                Some(items.clone()),
            )
        } else if qto {
            let base = target
                .trim_start_matches("IfcQuantity")
                .trim_start_matches("Ifc")
                .trim_end_matches("Measure");
            let q = QUANTITIES.iter().find(|(k, _)| *k == base).map_or_else(
                || format!("Q_{}", base.to_uppercase()),
                |(_, q)| (*q).to_owned(),
            );
            (q, None, None)
        } else if let Some(c) = self.classes.get(&a.ty.id).or_else(|| {
            (a.ty.kind == "Class")
                .then(|| self.classes.get(&format!("cl_{target}")))
                .flatten()
        }) {
            if c.generals.iter().any(|g| g == "cl_PropertyTableValueBase") {
                let defining = c.attributes.first().map(|d| type_name(self, &d.ty.id));
                ("P_TABLEVALUE".to_owned(), defining, None)
            } else {
                ("P_REFERENCEVALUE".to_owned(), Some(c.name.clone()), None)
            }
        } else if a.ty.kind == "Class" {
            ("P_REFERENCEVALUE".to_owned(), Some(target), None)
        } else if a.comments.iter().any(|c| c == "bounded") {
            ("P_BOUNDEDVALUE".to_owned(), Some(target), None)
        } else if a.upper == "*" {
            ("P_LISTVALUE".to_owned(), Some(target), None)
        } else {
            ("P_SINGLEVALUE".to_owned(), Some(target), None)
        };
        let mut p = json!({"name": a.name, "template": template, "measure": measure});
        if let Some(items) = enumeration {
            p["enum"] = json!(items);
        }
        p
    }

    /// `Class` or `Class/PREDEFINED` for the supplier of a dependency.
    fn applicable(&self, supplier: &str) -> Option<String> {
        let c = self.classes.get(supplier)?;
        match c.name.split_once('.') {
            Some((en, item)) => Some(format!("{}/{item}", en.trim_end_matches("TypeEnum"))),
            None => Some(c.name.clone()),
        }
    }
}

/// The class tree under `IfcElement` (`[[depth, class, abstract, predefined types | null]…]`)
/// and the property and quantity set templates per class (`{class: [set…]}`), read from the
/// `*.uml` files in `dir`.
pub fn extract_ifc_uml(dir: &Path) -> Result<(Json, Json), Error> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::Io(dir.to_owned(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "uml"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(Error::Source(format!("{}: no .uml files", dir.display())));
    }
    let mut model = Model::default();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| Error::Io(f.clone(), e))?;
        read_file(&text, &mut model).map_err(|e| Error::Source(format!("{}: {e}", f.display())))?;
    }

    // Entity classes: real classes (no `Enum.ITEM` pseudo classes) by name.
    let entity = |c: &Class| c.name.starts_with("Ifc") && !c.name.contains('.');
    let by_name: HashMap<&str, (&str, &Class)> = model
        .classes
        .iter()
        .filter(|(_, c)| entity(c))
        .map(|(id, c)| (c.name.as_str(), (id.as_str(), c)))
        .collect();
    let mut children: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (name, (_, c)) in &by_name {
        for g in &c.generals {
            if let Some(parent) = model.classes.get(g).filter(|p| entity(p)) {
                children
                    .entry(parent.name.as_str())
                    .or_default()
                    .insert(name);
            }
        }
    }
    let predefined = |name: &str| -> Option<Vec<String>> {
        let en = format!("{name}TypeEnum");
        model
            .enums
            .values()
            .find(|(n, _)| *n == en)
            .map(|(_, items)| {
                items
                    .iter()
                    .filter(|i| *i != "USERDEFINED" && *i != "NOTDEFINED")
                    .cloned()
                    .collect()
            })
    };
    let mut elements = Vec::new();
    let mut todo = vec![("IfcElement", 0usize)];
    while let Some((name, depth)) = todo.pop() {
        let Some((_, c)) = by_name.get(name) else {
            continue;
        };
        elements.push(json!([depth, name, c.is_abstract, predefined(name)]));
        if let Some(kids) = children.get(name) {
            todo.extend(kids.iter().rev().map(|k| (*k, depth + 1)));
        }
    }
    if elements.is_empty() {
        return Err(Error::Source(format!(
            "{}: no IfcElement class",
            dir.display()
        )));
    }

    // Applicability of each property and quantity set.
    let mut applies: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (client, supplier) in &model.dependencies {
        let Some(set) = model.classes.get(client) else {
            continue;
        };
        if !(set.name.starts_with("Pset_") || set.name.starts_with("Qto_")) {
            continue;
        }
        if let Some(a) = model.applicable(supplier) {
            applies.entry(client.clone()).or_default().push(a);
        }
    }
    let mut psets = serde_json::Map::new();
    for e in &elements {
        let (Some(cls), Some(false)) = (e[1].as_str(), e[2].as_bool()) else {
            continue;
        };
        let type_cls = format!("{cls}Type");
        let mut sets: Vec<(String, Json)> = Vec::new();
        for (id, ents) in &applies {
            let base = |x: &str| x.split('/').next().unwrap_or(x).to_owned();
            let mine: Vec<&String> = ents
                .iter()
                .filter(|x| {
                    let b = base(x);
                    b == cls || b == type_cls
                })
                .collect();
            // Sets that apply to the class only through some predefined types are left
            // out for now, as in the IFC data the templates were first generated from.
            if !mine.iter().any(|x| !x.contains('/')) {
                continue;
            }
            let set = &model.classes[id];
            let qto = set.name.starts_with("Qto_");
            let applies_to: BTreeSet<&str> = mine
                .iter()
                .filter_map(|x| x.split_once('/').map(|(_, t)| t))
                .collect();
            let bases: BTreeSet<String> = ents
                .iter()
                .map(|x| {
                    let b = base(x);
                    b.strip_suffix("Type").map_or(b.clone(), str::to_owned)
                })
                .collect();
            let props: Vec<Json> = set
                .attributes
                .iter()
                .map(|a| model.property(a, qto))
                .collect();
            sets.push((
                set.name.clone(),
                json!({
                    "name": set.name,
                    "kind": if qto { "qto" } else { "pset" },
                    "generic": bases.len() > 6,
                    "applies_to": applies_to,
                    "props": props,
                }),
            ));
        }
        sets.sort_by(|a, b| a.0.cmp(&b.0));
        psets.insert(
            cls.to_owned(),
            Json::Array(sets.into_iter().map(|s| s.1).collect()),
        );
    }
    Ok((Json::Array(elements), Json::Object(psets)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small model in the same shape as the IFC UML sources (written for this test).
    const KERNEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<uml:Model xmlns:xmi="http://www.omg.org/spec/XMI/20131001" xmlns:uml="http://www.eclipse.org/uml2/5.0.0/UML" xmi:id="pk_a" name="a">
  <packagedElement xmi:type="uml:Package" xmi:id="pk_inner" name="inner">
    <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcElement" name="IfcElement" isAbstract="true"/>
    <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcBox" name="IfcBox">
      <generalization xmi:type="uml:Generalization" xmi:id="g1" general="cl_IfcElement"/>
      <ownedAttribute xmi:type="uml:Property" xmi:id="at1" name="PredefinedType" type="en_IfcBoxTypeEnum"/>
    </packagedElement>
    <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcBoxType" name="IfcBoxType"/>
    <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcBoxTypeEnum_TALL" name="IfcBoxTypeEnum.TALL">
      <generalization xmi:type="uml:Generalization" xmi:id="g2" general="cl_IfcBox"/>
    </packagedElement>
    <packagedElement xmi:type="uml:Enumeration" xmi:id="en_IfcBoxTypeEnum" name="IfcBoxTypeEnum">
      <ownedLiteral xmi:type="uml:EnumerationLiteral" xmi:id="l1" name="TALL"/>
      <ownedLiteral xmi:type="uml:EnumerationLiteral" xmi:id="l2" name="USERDEFINED"/>
      <ownedLiteral xmi:type="uml:EnumerationLiteral" xmi:id="l3" name="NOTDEFINED"/>
    </packagedElement>
  </packagedElement>
</uml:Model>
"#;
    const SETS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<uml:Model xmlns:xmi="http://www.omg.org/spec/XMI/20131001" xmlns:uml="http://www.eclipse.org/uml2/5.0.0/UML" xmi:id="pk_b" name="b">
  <packagedElement xmi:type="uml:Class" xmi:id="cl_Pset_BoxCommon" name="Pset_BoxCommon">
    <ownedComment xmi:type="uml:Comment" xmi:id="c0"><body>PSET_TYPEDRIVENOVERRIDE</body></ownedComment>
    <ownedAttribute xmi:type="uml:Property" xmi:id="p1" name="Label">
      <type xmi:type="uml:DataType" href="m.uml#dt_IfcLabel"/>
      <upperValue xmi:type="uml:LiteralUnlimitedNatural" xmi:id="p1u" value="1"/>
    </ownedAttribute>
    <ownedAttribute xmi:type="uml:Property" xmi:id="p2" name="State">
      <type xmi:type="uml:Enumeration" href="b.uml#en_PEnum_State"/>
      <upperValue xmi:type="uml:LiteralUnlimitedNatural" xmi:id="p2u" value="*"/>
    </ownedAttribute>
    <ownedAttribute xmi:type="uml:Property" xmi:id="p3" name="Range">
      <ownedComment xmi:type="uml:Comment" xmi:id="p3c"><body>bounded</body></ownedComment>
      <type xmi:type="uml:DataType" href="m.uml#dt_IfcLengthMeasure"/>
    </ownedAttribute>
    <ownedAttribute xmi:type="uml:Property" xmi:id="p4" name="Flags">
      <type xmi:type="uml:DataType" href="m.uml#dt_IfcBoolean"/>
      <upperValue xmi:type="uml:LiteralUnlimitedNatural" xmi:id="p4u" value="*"/>
    </ownedAttribute>
    <ownedAttribute xmi:type="uml:Property" xmi:id="p5" name="Table">
      <type xmi:type="uml:Class" href="b.uml#cl_IfcLabelIfcInteger"/>
    </ownedAttribute>
    <ownedAttribute xmi:type="uml:Property" xmi:id="p6" name="Doc">
      <type xmi:type="uml:Class" href="r.uml#cl_IfcExternalReference"/>
    </ownedAttribute>
  </packagedElement>
  <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcLabelIfcInteger" name="IfcLabelIfcInteger">
    <generalization xmi:type="uml:Generalization" xmi:id="g3">
      <general xmi:type="uml:Class" href="x.uml#cl_PropertyTableValueBase"/>
    </generalization>
    <ownedAttribute xmi:type="uml:Property" xmi:id="t1" name="DefiningValue">
      <type xmi:type="uml:DataType" href="m.uml#dt_IfcLabel"/>
    </ownedAttribute>
  </packagedElement>
  <packagedElement xmi:type="uml:Enumeration" xmi:id="en_PEnum_State" name="PEnum_State">
    <ownedLiteral xmi:type="uml:EnumerationLiteral" xmi:id="s1" name="NEW"/>
    <ownedLiteral xmi:type="uml:EnumerationLiteral" xmi:id="s2" name="OLD"/>
  </packagedElement>
  <packagedElement xmi:type="uml:Class" xmi:id="cl_Qto_BoxQuantities" name="Qto_BoxQuantities">
    <ownedAttribute xmi:type="uml:Property" xmi:id="q1" name="Length">
      <type xmi:type="uml:DataType" href="m.uml#dt_IfcLengthMeasure"/>
    </ownedAttribute>
    <ownedAttribute xmi:type="uml:Property" xmi:id="q2" name="Weight">
      <type xmi:type="uml:Class" href="q.uml#cl_IfcQuantityWeight"/>
    </ownedAttribute>
  </packagedElement>
  <packagedElement xmi:type="uml:Class" xmi:id="cl_Pset_TallOnly" name="Pset_TallOnly"/>
  <packagedElement xmi:type="uml:Dependency" xmi:id="d1" client="cl_Pset_BoxCommon" supplier="cl_IfcBox"/>
  <packagedElement xmi:type="uml:Dependency" xmi:id="d2" client="cl_Pset_BoxCommon">
    <supplier xmi:type="uml:Class" href="a.uml#cl_IfcBoxTypeEnum_TALL"/>
  </packagedElement>
  <packagedElement xmi:type="uml:Dependency" xmi:id="d3" client="cl_Qto_BoxQuantities" supplier="cl_IfcBoxType"/>
  <packagedElement xmi:type="uml:Dependency" xmi:id="d4" client="cl_Pset_TallOnly" supplier="cl_IfcBoxTypeEnum_TALL"/>
</uml:Model>
"#;

    #[test]
    fn reads_classes_and_property_sets() {
        let dir = std::env::temp_dir().join(format!("opo-ifc-uml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.uml"), KERNEL).unwrap();
        std::fs::write(dir.join("b.uml"), SETS).unwrap();
        let (elements, psets) = extract_ifc_uml(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            elements,
            json!([
                [0, "IfcElement", true, null],
                [1, "IfcBox", false, ["TALL"]]
            ])
        );
        let sets = psets["IfcBox"].as_array().unwrap();
        // Sorted by name; Pset_TallOnly applies only to TALL boxes and is left out.
        let names: Vec<_> = sets.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Pset_BoxCommon", "Qto_BoxQuantities"]);
        let common = &sets[0];
        assert_eq!(common["kind"], "pset");
        assert_eq!(common["generic"], false);
        assert_eq!(common["applies_to"], json!(["TALL"]));
        let templates: Vec<_> = common["props"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["template"].as_str().unwrap(), p["measure"].clone()))
            .collect();
        assert_eq!(
            templates,
            [
                ("P_SINGLEVALUE", json!("IfcLabel")),
                ("P_ENUMERATEDVALUE", json!("IfcLabel")),
                ("P_BOUNDEDVALUE", json!("IfcLengthMeasure")),
                ("P_LISTVALUE", json!("IfcBoolean")),
                ("P_TABLEVALUE", json!("IfcLabel")),
                ("P_REFERENCEVALUE", json!("IfcExternalReference")),
            ]
        );
        assert_eq!(common["props"][1]["enum"], json!(["NEW", "OLD"]));
        assert_eq!(sets[1]["kind"], "qto");
        assert_eq!(sets[1]["props"][0]["template"], "Q_LENGTH");
        assert_eq!(sets[1]["props"][1]["template"], "Q_WEIGHT");
        assert_eq!(sets[1]["props"][1]["measure"], Json::Null);
    }

    #[test]
    fn needs_uml_files() {
        let dir = std::env::temp_dir().join(format!("opo-ifc-uml-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(extract_ifc_uml(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
