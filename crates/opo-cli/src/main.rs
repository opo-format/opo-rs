//! The `opo` command.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use opo_check::{DirResolver, IfcTemplates, Options, check_dir};
use opo_ifcx::Document;

const USAGE: &str = "\
opo — tools for OPO objects (Open Parametric Object)

Usage:
  opo check [--root DIR] [--ifc FILE] FOLDER...
  opo bake [--root DIR] [--base PATH] FOLDER...
  opo pack FOLDER [-o FILE]
  opo unpack FILE [-o FOLDER]
  opo build SOURCE... --library ROOT [--base PATH] [-o OUT] [--timestamp DATE] [--package]
  opo export FOLDER --base PATH --library ROOT -o SOURCE
  opo generate --library ROOT [--ifc DIR] [-o OUT] [--timestamp DATE] [CLASS...]
  opo ifc UML_DIR [-o OUT]
  opo catalogue --library ROOT [-o FILE.md]
  opo coverage --library ROOT --ifc DIR [-o FILE.md]
  opo compare FOLDER_A FOLDER_B [--ignore-generator]
  opo render FOLDER [--root DIR] [--view plan|model|elevation] [--representation ID]
             [--scale N] [--values JSON] [--inputs JSON] [-o FILE.svg]

Commands:
  check   Validate unpacked object folders (object.ifcx + layers/): schema, names,
          evaluation of every type, option and variant, IFC names, baked geometry.
  bake    Run each object's recipe at its defaults and rewrite the baked fallback
          geometry in its object.ifcx (meshes, plan curves, void).
  pack    Write the .opo package of an object folder (default: <folder>.opo).
  unpack  Extract a .opo package (default: a folder named after the file).
  build   Compile YAML object sources (object.yaml + layers/*.yaml, proposal 0002) with the
          library's registries, bake the fallback geometry, and write
          <OUT>/<base>/object.ifcx + layers (or a .opo package with --package).
          The base defaults to library/<source path relative to ROOT>; OUT defaults
          to <ROOT>/build. Nested objects resolve under OUT.
  export  Convert an object folder (IFCX) into YAML sources; registry entries are
          merged into the library's references.yaml and styles/.
  generate
          Generate class templates from <ROOT>/templates/archetypes/*.yaml,
          <ROOT>/templates/classes.yaml and the IFC data in DIR (elements.json and
          psets.json, as written by 'opo ifc'; default <ROOT>/ifc), then build them
          like 'opo build' into <OUT>/library/templates/<Class> (default: every class).
  ifc     Read the IFC 4.3 element classes and property set templates from
          buildingSMART's UML sources (schemas/*.uml of IFC4.x-development) and write
          OUT/elements.json and OUT/psets.json (default OUT: .).
  catalogue
          Write the object catalogue (Markdown) from <ROOT>/objects/*/object.yaml,
          planned entries included, and report problems in the entries (exit 1).
  coverage
          Write the IFC coverage table (Markdown): every concrete IFC element class with
          its archetype, template, predefined types and library objects, from the IFC
          data in DIR, <ROOT>/templates/classes.yaml and <ROOT>/objects; exit 1 if the
          per-class table names a class that is not a concrete element class.
  compare Compare two object folders (type node attributes, parameter order, baked
          parts, layer nodes); exit 1 on any difference.
  render  Draw an object as SVG: a hatched plan (default plan.medium at 1:50) or a
          shaded 3D view (model.medium). Values and inputs are JSON objects.

Options:
  --root DIR   Folder that object ids 'opo-format.org/<path>' resolve under (default: .)
  --base PATH  Repository path of the object, used for baked node ids
               (default: the folder relative to --root)
  -o PATH      Output file or folder
  --ifc FILE   IFC 4.3 property template table (psets.json written by 'opo ifc'); without
               it the IFC name check is skipped
  -h, --help   Show this help
";

struct CheckArgs {
    root: PathBuf,
    ifc: Option<PathBuf>,
    folders: Vec<PathBuf>,
}

fn parse_check(args: &[String]) -> Result<CheckArgs, String> {
    let mut a = CheckArgs {
        root: PathBuf::from("."),
        ifc: None,
        folders: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => a.root = it.next().ok_or("--root needs a folder")?.into(),
            "--ifc" => a.ifc = Some(it.next().ok_or("--ifc needs a file")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => a.folders.push(s.into()),
        }
    }
    if a.folders.is_empty() {
        return Err("check needs at least one object folder".into());
    }
    Ok(a)
}

fn check(args: &[String]) -> Result<bool, String> {
    let a = parse_check(args)?;
    let ifc = a.ifc.as_deref().map(IfcTemplates::read).transpose()?;
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: a.root.clone(),
    };
    let opts = Options {
        resolver: &resolver,
        ifc: ifc.as_ref(),
    };
    let mut all_ok = true;
    for f in &a.folders {
        let r = check_dir(Path::new(f), &opts);
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.display().to_string());
        let vols = if r.volume_checks > 0 {
            format!("; volume check {} cases", r.volume_checks)
        } else {
            String::new()
        };
        let verify = if r.to_verify.is_empty() {
            String::new()
        } else {
            format!(" ({} values to verify)", r.to_verify.len())
        };
        if r.ok() {
            println!("{name:<20} {:>3} cases{vols}: OK{verify}", r.cases);
        } else {
            all_ok = false;
            println!(
                "{name:<20} {:>3} cases{vols}: {} problem(s){verify}",
                r.cases,
                r.errors.len()
            );
            for e in &r.errors {
                println!("  - {e}");
            }
        }
    }
    if ifc.is_none() {
        eprintln!("note: no IFC template table; IFC property names were not checked");
    }
    Ok(all_ok)
}

