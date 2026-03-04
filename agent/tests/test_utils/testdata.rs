use miru_agent::filesys;
use std::path::PathBuf;

// test file locations
pub fn testdata_dir() -> filesys::Dir {
    let project_root_path = env!("CARGO_MANIFEST_DIR");
    let miru_dir = filesys::Dir::new(project_root_path);
    miru_dir.parent().unwrap().subdir(PathBuf::from("testdata"))
}
