//! The `opo` command.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use opo_check::{DirResolver, IfcTemplates, Options, check_dir};

const USAGE: &str = "\
opo — tools for OPO objects (Open Parametric Object)

Usage:
  opo check [--root DIR] [--ifc FILE] FOLDER...

Commands:
  check   Validate unpacked object folders (object.ifcx + layers/): schema, names,
          evaluation of every type, option and variant, IFC names, baked geometry.

Options:
  --root DIR   Folder that object ids 'opo-format.org/<path>' resolve under (default: .)
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("check") => check(&args[1..]),
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
