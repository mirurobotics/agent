// internal crates
use crate::filesys;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug)]
pub struct Layout {
    pub filesystem_root: filesys::Dir,
}

impl Layout {
    pub fn new(filesystem_root: filesys::Dir) -> Self {
        Self { filesystem_root }
    }

    pub fn root(&self) -> filesys::Dir {
        self.filesystem_root
            .subdir("var")
            .subdir("lib")
            .subdir("miru")
    }

    pub fn temp_dir(&self) -> filesys::Dir {
        self.root().subdir("tmp")
    }

    pub fn auth(&self) -> AuthLayout {
        AuthLayout::new(self.root().subdir("auth"))
    }

    pub fn settings(&self) -> filesys::File {
        self.root().file("settings.json")
    }

    pub fn resources(&self) -> filesys::Dir {
        self.root().subdir("resources")
    }

    pub fn device(&self) -> filesys::File {
        self.root().file("device.json")
    }

    fn config_instances(&self) -> filesys::Dir {
        self.resources().subdir("config_instances")
    }

    pub fn config_instance_meta(&self) -> filesys::File {
        self.config_instances().file("metadata.json")
    }

    pub fn config_instance_content(&self) -> filesys::Dir {
        self.config_instances().subdir("contents")
    }

    pub fn deployments(&self) -> filesys::File {
        self.resources().file("deployments.json")
    }

    pub fn releases(&self) -> filesys::File {
        self.resources().file("releases.json")
    }

    pub fn git_commits(&self) -> filesys::File {
        self.resources().file("git_commits.json")
    }

    pub fn customer_configs(&self) -> filesys::Dir {
        self.filesystem_root
            .subdir("srv")
            .subdir("miru")
            .subdir("config_instances")
    }

    pub fn srv_temp_dir(&self) -> filesys::Dir {
        self.filesystem_root
            .subdir("srv")
            .subdir("miru")
            .subdir(".temp")
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::new(filesys::Dir::new("/"))
    }
}

pub struct AuthLayout {
    pub root: filesys::Dir,
}

impl AuthLayout {
    pub fn new(root: filesys::Dir) -> Self {
        Self { root }
    }

    pub fn private_key(&self) -> filesys::File {
        self.root.file("private_key.pem")
    }

    pub fn public_key(&self) -> filesys::File {
        self.root.file("public_key.pem")
    }

    pub fn token(&self) -> filesys::File {
        self.root.file("token.json")
    }
}
