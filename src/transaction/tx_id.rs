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

    /// Advance the counter so it is at least `max_used` (MS09 Iter000
    /// 001-replan, D10): the next [`TransactionId::allocate`] returns a value
    /// strictly greater than `max_used`. Used after recovery so ids are never
    /// reused across restarts. The CAS loop keeps the counter monotonic under
    /// concurrent allocators; a `max_used` at or below the current counter is
    /// a no-op.
    pub fn advance_past(&self, max_used: u64) {
        let mut observed = self.counter.load(Ordering::SeqCst);
        while max_used > observed {
            match self.counter.compare_exchange(
                observed,
                max_used,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(now) => observed = now,
            }
        }
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

    // MS09 Iter000 001-replan (T6-R3/D10): the allocator must never hand out
    // an id at or below a previously used one, including after a restart
    // hands recovery's max tx id to `advance_past`.

    #[test]
    fn test_advance_past_moves_counter_and_allocates_above() {
        let tx_id = TransactionId::new();
        tx_id.advance_past(100);
        assert_eq!(tx_id.current(), 100);
        assert_eq!(tx_id.allocate(), 101);
    }

    #[test]
    fn test_advance_past_below_current_is_noop() {
        let tx_id = TransactionId::new();
        assert_eq!(tx_id.allocate(), 1);
        assert_eq!(tx_id.allocate(), 2);
        tx_id.advance_past(1);
        assert_eq!(tx_id.current(), 2);
        assert_eq!(tx_id.allocate(), 3);
    }

    #[test]
    fn test_advance_past_concurrent_with_allocate_never_regresses() {
        let tx_id = Arc::new(TransactionId::new());
        tx_id.advance_past(50);

        let mut handles = vec![];
        for _ in 0..8 {
            let tx_id_clone = tx_id.clone();
            handles.push(thread::spawn(move || tx_id_clone.allocate()));
        }
        // Racing lower advances must never pull the counter back below the
        // watermark while allocations are in flight.
        for _ in 0..4 {
            let tx_id_clone = tx_id.clone();
            handles.push(thread::spawn(move || {
                tx_id_clone.advance_past(40);
                0u64
            }));
        }

        let mut ids: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        ids.sort();
        // The 8 allocate results land strictly above the watermark, distinct
        // and dense; the 4 advance results contribute the 0 padding.
        assert_eq!(ids[0..4].to_vec(), vec![0u64; 4]);
        assert_eq!(ids[4..].to_vec(), (51..=58).collect::<Vec<u64>>());
        assert!(tx_id.current() >= 58);
    }

    #[test]
    fn test_advance_past_concurrent_high_watermark_with_allocate() {
        let tx_id = Arc::new(TransactionId::new());
        let mut handles = vec![];
        for _ in 0..4 {
            let tx_id_clone = tx_id.clone();
            handles.push(thread::spawn(move || tx_id_clone.allocate()));
        }
        for _ in 0..4 {
            let tx_id_clone = tx_id.clone();
            handles.push(thread::spawn(move || {
                tx_id_clone.advance_past(100);
                0u64
            }));
        }

        let ids: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        // Allocated ids stay unique under the racing watermark push, and the
        // counter ends at or above the pushed watermark.
        let mut sorted: Vec<u64> = ids.clone();
        sorted.retain(|id| *id > 0);
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            4,
            "allocate results must be distinct: {ids:?}"
        );
        assert!(tx_id.current() >= 100);
    }
}
