# M1 设计规范：文件/缓存层

> 创建时间：2026-05-20
> 状态：Draft

---

## 一、设计目标

实现单文件持久化数据库的底层存储和缓存系统：

- **AsyncStorage trait**：定义异步存储接口，支持未来扩展（内存模式、io_uring）
- **FileStorage**：单文件持久化实现，使用 `spawn_blocking` 执行文件 I/O
- **BufferPool**：异步页缓存管理器，Clock 淘汰策略，支持并发读
- **PageFuture**：异步页访问包装，支持零拷贝读和写后写回

---

## 二、关键设计决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 存储模式 | 单文件持久化 | 嵌入式场景，部署简单，类似 SQLite |
| 页大小 | 4KB | SQLite 默认值，广泛验证，磁盘 I/O 和内存平衡 |
| 缓存容量 | 1000 页（约 4MB） | 嵌入式场景适用，足够缓存热点数据 |
| 淘汰策略 | Clock（近似 LRU） | 比纯 LRU 更好适应扫描访问，实现适中 |
| 并发控制 | 异步 RwLock | 读操作可并发，写操作独占，符合数据库读多写少特性 |

---

## 三、架构设计

### 3.1 模块结构

```
src/storage/
├── mod.rs           # 模块导出
├── async_storage.rs # AsyncStorage trait 定义
├── file_storage.rs  # FileStorage 实现
├── buffer_pool.rs   # BufferPool 管理器
├── page.rs          # Page 结构定义
└── page_id.rs       # PageId 类型定义
```

### 3.2 组件关系图

```
┌─────────────────────────────────────────────────────────┐
│                    上层调用者                             │
│                 (Executor/Transaction)                   │
└─────────────────────┬───────────────────────────────────┘
                      │ get_page(page_id)
                      ▼
┌─────────────────────────────────────────────────────────┐
│                   BufferPool                             │
│  - pages: HashMap<PageId, PageFrame>                    │
│  - clock_hand: Clock 淘汰指针                            │
│  - lock: RwLock (异步)                                  │
│  - storage: Arc<dyn AsyncStorage>                       │
│                                                         │
│  pub async fn get_page(&self, page_id) -> PageGuard    │
└─────────────────────┬───────────────────────────────────┘
                      │ read_page(page_id) / write_page()
                      ▼
┌─────────────────────────────────────────────────────────┐
│               AsyncStorage (trait)                       │
│  async fn read_page(&self, page_id) -> Result<Page>    │
│  async fn write_page(&self, page_id, &Page) -> Result  │
│  async fn allocate_page(&self) -> Result<PageId>       │
│  async fn sync(&self) -> Result<()>                     │
└─────────────────────┬───────────────────────────────────┘
                      │ 实现
                      ▼
┌─────────────────────────────────────────────────────────┐
│                   FileStorage                            │
│  - file: Arc<File>                                      │
│  - page_size: usize = 4096                              │
│  - file_len: AtomicU64                                  │
│                                                         │
│  impl AsyncStorage:                                     │
│    - read_page → spawn_blocking(file.read_exact)       │
│    - write_page → spawn_blocking(file.write_all)       │
│    - allocate_page → fetch_add + file.set_len           │
│    - sync → spawn_blocking(file.sync_all)              │
└─────────────────────────────────────────────────────────┘
                      │
                      ▼
              ┌──────────────┐
              │   单文件      │
              │  database.db │
              └──────────────┘
```

---

## 四、核心数据结构

### 4.1 PageId

```rust
/// 页标识符，从 0 开始编号
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageId(pub u64);

impl PageId {
    /// 将 PageId 转换为文件偏移量
    pub fn to_offset(&self, page_size: usize) -> u64 {
        self.0 * page_size as u64
    }
}
```

### 4.2 Page

```rust
/// 固定大小的页，4KB
#[derive(Debug, Clone)]
pub struct Page {
    pub id: PageId,
    pub data: [u8; PAGE_SIZE],
}

impl Page {
    pub const PAGE_SIZE: usize = 4096;

    pub fn new(id: PageId) -> Self {
        Self {
            id,
            data: [0u8; PAGE_SIZE],
        }
    }

    /// 从字节切片创建页（用于文件读取）
    pub fn from_bytes(id: PageId, bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() == PAGE_SIZE, "Page size mismatch");
        let mut page = Self::new(id);
        page.data.copy_from_slice(bytes);
        Ok(page)
    }
}
```

### 4.3 PageFrame（缓存帧）

