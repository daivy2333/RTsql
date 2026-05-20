use std::sync::atomic::{AtomicU64, Ordering};

pub struct TransactionId {
    counter: AtomicU64,
}

impl TransactionId {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    pub fn allocate(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current(&self) -> u64 {
        self.counter.load(Ordering::SeqCst)
    }
}

impl Default for TransactionId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_tx_id_allocate_single_thread() {
        let tx_id = TransactionId::new();

        let id1 = tx_id.allocate();
        let id2 = tx_id.allocate();
        let id3 = tx_id.allocate();

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
        assert_eq!(tx_id.current(), 3);
    }

    #[test]
    fn test_tx_id_allocate_multi_thread() {
        let tx_id = Arc::new(TransactionId::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let tx_id_clone = tx_id.clone();
            handles.push(thread::spawn(move || tx_id_clone.allocate()));
        }

        let mut ids: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        ids.sort();

        assert_eq!(ids, (1..=10).collect::<Vec<u64>>());
        assert_eq!(tx_id.current(), 10);
    }
}
