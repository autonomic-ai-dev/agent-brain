use agent_brain::config::Config;
use agent_brain::db::store::BrainStore;
use agent_brain::embed::Embedder;
use agent_brain::index::sync_index_opts;

#[test]
fn sync_indexes_rules_and_skills_then_skips_unchanged_files() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("AUTONOMIC_HOME", home.path());
    let config = Config::isolated(home.path().to_path_buf());
    config.ensure_dirs().unwrap();

    let rule = home.path().join("rules/style.mdc");
    let skill = home.path().join("skills/alpha/SKILL.md");
    let reference = home.path().join("skills/alpha/references/api.md");
    std::fs::create_dir_all(rule.parent().unwrap()).unwrap();
    std::fs::create_dir_all(skill.parent().unwrap().join("references")).unwrap();
    std::fs::write(&rule, "Prefer small functions.").unwrap();
    std::fs::write(
        &skill,
        "---\nname: alpha\ndescription: Dispatches fixtures.\n---\n# Alpha\n",
    )
    .unwrap();
    std::fs::write(&reference, "reference doc, not routable").unwrap();

    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = Embedder::deterministic();

    let first = sync_indexed(&store, &config, &embedder, false).unwrap();
    assert_eq!(first, 2);
    let mtimes = store.get_all_indexed_mtimes().unwrap();
    assert!(mtimes.contains_key(&rule.display().to_string()));
    assert!(mtimes.contains_key(&skill.display().to_string()));
    assert!(!mtimes.contains_key(&reference.display().to_string()));

    let second = sync_indexed(&store, &config, &embedder, true).unwrap();
    assert_eq!(second, 0);
}

fn sync_indexed(
    store: &BrainStore,
    config: &Config,
    embedder: &Embedder,
    changed_only: bool,
) -> anyhow::Result<usize> {
    sync_index_opts(store, config, embedder, None, changed_only, false)
}