```rust
/// 缓存中的页帧，包含元数据
struct PageFrame {
    page: Page,
    dirty: bool,      // 是否被修改
    ref_count: u32,   // 引用计数（防止淘汰正在使用的页）
    clock_bit: bool,  // Clock 算法标记位
}

impl PageFrame {
    fn new(page: Page) -> Self {
        Self {
            page,
            dirty: false,
            ref_count: 0,
            clock_bit: true, // 新加载的页，初始为 true
        }
    }
}
```

### 4.4 PageGuard（页访问守卫）

```rust
/// 页访问守卫，类似 RwLockReadGuard
/// Drop 时自动减少引用计数
pub struct PageGuard<'a> {
    frame: Arc<Mutex<PageFrame>>,
    pool: &'a BufferPool,
}

impl<'a> Deref for PageGuard<'a> {
    type Target = Page;
    fn deref(&self) -> &Self::Target {
        &self.frame.lock().page
    }
}

impl<'a> Drop for PageGuard<'a> {
    fn drop(&mut self) {
        // 减少引用计数
        self.frame.lock().ref_count -= 1;
    }
}

impl<'a> PageGuard<'a> {
    /// 标记页为脏页，需要在淘汰时写回
    pub fn mark_dirty(&self) {
        self.frame.lock().dirty = true;
    }
}
```

---

## 五、AsyncStorage Trait

```rust
use async_trait::async_trait;

#[async_trait]
pub trait AsyncStorage: Send + Sync {
    /// 读取指定页
    async fn read_page(&self, page_id: PageId) -> Result<Page>;

    /// 写入指定页
    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()>;

    /// 分配新页，返回 PageId
    async fn allocate_page(&self) -> Result<PageId>;

    /// 同步到磁盘（fsync）
    async fn sync(&self) -> Result<()>;

    /// 返回页大小
    fn page_size(&self) -> usize {
        Page::PAGE_SIZE
    }
}
```

---

## 六、FileStorage 实现

### 6.1 结构定义

```rust
use std::fs::File;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct FileStorage {
    file: Arc<File>,
    page_size: usize,
    /// 文件长度（页数），用于分配新页
    file_len: AtomicU64,
}

impl FileStorage {
    /// 打开或创建数据库文件
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let page_size = Page::PAGE_SIZE;

        // 验证文件长度是页大小的整数倍
        ensure!(
            file_len % page_size as u64 == 0,
            "File size not aligned to page size"
        );

        let page_count = file_len / page_size as u64;

        Ok(Self {
            file: Arc::new(file),
            page_size,
            file_len: AtomicU64::new(page_count),
        })
    }
}
```

### 6.2 AsyncStorage 实现

```rust
#[async_trait]
impl AsyncStorage for FileStorage {
    async fn read_page(&self, page_id: PageId) -> Result<Page> {
        let file = self.file.clone();
        let page_size = self.page_size;
        let offset = page_id.to_offset(page_size);

        // 使用 spawn_blocking 在 blocking 线程池执行文件 I/O
        let bytes = spawn_blocking(move || {
            use std::io::{Read, Seek, SeekFrom};

            let mut file = file.as_ref();
            file.seek(SeekFrom::Start(offset))?;

            let mut buf = vec![0u8; page_size];
            file.read_exact(&mut buf)?;
            Ok::<_, Error>(buf)
        })
        .await??;

        Page::from_bytes(page_id, &bytes)
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
        let file = self.file.clone();
        let page_size = self.page_size;
        let offset = page_id.to_offset(page_size);
        let data = page.data.clone();

        spawn_blocking(move || {
            use std::io::{Seek, SeekFrom, Write};

            let mut file = file.as_ref();
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(&data)?;
            Ok::<_, Error>(())
        })
        .await??;

        Ok(())
    }

    async fn allocate_page(&self) -> Result<PageId> {
        // 原子地分配新页 ID
        let page_id = self.file_len.fetch_add(1, Ordering::SeqCst);
        let offset = PageId(page_id).to_offset(self.page_size);

        let file = self.file.clone();
        spawn_blocking(move || {
            file.as_ref().set_len(offset + self.page_size as u64)?;
            Ok::<_, Error>(())
        })
        .await??;

        Ok(PageId(page_id))
    }

    async fn sync(&self) -> Result<()> {
        let file = self.file.clone();
        spawn_blocking(move || {
            file.as_ref().sync_all()?;
            Ok::<_, Error>(())
        })
        .await??;
        Ok(())
    }
}
```

---

## 七、BufferPool 实现

