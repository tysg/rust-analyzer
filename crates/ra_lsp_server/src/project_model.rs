use std::path::{Path, PathBuf};

use cargo_metadata::{metadata_run, CargoOpt};
use ra_syntax::SmolStr;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    thread_watcher::{ThreadWatcher, Worker},
    Result,
};

#[derive(Debug, Clone)]
pub struct CargoWorkspace {
    packages: Vec<PackageData>,
    targets: Vec<TargetData>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Package(usize);
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Target(usize);

#[derive(Debug, Clone)]
struct PackageData {
    name: SmolStr,
    manifest: PathBuf,
    targets: Vec<Target>,
    is_member: bool,
}

#[derive(Debug, Clone)]
struct TargetData {
    pkg: Package,
    name: SmolStr,
    root: PathBuf,
    kind: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Bin,
    Lib,
    Example,
    Test,
    Bench,
    Other,
}

impl Package {
    pub fn name(self, ws: &CargoWorkspace) -> &str {
        ws.pkg(self).name.as_str()
    }
    pub fn root(self, ws: &CargoWorkspace) -> &Path {
        ws.pkg(self).manifest.parent().unwrap()
    }
    pub fn targets<'a>(self, ws: &'a CargoWorkspace) -> impl Iterator<Item = Target> + 'a {
        ws.pkg(self).targets.iter().cloned()
    }
    pub fn is_member(self, ws: &CargoWorkspace) -> bool {
        ws.pkg(self).is_member
    }
}

impl Target {
    pub fn package(self, ws: &CargoWorkspace) -> Package {
        ws.tgt(self).pkg
    }
    pub fn name(self, ws: &CargoWorkspace) -> &str {
        ws.tgt(self).name.as_str()
    }
    pub fn root(self, ws: &CargoWorkspace) -> &Path {
        ws.tgt(self).root.as_path()
    }
    pub fn kind(self, ws: &CargoWorkspace) -> TargetKind {
        ws.tgt(self).kind
    }
}

impl CargoWorkspace {
    pub fn from_cargo_metadata(path: &Path) -> Result<CargoWorkspace> {
        let cargo_toml = find_cargo_toml(path)?;
        let meta = metadata_run(
            Some(cargo_toml.as_path()),
            true,
            Some(CargoOpt::AllFeatures),
        )
        .map_err(|e| format_err!("cargo metadata failed: {}", e))?;
        let mut pkg_by_id = FxHashMap::default();
        let mut packages = Vec::new();
        let mut targets = Vec::new();

        let ws_members: FxHashSet<String> = meta
            .workspace_members
            .into_iter()
            .map(|it| it.raw)
            .collect();

        for meta_pkg in meta.packages {
            let pkg = Package(packages.len());
            let is_member = ws_members.contains(&meta_pkg.id);
            pkg_by_id.insert(meta_pkg.id.clone(), pkg);
            let mut pkg_data = PackageData {
                name: meta_pkg.name.into(),
                manifest: PathBuf::from(meta_pkg.manifest_path),
                targets: Vec::new(),
                is_member,
            };
            for meta_tgt in meta_pkg.targets {
                let tgt = Target(targets.len());
                targets.push(TargetData {
                    pkg,
                    name: meta_tgt.name.into(),
                    root: PathBuf::from(meta_tgt.src_path),
                    kind: TargetKind::new(meta_tgt.kind.as_slice()),
                });
                pkg_data.targets.push(tgt);
            }
            packages.push(pkg_data)
        }

        Ok(CargoWorkspace { packages, targets })
    }
    pub fn packages<'a>(&'a self) -> impl Iterator<Item = Package> + 'a {
        (0..self.packages.len()).map(Package)
    }
    pub fn target_by_root(&self, root: &Path) -> Option<Target> {
        self.packages()
            .filter_map(|pkg| pkg.targets(self).find(|it| it.root(self) == root))
            .next()
    }
    fn pkg(&self, pkg: Package) -> &PackageData {
        &self.packages[pkg.0]
    }
    fn tgt(&self, tgt: Target) -> &TargetData {
        &self.targets[tgt.0]
    }
}

fn find_cargo_toml(path: &Path) -> Result<PathBuf> {
    if path.ends_with("Cargo.toml") {
        return Ok(path.to_path_buf());
    }
    let mut curr = Some(path);
    while let Some(path) = curr {
        let candidate = path.join("Cargo.toml");
        if candidate.exists() {
            return Ok(candidate);
        }
        curr = path.parent();
    }
    bail!("can't find Cargo.toml at {}", path.display())
}

impl TargetKind {
    fn new(kinds: &[String]) -> TargetKind {
        for kind in kinds {
            return match kind.as_str() {
                "bin" => TargetKind::Bin,
                "test" => TargetKind::Test,
                "bench" => TargetKind::Bench,
                "example" => TargetKind::Example,
                _ if kind.contains("lib") => TargetKind::Lib,
                _ => continue,
            };
        }
        TargetKind::Other
    }
}

pub fn workspace_loader() -> (Worker<PathBuf, Result<CargoWorkspace>>, ThreadWatcher) {
    Worker::<PathBuf, Result<CargoWorkspace>>::spawn(
        "workspace loader",
        1,
        |input_receiver, output_sender| {
            input_receiver
                .map(|path| CargoWorkspace::from_cargo_metadata(path.as_path()))
                .for_each(|it| output_sender.send(it))
        },
    )
}
