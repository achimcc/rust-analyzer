//! Loads a Cargo project into a static instance of analysis, without support
//! for incorporating changes.
use std::{path::Path, sync::Arc};

use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver};
use hir::db::DefDatabase;
use ide::{AnalysisHost, Change};
use ide_db::base_db::CrateGraph;
use project_model::{
    BuildDataCollector, CargoConfig, ProcMacroClient, ProjectManifest, ProjectWorkspace,
};
use vfs::{loader::Handle, AbsPath, AbsPathBuf};

use crate::reload::{ProjectFolders, SourceRootConfig};

pub(crate) struct LoadCargoConfig {
    pub(crate) load_out_dirs_from_check: bool,
    pub(crate) wrap_rustc: bool,
    pub(crate) with_proc_macro: bool,
    pub(crate) prefill_caches: bool,
}

pub(crate) fn load_workspace_at(
    root: &Path,
    cargo_config: &CargoConfig,
    load_config: &LoadCargoConfig,
    progress: &dyn Fn(String),
) -> Result<(AnalysisHost, vfs::Vfs, Option<ProcMacroClient>)> {
    let root = AbsPathBuf::assert(std::env::current_dir()?.join(root));
    eprintln!("root = {:?}", root);
    let root = ProjectManifest::discover_single(&root)?;
    eprintln!("root = {:?}", root);
    let workspace = ProjectWorkspace::load(root, cargo_config, progress)?;

    load_workspace(workspace, load_config, progress)
}

fn load_workspace(
    ws: ProjectWorkspace,
    config: &LoadCargoConfig,
    progress: &dyn Fn(String),
) -> Result<(AnalysisHost, vfs::Vfs, Option<ProcMacroClient>)> {
    let lru_cap = std::env::var("RA_LRU_CAP").ok().and_then(|it| it.parse::<usize>().ok());
    let mut host = AnalysisHost::new(lru_cap);
    host.raw_database_mut().set_enable_proc_attr_macros(true);

    let (change, vfs, proc_macro_client) = load_change(ws, config, progress)?;

    host.apply_change(change);

    if config.prefill_caches {
        host.analysis().prime_caches(|_| {})?;
    }
    Ok((host, vfs, proc_macro_client))
}

pub(crate) fn load_change(
    ws: ProjectWorkspace,
    config: &LoadCargoConfig,
    progress: &dyn Fn(String),
) -> Result<(Change, vfs::Vfs, Option<ProcMacroClient>)> {
    let (sender, receiver) = unbounded();
    let mut vfs = vfs::Vfs::default();
    let mut loader = {
        let loader =
            vfs_notify::NotifyHandle::spawn(Box::new(move |msg| sender.send(msg).unwrap()));
        Box::new(loader)
    };

    let proc_macro_client = if config.with_proc_macro {
        let path = AbsPathBuf::assert(std::env::current_exe()?);
        Some(ProcMacroClient::extern_process(path, &["proc-macro"]).unwrap())
    } else {
        None
    };

    let build_data = if config.load_out_dirs_from_check {
        let mut collector = BuildDataCollector::new(config.wrap_rustc);
        ws.collect_build_data_configs(&mut collector);
        Some(collector.collect(progress)?)
    } else {
        None
    };

    let crate_graph = ws.to_crate_graph(
        build_data.as_ref(),
        proc_macro_client.as_ref(),
        &mut |path: &AbsPath| {
            let contents = loader.load_sync(path);
            let path = vfs::VfsPath::from(path.to_path_buf());
            vfs.set_file_contents(path.clone(), contents);
            vfs.file_id(&path)
        },
    );

    let project_folders = ProjectFolders::new(&[ws], &[], build_data.as_ref());
    loader.set_config(vfs::loader::Config {
        load: project_folders.load,
        watch: vec![],
        version: 0,
    });

    log::debug!("crate graph: {:?}", crate_graph);

    let change =
        load_crate_graph(crate_graph, project_folders.source_root_config, &mut vfs, &receiver);

    Ok((change, vfs, proc_macro_client))
}

fn load_crate_graph(
    crate_graph: CrateGraph,
    source_root_config: SourceRootConfig,
    vfs: &mut vfs::Vfs,
    receiver: &Receiver<vfs::loader::Message>,
) -> Change {
    let mut analysis_change = Change::new();

    // wait until Vfs has loaded all roots
    for task in receiver {
        match task {
            vfs::loader::Message::Progress { n_done, n_total, config_version: _ } => {
                if n_done == n_total {
                    break;
                }
            }
            vfs::loader::Message::Loaded { files } => {
                for (path, contents) in files {
                    vfs.set_file_contents(path.into(), contents);
                }
            }
        }
    }
    let changes = vfs.take_changes();
    for file in changes {
        if file.exists() {
            let contents = vfs.file_contents(file.file_id).to_vec();
            if let Ok(text) = String::from_utf8(contents) {
                analysis_change.change_file(file.file_id, Some(Arc::new(text)))
            }
        }
    }
    let source_roots = source_root_config.partition(vfs);
    analysis_change.set_roots(source_roots);

    analysis_change.set_crate_graph(crate_graph);

    analysis_change
}

#[cfg(test)]
mod tests {
    use super::*;

    use hir::Crate;

    #[test]
    fn test_loading_rust_analyzer() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let cargo_config = Default::default();
        let load_cargo_config = LoadCargoConfig {
            load_out_dirs_from_check: false,
            wrap_rustc: false,
            with_proc_macro: false,
            prefill_caches: false,
        };
        let (host, _vfs, _proc_macro) =
            load_workspace_at(path, &cargo_config, &load_cargo_config, &|_| {}).unwrap();

        let n_crates = Crate::all(host.raw_database()).len();
        // RA has quite a few crates, but the exact count doesn't matter
        assert!(n_crates > 20);
    }
}
