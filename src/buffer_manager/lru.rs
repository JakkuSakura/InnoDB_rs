use super::BufferManager;
use crate::page::{Page, PAGE_SIZE};
use crate::InnoDBError;
use anyhow::{anyhow, Result};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    slice,
    time::SystemTime,
};
use tracing::trace;

const LRU_PAGE_COUNT: usize = 16;

pub struct LRUBufferManager {
    backing_store: Vec<[u8; PAGE_SIZE]>,
    page_pin_counter: RefCell<Vec<u32>>,
    page_directory: PathBuf,
    page_pin_map: RefCell<HashMap<u32, usize>>,
    lru_list: RefCell<Vec<u64>>,
}

impl LRUBufferManager {
    pub fn new<P>(dir: P) -> Self
    where
        P: AsRef<Path>,
    {
        let mut buffer_manager = LRUBufferManager {
            backing_store: Vec::new(),
            page_pin_counter: RefCell::new(Vec::new()),
            page_directory: dir.as_ref().to_owned(),
            page_pin_map: RefCell::new(HashMap::new()),
            lru_list: RefCell::new(Vec::new()),
        };
        buffer_manager
            .backing_store
            .resize(LRU_PAGE_COUNT, [0u8; PAGE_SIZE]);
        buffer_manager
            .page_pin_counter
            .borrow_mut()
            .resize(LRU_PAGE_COUNT, 0);
        buffer_manager
            .lru_list
            .borrow_mut()
            .resize(LRU_PAGE_COUNT, 0);
        buffer_manager
    }

    pub fn find_free(&self) -> usize {
        let mut min_timestamp = u64::MAX;
        let mut result_frame = 0;
        let page_pin_counter = self.page_pin_counter.borrow();
        for (idx, timestamp) in self.lru_list.borrow().iter().enumerate() {
            if *timestamp == 0 {
                return idx;
            }
            // find unpinned page
            if *timestamp < min_timestamp && page_pin_counter[idx] == 0 {
                min_timestamp = *timestamp;
                result_frame = idx;
            }
        }
        if min_timestamp != u64::MAX {
            let mut borrowed_pin_map = self.page_pin_map.borrow_mut();
            let (offset, _) = borrowed_pin_map
                .iter()
                .find(|(_, val)| **val == result_frame)
                .unwrap_or_else(|| {
                    panic!(
                        "can't find the frame({result_frame}), {:#?}, pinmap: {:#?}",
                        self, borrowed_pin_map
                    )
                });
            let offset = *offset;

            borrowed_pin_map.remove(&offset);
            self.lru_list.borrow_mut()[result_frame] = 0;
            result_frame
        } else {
            panic!("pin too many pages, \nState: {:#?}", self);
        }
    }
}

impl std::fmt::Debug for LRUBufferManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LRUBufferManager")
            .field("page_pin_counter", &self.page_pin_counter)
            .field("page_directory", &self.page_directory)
            .field("page_pin_map", &self.page_pin_map)
            .field("lru_list", &self.lru_list)
            .finish()
    }
}

impl BufferManager for LRUBufferManager {
    fn pin(&self, offset: u32) -> Result<&Page> {
        trace!("Pinning {}", offset);
        let cur_sys_time = SystemTime::now();
        let current_time = cur_sys_time
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos() as u64;

        // If we have the page already pinned
        if let Some(frame_number) = self.page_pin_map.borrow().get(&(offset)) {
            self.page_pin_counter.borrow_mut()[*frame_number] += 1;
            self.lru_list.borrow_mut()[*frame_number] = current_time;
            let page = Page::from_bytes(&self.backing_store[*frame_number]);
            return Ok(page);
        }

        // If we don't have page already pinned
        let mut file = File::open(self.page_directory.join("0.pages".to_string()))?;
        file.seek(SeekFrom::Start(offset as u64 * PAGE_SIZE as u64))?;
        let free_frame = self.find_free();
        file.read_exact(unsafe {
            let selected_frame = &self.backing_store[free_frame];
            slice::from_raw_parts_mut(selected_frame.as_ptr() as *mut u8, PAGE_SIZE)
        })?;

        // Validate page *FIRST*
        let page = Page::from_bytes(&self.backing_store[free_frame]);
        if page.header().page_id.0 == 0 {
            return Err(anyhow!(InnoDBError::PageNotFound));
        }

        assert_eq!(page.header().page_id.0, offset);

        // Can't fail from this point on, so we update internal state

        self.lru_list.borrow_mut()[free_frame] = current_time;
        self.page_pin_counter.borrow_mut()[free_frame] += 1;
        self.page_pin_map.borrow_mut().insert(offset, free_frame);

        Ok(page)
    }

    fn unpin(&self, page: &Page) {
        let offset = page.header().page_id;
        trace!("Unpinning {}", offset);
        if let Some(frame_number) = self.page_pin_map.borrow().get(&offset.0) {
            self.page_pin_counter.borrow_mut()[*frame_number] -= 1;
        } else {
            panic!("Unpinning a non-pinned page");
        }
    }
}