### 7.1 结构定义

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct BufferPool {
    /// 缓存页，PageId -> PageFrame
    pages: Arc<RwLock<HashMap<PageId, Arc<Mutex<PageFrame>>>>>,
    /// Clock 淘汰指针（指向 pages.keys 的位置）
    clock_hand: Arc<RwLock<Vec<PageId>>>,
    /// 最大缓存页数
    capacity: usize,
    /// 底层存储
    storage: Arc<dyn AsyncStorage>,
}

impl BufferPool {
    pub fn new(capacity: usize, storage: Arc<dyn AsyncStorage>) -> Self {
        Self {
            pages: Arc::new(RwLock::new(HashMap::new())),
            clock_hand: Arc::new(RwLock::new(Vec::new())),
            capacity,
            storage,
        }
    }

    /// 获取页（缓存未命中则从存储加载）
    pub async fn get_page(&self, page_id: PageId) -> Result<PageGuard<'_>> {
        // 1. 读锁检查缓存
        {
            let pages = self.pages.read().await;
            if let Some(frame) = pages.get(&page_id) {
                frame.lock().ref_count += 1;
                frame.lock().clock_bit = true;
                return Ok(PageGuard {
                    frame: frame.clone(),
                    pool: self,
                });
            }
        }

        // 2. 写锁加载页（可能需要淘汰）
        let mut pages = self.pages.write().await;

        // Double check（避免并发重复加载）
        if let Some(frame) = pages.get(&page_id) {
            frame.lock().ref_count += 1;
            frame.lock().clock_bit = true;
            return Ok(PageGuard {
                frame: frame.clone(),
                pool: self,
            });
        }

        // 3. 缓存满则淘汰
        if pages.len() >= self.capacity {
            self.evict_one(&mut pages).await?;
        }

        // 4. 从存储加载页
        let page = self.storage.read_page(page_id).await?;
        let frame = Arc::new(Mutex::new(PageFrame::new(page)));

        pages.insert(page_id, frame.clone());
        self.clock_hand.write().await.push(page_id);

        frame.lock().ref_count += 1;

        Ok(PageGuard {
            frame,
            pool: self,
        })
    }
}
```

### 7.2 Clock 淘汰算法

```rust
impl BufferPool {
    /// 使用 Clock 算法淘汰一页
    async fn evict_one(
        &self,
        pages: &mut HashMap<PageId, Arc<Mutex<PageFrame>>>,
    ) -> Result<()> {
        let mut clock_hand = self.clock_hand.write().await;

        loop {
            if clock_hand.is_empty() {
                bail!("Buffer pool full and no evictable page");
            }

            let candidate_id = clock_hand.remove(0); // 取出第一个

            if let Some(frame) = pages.get(&candidate_id) {
                let mut frame = frame.lock();

                // 跳过正在使用的页
                if frame.ref_count > 0 {
                    clock_hand.push(candidate_id); // 放回队尾
                    continue;
                }

                // Clock bit 为 true，重置并放回队尾
                if frame.clock_bit {
                    frame.clock_bit = false;
                    clock_hand.push(candidate_id);
                    continue;
                }

                // 找到淘汰候选：dirty page 先写回
                if frame.dirty {
                    self.storage.write_page(candidate_id, &frame.page).await?;
                }

                pages.remove(&candidate_id);
                return Ok(());
            }
        }
    }
}
```

---

## 八、数据流场景

### 8.1 Happy Path

| 场景 | 流程 |
|------|------|
| **启动打开文件** | `FileStorage::open(path)` → 创建文件或打开已有文件 |
| **首次读页（缓存未命中）** | `BufferPool::get_page(page_id)` → 未命中 → `storage.read_page` → 放入缓存 → 返回 PageGuard |
| **再次读页（缓存命中）** | `BufferPool::get_page(page_id)` → 命中 → ref_count++ → 返回 PageGuard |
| **写入修改页** | `PageGuard::mark_dirty()` → 标记 dirty = true |
| **缓存满淘汰** | `evict_one()` → Clock 扫描 → dirty page 先 write_page → 淘汰 |
| **关闭持久化** | `BufferPool::flush_all()` → 写回所有 dirty pages → `storage.sync()` |

### 8.2 Sad Path

| 场景 | 处理 |
|------|------|
| **文件不存在** | `FileStorage::open(path)` 使用 `create(true)`，自动创建 |
| **权限不足** | `open()` 返回 Error，向上传播 |
| **磁盘空间不足** | `allocate_page()` 或 `write_page()` 返回 Error，向上传播 |

### 8.3 Edge Cases

| 场景 | 处理 |
|------|------|
| **缓存容量为 0** | 初始化时检查 `capacity > 0`，否则返回错误 |
| **并发访问同一页** | RwLock 保护，PageGuard 引用计数，淘汰时跳过 ref_count > 0 的页 |
| **淘汰正被使用的页** | Clock 算法跳过 ref_count > 0 的页 |
| **文件大小超限** | 不主动处理，依赖操作系统错误传播 |

---

## 九、默认假设（BDD 场景补充）

| 场景类别 | 假设 |
|---------|------|
| **启动行为** | 文件不存在自动创建，不验证配置合法性（capacity > 0 除外） |
| **错误处理** | 不处理页损坏（M2 负责），磁盘错误向上传播，不自动重试 |
| **并发安全** | 并发访问同一页安全，依赖 RwLock + 引用计数 |
| **Dirty Page** | 写回后淘汰，不主动 flush（除非用户调用 `sync()`） |
| **崩溃恢复** | 不保证（M3 事务层负责），仅保证 `sync()` 后持久化 |

---

## 十、测试策略

### 10.1 单元测试

| 模块 | 测试点 |
|------|--------|
| **PageId** | 偏移量计算正确性 |
| **Page** | 创建、字节转换 |
| **FileStorage** | 打开文件、读写页、分配页、sync |
| **BufferPool** | 缓存命中、缓存未命中、淘汰逻辑、Clock 算法 |

### 10.2 集成测试

| 场景 | 测试 |
|------|------|
| **基础流程** | 打开文件 → 读页 → 写页 → 关闭 |
| **缓存淘汰** | 读超过 capacity 的页数，验证淘汰 |
| **并发访问** | 多协程并发读写，验证数据一致性 |
| **持久化** | 写入 → sync → 重启 → 验证数据 |

### 10.3 测试框架

- 使用 `tokio::test` 运行异步测试
- 使用 `tempfile` 创建临时文件
- 使用 `proptest` 进行属性测试

---

## 十一、错误处理

### 11.1 错误类型

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Page size mismatch: expected {expected}, got {actual}")]
    PageSizeMismatch { expected: usize, actual: usize },

    #[error("Buffer pool full, no evictable page")]
    BufferPoolFull,

    #[error("Invalid page id: {0}")]
    InvalidPageId(u64),
}

pub type Result<T> = std::result::Result<T, StorageError>;
```

