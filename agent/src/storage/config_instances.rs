// internal crates
use crate::cache::{dir::DirCache, entry::CacheEntry, file::FileCache};
use crate::models::config_instance::{CfgInstID, ConfigInstance};

pub type CfgInstEntry = CacheEntry<CfgInstID, ConfigInstance>;

// The config instance storage is split into two parts: metadata is stored in a single file
// while the content is stored in a directory with a file for each entry. This is for
// performance reasons since accessing the config instances and storing them in a single
// file allows for better performance / caching by the OS. On the other hand, the actual
// configuration content can be quite large so they each need to be stored in their own
// file to maintain a small memory footprint. This is also why we have a separate store
// for the content.
pub type CfgInsts = FileCache<CfgInstID, ConfigInstance>;
pub type CfgInstContent = DirCache<CfgInstID, serde_json::Value>;