fn bake(args: &[String]) -> Result<bool, String> {
    let (mut root, mut base, mut folders) =
        (PathBuf::from("."), None::<String>, Vec::<PathBuf>::new());
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => root = it.next().ok_or("--root needs a folder")?.into(),
            "--base" => base = Some(it.next().ok_or("--base needs a path")?.clone()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => folders.push(s.into()),
        }
    }
    if folders.is_empty() {
        return Err("bake needs at least one object folder".into());
    }
    if base.is_some() && folders.len() > 1 {
        return Err("--base applies to one folder only".into());
    }
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: root.clone(),
    };
    let root_abs = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    for f in &folders {
        let file = f.join("object.ifcx");
        let mut doc = Document::read(&file).map_err(|e| e.to_string())?;
        let base = match &base {
            Some(b) => b.clone(),
            None => {
                let abs = f
                    .canonicalize()
                    .map_err(|e| format!("{}: {e}", f.display()))?;
                abs.strip_prefix(&root_abs)
                    .map_err(|_| {
                        format!(
                            "{} is not under --root {}; give --base",
                            f.display(),
                            root.display()
                        )
                    })?
                    .to_string_lossy()
                    .replace('\\', "/")
            }
        };
        let r = opo_build::bake(&mut doc, &resolver, &base)
            .map_err(|e| format!("{}: {e}", f.display()))?;
        std::fs::write(&file, doc.to_json()).map_err(|e| format!("{}: {e}", file.display()))?;
        println!(
            "{}: {} meshes, {} curves (replaced {} nodes)",
            f.display(),
            r.meshes,
            r.curves,
            r.removed
        );
    }
    Ok(true)
}

/// `FOLDER [-o FILE]` or `FILE [-o FOLDER]`.
fn in_out(args: &[String], what: &str) -> Result<(PathBuf, Option<PathBuf>), String> {
    let (mut input, mut out) = (None::<PathBuf>, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" => out = Some(it.next().ok_or("-o needs a path")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s if input.is_none() => input = Some(s.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    Ok((input.ok_or(format!("{what} needs an input"))?, out))
}

fn pack(args: &[String]) -> Result<bool, String> {
    let (dir, out) = in_out(args, "pack")?;
    let out = out.unwrap_or_else(|| {
        let name = dir
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_owned()))
            .unwrap_or_else(|| "object".into());
        PathBuf::from(name).with_extension("opo")
    });
    let n = opo_build::pack(&dir, &out).map_err(|e| e.to_string())?;
    println!("{}: {n} files", out.display());
    Ok(true)
}

