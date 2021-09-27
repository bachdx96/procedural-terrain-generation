use priority_queue::PriorityQueue;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::time::Instant;

pub struct Cache<K, V>
where
    K: std::hash::Hash + Eq,
{
    cache: HashMap<K, V>,
    last_accessed: PriorityQueue<K, Reverse<Instant>>,
    max_size: usize,
}

impl<K, V> Cache<K, V>
where
    K: Clone + std::hash::Hash + Eq,
{
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: HashMap::new(),
            last_accessed: PriorityQueue::new(),
            max_size,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.cache.get(key)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.cache.get_mut(key)
    }

    pub fn insert(&mut self, key: &K, value: V) {
        self.insert_with_priority(key, value, Reverse(Instant::now()))
    }

    pub fn insert_with_priority(&mut self, key: &K, value: V, priority: Reverse<Instant>) {
        self.last_accessed.push_decrease(key.clone(), priority);
        self.cache.insert(key.clone(), value);
        if self.cache.len() > self.max_size {
            let (key, _) = self.last_accessed.pop().unwrap();
            self.cache.remove(&key);
        }
    }

    pub fn update_last_accessed(&mut self, key: &K) {
        if self.cache.contains_key(key) {
            self.last_accessed
                .push_decrease(key.clone(), Reverse(Instant::now()));
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.last_accessed.clear();
    }

    pub fn values_mut(&mut self) -> std::collections::hash_map::ValuesMut<K, V> {
        self.cache.values_mut()
    }
}