### 11.2 错误传播

- 所有错误使用 `Result<T, StorageError>`
- 使用 `thiserror` 定义错误类型
- 使用 `anyhow` 的 `ensure!` 宏进行前置条件检查
- 错误向上传播，不自动重试

---

## 十二、性能考量

### 12.1 并发性能

- RwLock 读锁并发，写锁独占
- Clock 算法避免全局锁（仅写锁淘汰时需要）
- spawn_blocking 不阻塞 Tokio 运行时

### 12.2 I/O 性能

- 页大小 4KB，对齐文件系统块大小
- 批量操作可考虑预读（未来优化）
- sync() 可选择性调用

### 12.3 内存占用

- 固定容量 1000 页，约 4MB
- PageGuard 零拷贝（直接引用缓存）

---

## 十三、依赖项

| 依赖 | 版本 | 用途 |
|------|------|------|
| `tokio` | 1.x | 异步运行时，RwLock，spawn_blocking |
| `async-trait` | 0.1 | AsyncStorage trait 定义 |
| `thiserror` | 1.x | 错误类型定义 |
| `anyhow` | 1.x | ensure! 宏 |
| `tempfile` | 3.x | 测试临时文件 |

---

## 十四、扩展点

| 扩展需求 | 如何支持 |
|---------|---------|
| **内存模式** | 实现 `InMemoryStorage`，无需 spawn_blocking |
| **io_uring** | 实现 `IoUringStorage`，使用 tokio-uring |
| **可配置缓存容量** | BufferPool::new(capacity) 已支持 |
| **更优淘汰策略** | 替换 evict_one 实现（如 LRU-K、2Q） |
| **预读优化** | 在 AsyncStorage trait 添加 read_pages 批量读取 |

---

## 十五、里程碑完成标准

| 标准 | 验证方式 |
|------|---------|
| **功能完整** | 所有 API 实现并通过单元测试 |
| **缓存淘汰** | Clock 算法正确工作，测试覆盖淘汰场景 |
| **并发安全** | 多协程并发测试通过，无数据竞争 |
| **持久化** | 写入后重启，数据正确恢复 |
| **代码质量** | cargo clippy 无警告，rustfmt 格式化 |