fn unpack(args: &[String]) -> Result<bool, String> {
    let (file, out) = in_out(args, "unpack")?;
    let out = out.unwrap_or_else(|| file.with_extension(""));
    let n = opo_build::unpack(&file, &out).map_err(|e| e.to_string())?;
    println!("{}: {n} files", out.display());
    Ok(true)
}

fn render(args: &[String]) -> Result<bool, String> {
    let (mut root, mut view, mut rep, mut scale) =
        (PathBuf::from("."), "plan".to_owned(), None::<String>, 50.0);
    let (mut values, mut inputs, mut out, mut folder) = (
        None::<String>,
        None::<String>,
        None::<PathBuf>,
        None::<PathBuf>,
    );
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--root" => root = next("a folder")?.into(),
            "--view" => view = next("plan, model or elevation")?,
            "--representation" => rep = Some(next("an id")?),
            "--scale" => {
                scale = next("a number")?
                    .parse()
                    .map_err(|_| "--scale needs a number".to_owned())?
            }
            "--values" => values = Some(next("a JSON object")?),
            "--inputs" => inputs = Some(next("a JSON object")?),
            "-o" => out = Some(next("a file")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s if folder.is_none() => folder = Some(s.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let folder = folder.ok_or("render needs an object folder")?;
    let json_map = |s: Option<String>,
                    what: &str|
     -> Result<serde_json::Map<String, serde_json::Value>, String> {
        match s {
            None => Ok(Default::default()),
            Some(s) => match serde_json::from_str(&s) {
                Ok(serde_json::Value::Object(m)) => Ok(m),
                _ => Err(format!("{what} must be a JSON object")),
            },
        }
    };
    let input = opo_recipe::Input {
        values: json_map(values, "--values")?,
        inputs: json_map(inputs, "--inputs")?,
        ..Default::default()
    };
    let def = opo_check::load_type(&folder)?;
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root,
    };
    let ev = opo_recipe::run(&def, &input, &resolver).map_err(|e| e.to_string())?;
    let svg = match view.as_str() {
        "plan" => opo_render::plan_svg(&def, &ev, rep.as_deref().unwrap_or("plan.medium"), scale),
        "model" => opo_render::model_svg(&def, &ev, rep.as_deref().unwrap_or("model.medium")),
        "elevation" => {
            // The 3D model seen from the front, with the first elevation representation that
            // has 2D items (opening symbols) drawn over it.
            let overlay = [
                "elevation",
                "elevation.medium",
                "elevation.low",
                "elevation.high",
            ]
            .into_iter()
            .find(|id| {
                def.representation(id)
                    .is_some_and(|r| r.items.as_ref().is_some_and(|i| !i.is_empty()))
            });
            opo_render::elevation_svg(
                &def,
                &ev,
                rep.as_deref().unwrap_or("model.medium"),
                overlay,
                scale,
            )
        }
        other => {
            return Err(format!(
                "--view must be plan, model or elevation, not {other}"
            ));
        }
    }
    .map_err(|e| e.to_string())?;
    match out {
        Some(f) => std::fs::write(&f, svg).map_err(|e| format!("{}: {e}", f.display()))?,
        None => print!("{svg}"),
    }
    Ok(true)
}

/// Today's date (UTC) as YYYY-MM-DD.
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn build(args: &[String]) -> Result<bool, String> {
    let (mut library, mut base, mut out, mut timestamp, mut package) = (
        None::<PathBuf>,
        None::<String>,
        None::<PathBuf>,
        today(),
        false,
    );
    let mut sources = Vec::<PathBuf>::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--library" => library = Some(next("a folder")?.into()),
            "--base" => base = Some(next("a path")?),
            "-o" => out = Some(next("a folder")?.into()),
            "--timestamp" => timestamp = next("a date")?,
            "--package" => package = true,
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => sources.push(s.into()),
        }
    }
    let library = library.ok_or("build needs --library ROOT")?;
    if sources.is_empty() {
        return Err("build needs at least one source folder".into());
    }
    if base.is_some() && sources.len() > 1 {
        return Err("--base applies to one source only".into());
    }
    let lib = opo_source::Library::load(&library).map_err(|e| e.to_string())?;
    let out = out.unwrap_or_else(|| library.join("build"));
    let root_abs = library
        .canonicalize()
        .map_err(|e| format!("{}: {e}", library.display()))?;
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: out.clone(),
    };
    for src in &sources {
        let base = match &base {
            Some(b) => b.clone(),
            None => {
                let abs = src
                    .canonicalize()
                    .map_err(|e| format!("{}: {e}", src.display()))?;
                let rel = abs.strip_prefix(&root_abs).map_err(|_| {
                    format!("{} is not under --library; give --base", src.display())
                })?;
                format!("library/{}", rel.to_string_lossy().replace('\\', "/"))
            }
        };
        let (object, _) = opo_source::read_source_dir(src).map_err(|e| e.to_string())?;
        if object.get("status").and_then(serde_json::Value::as_str) == Some("planned") {
            println!("{}: planned, not built", src.display());
            continue;
        }
        let built = opo_source::compile_dir(src, &lib, &base, &resolver, &timestamp)
            .map_err(|e| format!("{}: {e}", src.display()))?;
        for s in &built.skipped {
            println!(
                "{}: rule set {s} left out (values still to verify)",
                src.display()
            );
        }
        let mut doc = built.base;
        let r = opo_build::bake(&mut doc, &resolver, &base)
            .map_err(|e| format!("{}: {e}", src.display()))?;
        let dir = out.join(&base);
        std::fs::create_dir_all(dir.join("layers"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join("object.ifcx"), doc.to_json()).map_err(|e| e.to_string())?;
        for (f, d) in &built.layers {
            std::fs::write(dir.join("layers").join(f), d.to_json()).map_err(|e| e.to_string())?;
        }
        if built.layers.is_empty() {
            let _ = std::fs::remove_dir(dir.join("layers"));
        }
        if package {
            let file = dir.with_extension("opo");
            opo_build::pack(&dir, &file).map_err(|e| e.to_string())?;
            println!(
                "{}: {} ({} meshes, {} curves)",
                src.display(),
                file.display(),
                r.meshes,
                r.curves
            );
        } else {
            println!(
                "{}: {} ({} meshes, {} curves, {} layers)",
                src.display(),
                dir.display(),
                r.meshes,
                r.curves,
                built.layers.len()
            );
        }
    }
    Ok(true)
}

