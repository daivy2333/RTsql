#[cfg(test)]
mod tests {
    use rtsql::storage::PageId;
    use rtsql::storage::Page;

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

    #[test]
    fn test_page_new() {
        let page_id = PageId(1);
        let page = Page::new(page_id);
        assert_eq!(page.id, page_id);
        assert_eq!(page.data.len(), Page::PAGE_SIZE);
        assert_eq!(page.data.as_ref(), &[0u8; Page::PAGE_SIZE][..]);
    }

    #[test]
    fn test_page_from_bytes() {
        let page_id = PageId(2);
        let bytes = vec![42u8; Page::PAGE_SIZE];
        let page = Page::from_bytes(page_id, &bytes).unwrap();
        assert_eq!(page.id, page_id);
        assert!(page.data.iter().all(|&b| b == 42));
    }

    #[test]
    fn test_page_from_bytes_wrong_size() {
        let page_id = PageId(3);
        let bytes = vec![0u8; 100]; // Wrong size
        let result = Page::from_bytes(page_id, &bytes);
        assert!(result.is_err());
    }
}