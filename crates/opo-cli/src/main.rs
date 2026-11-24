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

Commands:
  check   Validate unpacked object folders (object.ifcx + layers/): schema, names,
          evaluation of every type, option and variant, IFC names, baked geometry.
  bake    Run each object's recipe at its defaults and rewrite the baked fallback
          geometry in its object.ifcx (meshes, plan curves, void).
  pack    Write the .opo package of an object folder (default: <folder>.opo).
  unpack  Extract a .opo package (default: a folder named after the file).

Options:
  --root DIR   Folder that object ids 'opo-format.org/<path>' resolve under (default: .)
  --base PATH  Repository path of the object, used for baked node ids
               (default: the folder relative to --root)
  -o PATH      Output file or folder
  --ifc FILE   IFC 4.3 property template table (default: <root>/tools/ifc4x3_psets.json
               if it exists; otherwise the IFC name check is skipped)
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
    let ifc_path = a
        .ifc
        .clone()
        .or_else(|| Some(a.root.join("tools/ifc4x3_psets.json")).filter(|p| p.exists()));
    let ifc = ifc_path.as_deref().map(IfcTemplates::read).transpose()?;
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
        if r.ok() {
            println!("{name:<20} {:>3} cases{vols}: OK", r.cases);
        } else {
            all_ok = false;
            println!(
                "{name:<20} {:>3} cases{vols}: {} problem(s)",
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
