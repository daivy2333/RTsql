use std::fmt;

/// 页标识符，从 0 开始编号
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub u64);

impl PageId {
    /// 将 PageId 转换为文件偏移量
    pub fn to_offset(&self, page_size: usize) -> u64 {
        self.0 * page_size as u64
    }

    /// 页号
    pub fn page_num(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PageId({})", self.0)
    }
}