fn export(args: &[String]) -> Result<bool, String> {
    let (mut library, mut base, mut out, mut folder) = (
        None::<PathBuf>,
        None::<String>,
        None::<PathBuf>,
        None::<PathBuf>,
    );
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--library" => library = Some(next("a folder")?.into()),
            "--base" => base = Some(next("a path")?),
            "-o" => out = Some(next("a folder")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s if folder.is_none() => folder = Some(s.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let (folder, library) = (
        folder.ok_or("export needs an object folder")?,
        library.ok_or("export needs --library ROOT")?,
    );
    let (base, out) = (
        base.ok_or("export needs --base PATH (the object's repository path)")?,
        out.ok_or("export needs -o SOURCE")?,
    );
    let ex = opo_source::export_dir(&folder, &base).map_err(|e| e.to_string())?;
    let mut lib = opo_source::Library::load(&library).map_err(|e| e.to_string())?;
    lib.merge(&ex.library).map_err(|e| e.to_string())?;
    lib.save(&library).map_err(|e| e.to_string())?;
    ex.write(&out).map_err(|e| e.to_string())?;
    println!(
        "{}: object.yaml and {} layers; registries in {}",
        out.display(),
        ex.layers.len(),
        library.display()
    );
    Ok(true)
}

/// Write a compiled object, baked, into `<out>/<base>`.
fn write_built(
    built: opo_source::Compiled,
    out: &Path,
    base: &str,
    resolver: &DirResolver,
) -> Result<(PathBuf, opo_build::BakeReport), String> {
    let mut doc = built.base;
    let r = opo_build::bake(&mut doc, resolver, base).map_err(|e| e.to_string())?;
    let dir = out.join(base);
    std::fs::create_dir_all(dir.join("layers")).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("object.ifcx"), doc.to_json()).map_err(|e| e.to_string())?;
    for (f, d) in &built.layers {
        std::fs::write(dir.join("layers").join(f), d.to_json()).map_err(|e| e.to_string())?;
    }
    if built.layers.is_empty() {
        let _ = std::fs::remove_dir(dir.join("layers"));
    }
    Ok((dir, r))
}

fn generate(args: &[String]) -> Result<bool, String> {
    let (mut library, mut out, mut timestamp, mut only) = (
        None::<PathBuf>,
        None::<PathBuf>,
        today(),
        Vec::<String>::new(),
    );
    let mut ifc_dir = None::<PathBuf>;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--library" => library = Some(next("a folder")?.into()),
            "-o" => out = Some(next("a folder")?.into()),
            "--ifc" => ifc_dir = Some(next("a folder")?.into()),
            "--timestamp" => timestamp = next("a date")?,
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => only.push(s.into()),
        }
    }
    let library = library.ok_or("generate needs --library ROOT")?;
    let out = out.unwrap_or_else(|| library.join("build"));
    let mut lib = opo_source::Library::load(&library).map_err(|e| e.to_string())?;
    let sources = opo_source::TemplateSources::load(&library).map_err(|e| e.to_string())?;
    let ifc_dir = ifc_dir.unwrap_or_else(|| library.join("ifc"));
    let ifc = opo_source::IfcData::load(&ifc_dir).map_err(|e| e.to_string())?;
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: out.clone(),
    };
    for c in &only {
        if !sources.classes.contains_key(c) {
            return Err(format!("{c} is not in templates/classes.yaml"));
        }
    }
    for (class, entry) in &sources.classes {
        if !only.is_empty() && !only.contains(class) {
            continue;
        }
        let (src, layers, refs) = opo_source::generate(
            class,
            entry,
            &sources,
            &ifc,
            &opo_source::GeneratorOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        lib.merge(&refs).map_err(|e| e.to_string())?;
        let base = format!("library/templates/{class}");
        let built = opo_source::compile(&src, &layers, &lib, &base, &resolver, &timestamp)
            .map_err(|e| format!("{class}: {e}"))?;
        let (dir, r) =
            write_built(built, &out, &base, &resolver).map_err(|e| format!("{class}: {e}"))?;
        println!(
            "{class:<22} {} ({} meshes, {} curves)",
            dir.display(),
            r.meshes,
            r.curves
        );
    }
    Ok(true)
}

