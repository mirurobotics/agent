// internal crates
use crate::filesys::dir::Dir;
use crate::filesys::file::File;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug)]
pub struct Layout {
    pub filesystem_root: Dir,
}

impl Layout {
    pub fn new(filesystem_root: Dir) -> Self {
        Self { filesystem_root }
    }

    pub fn root(&self) -> Dir {
        self.filesystem_root
            .subdir("var")
            .subdir("lib")
            .subdir("miru")
    }

    pub fn temp_dir(&self) -> Dir {
        self.root().subdir("tmp")
    }

    pub fn auth(&self) -> AuthLayout {
        AuthLayout::new(self.root().subdir("auth"))
    }

    pub fn settings(&self) -> File {
        self.root().file("settings.json")
    }

    pub fn resources(&self) -> Dir {
        self.root().subdir("resources")
    }

    pub fn device(&self) -> File {
        self.root().file("device.json")
    }

    fn config_instances(&self) -> Dir {
        self.resources().subdir("config_instances")
    }

    pub fn config_instance_meta(&self) -> File {
        self.config_instances().file("metadata.json")
    }

    pub fn config_instance_content(&self) -> Dir {
        self.config_instances().subdir("contents")
    }

    pub fn deployments(&self) -> File {
        self.resources().file("deployments.json")
    }

    pub fn customer_configs(&self) -> Dir {
        self.filesystem_root
            .subdir("srv")
            .subdir("miru")
            .subdir("config_instances")
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::new(Dir::new("/"))
    }
}

pub struct AuthLayout {
    pub root: Dir,
}

impl AuthLayout {
    pub fn new(root: Dir) -> Self {
        Self { root }
    }

    pub fn private_key(&self) -> File {
        self.root.file("private_key.pem")
    }

    pub fn public_key(&self) -> File {
        self.root.file("public_key.pem")
    }

    pub fn token(&self) -> File {
        self.root.file("token.json")
    }
}
