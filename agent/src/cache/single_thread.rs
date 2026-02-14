// standard library
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

// internal crates
use crate::cache::{
    entry::CacheEntry,
    errors::{CacheElementNotFound, CacheErr, FoundTooManyCacheElements},
};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::info;

pub trait CacheKey: Debug + Clone + ToString + Serialize + DeserializeOwned + Eq + Hash {}

impl<K> CacheKey for K where K: Debug + Clone + ToString + Serialize + DeserializeOwned + Eq + Hash {}

pub trait CacheValue: Debug + Clone + Serialize + DeserializeOwned {}

impl<V> CacheValue for V where V: Debug + Clone + Serialize + DeserializeOwned {}

#[allow(async_fn_in_trait)]
pub trait SingleThreadCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    // -------------------------------- CUSTOM METHODS --------------------------------- //
    async fn read_entry_impl(&self, key: &K) -> Result<Option<CacheEntry<K, V>>, CacheErr>;

    async fn write_entry_impl(
        &mut self,
        entry: &CacheEntry<K, V>,
        overwrite: bool,
    ) -> Result<(), CacheErr>;

    async fn delete_entry_impl(&mut self, key: &K) -> Result<(), CacheErr>;

    async fn size(&self) -> Result<usize, CacheErr>;

    async fn prune_invalid_entries(&self) -> Result<(), CacheErr>;

    async fn capacity(&self) -> Result<usize, CacheErr>;

    async fn entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr>;

    async fn values(&self) -> Result<Vec<V>, CacheErr>;

    async fn entry_map(&self) -> Result<HashMap<K, CacheEntry<K, V>>, CacheErr>;

    async fn value_map(&self) -> Result<HashMap<K, V>, CacheErr>;

    // -------------------------------- TRAIT METHODS ---------------------------------- //
    async fn set_last_accessed(
        &mut self,
        entry: &mut CacheEntry<K, V>,
        last_accessed: DateTime<Utc>,
    ) -> Result<(), CacheErr> {
        entry.last_accessed = last_accessed;
        self.write_entry_impl(entry, true).await?;
        Ok(())
    }

    async fn read_entry_optional(&mut self, key: &K) -> Result<Option<CacheEntry<K, V>>, CacheErr> {
        let mut entry = match self.read_entry_impl(key).await? {
            Some(entry) => entry,
            None => return Ok(None),
        };

        // update the last accessed time
        self.set_last_accessed(&mut entry, Utc::now()).await?;

        Ok(Some(entry))
    }

    async fn read_entry(&mut self, key: &K) -> Result<CacheEntry<K, V>, CacheErr> {
        let result = self.read_entry_optional(key).await?;
        match result {
            Some(entry) => Ok(entry),
            None => Err(CacheErr::CacheElementNotFound(CacheElementNotFound {
                msg: format!("Unable to find cache entry with key: '{}'", key.to_string()),
                trace: trace!(),
            })),
        }
    }

    async fn read_optional(&mut self, key: &K) -> Result<Option<V>, CacheErr> {
        let entry = self.read_entry_optional(key).await?;
        match entry {
            Some(entry) => Ok(Some(entry.value)),
            None => Ok(None),
        }
    }

    async fn read(&mut self, key: &K) -> Result<V, CacheErr> {
        Ok(self.read_entry(key).await?.value)
    }

    async fn write_entry(
        &mut self,
        entry: &CacheEntry<K, V>,
        overwrite: bool,
    ) -> Result<(), CacheErr> {
        self.prune().await?;
        self.write_entry_impl(entry, overwrite).await?;
        Ok(())
    }

    async fn write<F>(
        &mut self,
        key: K,
        value: V,
        is_dirty: F,
        overwrite: bool,
    ) -> Result<(), CacheErr>
    where
        F: Fn(Option<&CacheEntry<K, V>>, &V) -> bool + Send + Sync,
    {
        // if the entry already exists, keep the original created_at time
        let (created_at, last_accessed, is_dirty) = match self.read_entry_optional(&key).await? {
            Some(existing_entry) => (
                existing_entry.created_at,
                Utc::now(),
                is_dirty(Some(&existing_entry), &value),
            ),
            None => {
                let now = Utc::now();
                (now, now, is_dirty(None, &value))
            }
        };
        let entry = CacheEntry {
            key,
            value,
            created_at,
            last_accessed,
            is_dirty,
        };

        // write the entry
        self.write_entry(&entry, overwrite).await?;
        Ok(())
    }

    async fn delete(&mut self, key: &K) -> Result<(), CacheErr> {
        self.delete_entry_impl(key).await?;
        Ok(())
    }

    async fn prune(&mut self) -> Result<(), CacheErr> {
        let capacity = self.capacity().await?;

        // check if there are too many files
        let size = self.size().await?;
        if size <= capacity {
            return Ok(());
        }

        info!(
            "Pruning cache {} from {:?} entries to {:?} entries...",
            std::any::type_name::<V>(),
            size,
            capacity
        );

        // prune the invalid entries first
        self.prune_invalid_entries().await?;

        // prune by last accessed time
        let mut entries = self.entries().await?;
        entries.sort_by_key(|entry| entry.last_accessed);
        let num_delete = entries.len() - capacity;
        for entry in entries.into_iter().take(num_delete) {
            self.delete(&entry.key).await?;
        }
        Ok(())
    }

    async fn find_entries_where<F>(&mut self, filter: F) -> Result<Vec<CacheEntry<K, V>>, CacheErr>
    where
        F: Fn(&CacheEntry<K, V>) -> bool,
    {
        let entries = self.entries().await?;
        let mut filtered_entries: Vec<CacheEntry<K, V>> =
            entries.into_iter().filter(|entry| filter(entry)).collect();

        // update the last accessed time
        for entry in filtered_entries.iter_mut() {
            self.set_last_accessed(entry, Utc::now()).await?;
        }

        Ok(filtered_entries)
    }

    async fn find_where<F>(&mut self, filter: F) -> Result<Vec<V>, CacheErr>
    where
        F: Fn(&V) -> bool,
    {
        let entries = self
            .find_entries_where(|entry| filter(&entry.value))
            .await?;
        let values = entries.into_iter().map(|entry| entry.value).collect();
        Ok(values)
    }

    async fn find_one_entry_optional<F>(
        &mut self,
        filter_name: &str,
        filter: F,
    ) -> Result<Option<CacheEntry<K, V>>, CacheErr>
    where
        F: Fn(&CacheEntry<K, V>) -> bool,
    {
        let entries = self.find_entries_where(filter).await?;
        if entries.len() > 1 {
            return Err(CacheErr::FoundTooManyCacheElements(
                FoundTooManyCacheElements {
                    expected_count: 1,
                    actual_count: entries.len(),
                    filter_name: filter_name.to_string(),
                    trace: trace!(),
                },
            ));
        }
        Ok(entries.into_iter().next())
    }

    async fn find_one_optional<F>(
        &mut self,
        filter_name: &str,
        filter: F,
    ) -> Result<Option<V>, CacheErr>
    where
        F: Fn(&V) -> bool,
    {
        let entries = self.find_where(filter).await?;
        if entries.len() > 1 {
            return Err(CacheErr::FoundTooManyCacheElements(
                FoundTooManyCacheElements {
                    expected_count: 1,
                    actual_count: entries.len(),
                    filter_name: filter_name.to_string(),
                    trace: trace!(),
                },
            ));
        }
        Ok(entries.into_iter().next())
    }

    async fn find_one_entry<F>(
        &mut self,
        filter_name: &str,
        filter: F,
    ) -> Result<CacheEntry<K, V>, CacheErr>
    where
        F: Fn(&CacheEntry<K, V>) -> bool,
    {
        let entry = self.find_one_entry_optional(filter_name, filter).await?;
        match entry {
            Some(entry) => Ok(entry),
            None => Err(CacheErr::CacheElementNotFound(CacheElementNotFound {
                msg: format!("Unable to find cache entry with filter: '{filter_name}'"),
                trace: trace!(),
            })),
        }
    }

    async fn find_one<F>(&mut self, filter_name: &str, filter: F) -> Result<V, CacheErr>
    where
        F: Fn(&V) -> bool,
    {
        let opt_value = self.find_one_optional(filter_name, filter).await?;
        match opt_value {
            Some(value) => Ok(value),
            None => Err(CacheErr::CacheElementNotFound(CacheElementNotFound {
                msg: format!("Unable to find cache entry with filter: '{filter_name}'"),
                trace: trace!(),
            })),
        }
    }

    async fn get_dirty_entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr> {
        let entries = self.entries().await?;
        let dirty_entries = entries.into_iter().filter(|entry| entry.is_dirty).collect();
        Ok(dirty_entries)
    }
}