fn compare(args: &[String]) -> Result<bool, String> {
    let mut opts = opo_check::CompareOptions::default();
    let mut dirs = Vec::new();
    for a in args {
        match a.as_str() {
            "--ignore-generator" => opts.ignore_generator = true,
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => dirs.push(PathBuf::from(s)),
        }
    }
    let [a, b] = dirs.as_slice() else {
        return Err("compare needs two object folders".into());
    };
    let diffs = opo_check::compare_dirs(a, b, &opts)?;
    for d in &diffs {
        println!("  - {d}");
    }
    println!(
        "{}: {}",
        b.display(),
        if diffs.is_empty() {
            "same".to_owned()
        } else {
            format!("{} difference(s)", diffs.len())
        }
    );
    Ok(diffs.is_empty())
}

/// `opo ifc UML_DIR [-o OUT]`: write elements.json and psets.json from the IFC UML sources.
fn ifc(args: &[String]) -> Result<bool, String> {
    let (mut dir, mut out) = (None::<PathBuf>, PathBuf::from("."));
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" => out = it.next().ok_or("-o needs a folder")?.into(),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s if dir.is_none() => dir = Some(s.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let dir = dir.ok_or("ifc needs the folder of the .uml files")?;
    let (elements, psets) = opo_source::extract_ifc_uml(&dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for (name, v) in [("elements.json", &elements), ("psets.json", &psets)] {
        let p = out.join(name);
        std::fs::write(
            &p,
            serde_json::to_string(v).map_err(|e| e.to_string())? + "\n",
        )
        .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let sets: usize = psets.as_object().map_or(0, |m| {
        m.values()
            .filter_map(serde_json::Value::as_array)
            .map(Vec::len)
            .sum()
    });
    println!(
        "{} classes, {sets} property and quantity sets → {}",
        elements.as_array().map_or(0, Vec::len),
        out.display()
    );
    Ok(true)
}

/// `opo catalogue --library ROOT [-o FILE]`: the object catalogue as Markdown.
fn catalogue(args: &[String]) -> Result<bool, String> {
    let (mut library, mut out) = (None::<PathBuf>, None::<PathBuf>);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--library" => library = Some(it.next().ok_or("--library needs a folder")?.into()),
            "-o" => out = Some(it.next().ok_or("-o needs a file")?.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let library = library.ok_or("catalogue needs --library ROOT")?;
    let cat = opo_source::catalogue(&library).map_err(|e| e.to_string())?;
    let md = cat.to_markdown();
    match out {
        Some(f) => {
            if let Some(d) = f.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            std::fs::write(&f, md).map_err(|e| format!("{}: {e}", f.display()))?;
            println!("{} objects → {}", cat.entries.len(), f.display());
        }
        None => print!("{md}"),
    }
    for p in &cat.problems {
        println!("  - {p}");
    }
    Ok(cat.problems.is_empty())
}

/// `opo coverage --library ROOT --ifc DIR [-o FILE]`: the IFC coverage table as Markdown.
fn coverage(args: &[String]) -> Result<bool, String> {
    let (mut library, mut ifc, mut out) = (None::<PathBuf>, None::<PathBuf>, None::<PathBuf>);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--library" => library = Some(next("a folder")?.into()),
            "--ifc" => ifc = Some(next("a folder")?.into()),
            "-o" => out = Some(next("a file")?.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    let library = library.ok_or("coverage needs --library ROOT")?;
    let ifc = ifc.ok_or("coverage needs --ifc DIR (the output of opo ifc)")?;
    let cov = opo_source::coverage(&library, &ifc).map_err(|e| e.to_string())?;
    let md = cov.to_markdown();
    match out {
        Some(f) => {
            if let Some(d) = f.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            std::fs::write(&f, md).map_err(|e| format!("{}: {e}", f.display()))?;
            println!("{} classes → {}", cov.rows.len(), f.display());
        }
        None => print!("{md}"),
    }
    for c in &cov.unknown {
        println!("  - templates/classes.yaml: {c} is not a concrete IFC element class");
    }
    Ok(cov.unknown.is_empty())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("check") => check(&args[1..]),
        Some("bake") => bake(&args[1..]),
        Some("pack") => pack(&args[1..]),
        Some("unpack") => unpack(&args[1..]),
        Some("render") => render(&args[1..]),
        Some("build") => build(&args[1..]),
        Some("export") => export(&args[1..]),
        Some("generate") => generate(&args[1..]),
        Some("compare") => compare(&args[1..]),
        Some("ifc") => ifc(&args[1..]),
        Some("catalogue") => catalogue(&args[1..]),
        Some("coverage") => coverage(&args[1..]),
        Some(other) => Err(format!("unknown command '{other}'")),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("opo: {e}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
