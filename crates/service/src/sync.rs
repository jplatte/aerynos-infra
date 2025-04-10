//! Thread-safe synchronization types
use std::{collections::HashMap, hash::Hash, sync::Arc};

use tokio::sync::Mutex;

/// A thread-safe [`HashMap`] protected by a [`Mutex`]
#[derive(Debug, Clone)]
pub struct SharedMap<K, V>(Arc<Mutex<HashMap<K, V>>>);

impl<K, V> Default for SharedMap<K, V> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(HashMap::default())))
    }
}

impl<K, V> SharedMap<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Inserts a key-value pair into the map
    pub async fn insert(&self, key: K, value: V) {
        self.0.lock().await.insert(key, value);
    }

    /// Removes a key from the map, returning the value at the key if the key
    /// was previously in the map.
    pub async fn remove(&self, key: &K) -> Option<V> {
        self.0.lock().await.remove(key)
    }
}
