//! Spike: the newtype census (`ARCH-062`) and the field privacy count on
//! rust-analyzer's crates, on stable. It loads the workspace as the IDE does,
//! with no compile, and prints one line per struct of a workspace crate that
//! holds one field, `PhantomData` aside, over a primitive of the census list.
//! Line: `crate  struct  visibility  type`.

use std::path::Path;

use ra_ap_hir::{Adt, Crate, HasVisibility, HirDisplay, Module, ModuleDef, Visibility};
use ra_ap_ide_db::RootDatabase;
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_project_model::{CargoConfig, CargoFeatures, RustLibSource};

/// The census list of `ambre-spec-newtypes.md` section 1, by last segment.
const CENSUS: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64", "bool", "char", "String", "Uuid", "str", "[u8]", "Vec<u8>",
];

fn in_census(text: &str) -> bool {
    // rust-analyzer prints the default allocator parameter.
    let text = &text.replace(", Global>", ">");
    let bare = text.trim_start_matches('&');
    let bare = match bare.strip_prefix('\'') {
        Some(rest) => rest.split_once(' ').map_or(rest, |(_, t)| t),
        None => bare,
    };
    let bare = bare.trim_start_matches("mut ");
    let bare = bare
        .strip_prefix("Option<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(bare);
    let bare = bare
        .strip_prefix("Cow<")
        .and_then(|t| t.strip_suffix('>'))
        .map(|t| t.rsplit(", ").next().unwrap_or(t))
        .unwrap_or(bare)
        .trim_start_matches('&');
    CENSUS.contains(&bare)
}

fn walk(db: &RootDatabase, krate: Crate, module: Module, out: &mut Vec<String>) {
    let target = krate.to_display_target(db);
    for def in module.declarations(db) {
        let ModuleDef::Adt(Adt::Struct(s)) = def else {
            continue;
        };
        let fields: Vec<_> = s
            .fields(db)
            .into_iter()
            .filter(|f| {
                let ty = f.ty(db).display(db, target).to_string();
                !ty.starts_with("PhantomData")
            })
            .collect();
        let [field] = fields[..] else {
            continue;
        };
        let ty = field.ty(db).display(db, target).to_string();
        if std::env::var_os("RA_ALL").is_none() && !in_census(&ty) {
            continue;
        }
        let vis = match field.visibility(db) {
            Visibility::Public => "pub",
            Visibility::PubCrate(_) => "restricted",
            v if format!("{v:?}").contains("Implicit") => "private",
            _ => "restricted",
        };
        let name = krate
            .display_name(db)
            .map_or_else(String::new, |n| n.to_string());
        out.push(format!("{name}\t{}\t{vis}\t{ty}", s.name(db).as_str()));
    }
    for child in module.children(db) {
        walk(db, krate, child, out);
    }
}

fn main() -> anyhow::Result<()> {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let cargo = CargoConfig {
        features: CargoFeatures::All,
        sysroot: Some(RustLibSource::Discover),
        // `cfg(test)` code stays out, as the syn census leaves it out.
        set_test: false,
        ..CargoConfig::default()
    };
    let load = LoadCargoConfig {
        load_out_dirs_from_check: true,
        with_proc_macro_server: ProcMacroServerChoice::Sysroot,
        prefill_caches: false,
        num_worker_threads: 4,
        proc_macro_processes: 1,
    };
    let started = std::time::Instant::now();
    let (db, _vfs, _server) = load_workspace_at(Path::new(&root), &cargo, &load, &|_| {})?;
    eprintln!("loaded in {:?}", started.elapsed());
    let mut out = Vec::new();
    // The type queries of the next solver read the database from a thread
    // local, which this call sets.
    ra_ap_hir_ty::attach_db(&db, || {
        for krate in Crate::all(&db) {
            if !krate.origin(&db).is_local() {
                continue;
            }
            walk(&db, krate, krate.root_module(&db), &mut out);
        }
    });
    out.sort();
    out.dedup();
    for line in &out {
        println!("{line}");
    }
    eprintln!("census in {:?}", started.elapsed());
    Ok(())
}
