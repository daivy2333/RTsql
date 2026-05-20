#[cfg(test)]
mod tests {
    use rtsql::storage::PageId;

    #[test]
    fn test_page_id_offset() {
        let page_id = PageId(5);
        let offset = page_id.to_offset(4096);
        assert_eq!(offset, 20480); // 5 * 4096
    }

    #[test]
    fn test_page_id_zero() {
        let page_id = PageId(0);
        let offset = page_id.to_offset(4096);
        assert_eq!(offset, 0);
    }
}