// internal crates
use crate::filesys::dir::Dir;
use crate::filesys::file::File;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug)]
pub struct StorageLayout {
    pub root: Dir,
}

impl StorageLayout {
    pub fn new(root: Dir) -> Self {
        Self { root }
    }

    pub fn internal_dir(&self) -> Dir {
        self.root.subdir("var").subdir("lib").subdir("miru")
    }

    pub fn temp_dir(&self) -> Dir {
        self.internal_dir().subdir("tmp")
    }

    pub fn auth_dir(&self) -> AuthLayout {
        AuthLayout::new(self.internal_dir().subdir("auth"))
    }

    pub fn device_file(&self) -> File {
        self.internal_dir().file("device.json")
    }

    pub fn settings_file(&self) -> File {
        self.internal_dir().file("settings.json")
    }

    pub fn caches_dir(&self) -> Dir {
        self.internal_dir().subdir("cache")
    }

    pub fn config_instance_caches(&self) -> Dir {
        self.caches_dir().subdir("config_instances")
    }

    pub fn config_instance_cache(&self) -> File {
        self.config_instance_caches().file("metadata.json")
    }

    pub fn config_instance_content_cache(&self) -> Dir {
        self.config_instance_caches().subdir("contents")
    }

    pub fn config_instance_deployment_dir(&self) -> Dir {
        self.root
            .subdir("srv")
            .subdir("miru")
            .subdir("config_instances")
    }
}

impl Default for StorageLayout {
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

    pub fn private_key_file(&self) -> File {
        self.root.file("private_key.pem")
    }

    pub fn public_key_file(&self) -> File {
        self.root.file("public_key.pem")
    }

    pub fn token_file(&self) -> File {
        self.root.file("token.json")
    }
}
