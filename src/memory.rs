use crate::trap::{Result, Trap};

/// Page size used by Rune (matches Wasm).
pub const PAGE_SIZE: usize = 65_536;

/// Linear memory for a Rune instance.
///
/// On real hardware this would use mmap with guard pages; here we use a
/// Vec<u8> so the implementation works on all platforms without unsafe.
pub struct Memory {
    data: Vec<u8>,
    max_pages: Option<usize>,
}

impl Memory {
    pub fn new(initial_pages: usize, max_pages: Option<usize>) -> Self {
        let size = initial_pages * PAGE_SIZE;
        Memory {
            data: vec![0u8; size],
            max_pages,
        }
    }

    /// Current size in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Current size in pages.
    pub fn pages(&self) -> usize {
        self.data.len() / PAGE_SIZE
    }

    /// Raw base pointer (for zero-copy host access in the future).
    pub fn base(&self) -> *const u8 {
        self.data.as_ptr()
    }

    pub fn base_mut(&mut self) -> *mut u8 {
        self.data.as_mut_ptr()
    }

    /// Grow by `delta` pages. Returns old page count, or error.
    pub fn grow(&mut self, delta: usize) -> Result<usize> {
        let old_pages = self.pages();
        let new_pages = old_pages + delta;
        if let Some(max) = self.max_pages {
            if new_pages > max {
                return Err(Trap::OutOfMemory);
            }
        }
        self.data.resize(new_pages * PAGE_SIZE, 0);
        Ok(old_pages)
    }

    fn check(&self, offset: usize, len: usize) -> Result<()> {
        if offset
            .checked_add(len)
            .map(|end| end <= self.data.len())
            .unwrap_or(false)
        {
            Ok(())
        } else {
            Err(Trap::OutOfBounds)
        }
    }

    // ── Typed reads ──────────────────────────────────────────────────────────

    pub fn read_u8(&self, offset: usize) -> Result<u8> {
        self.check(offset, 1)?;
        Ok(self.data[offset])
    }

    pub fn read_u32(&self, offset: usize) -> Result<u32> {
        self.check(offset, 4)?;
        let bytes: [u8; 4] = self.data[offset..offset + 4].try_into().unwrap();
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn read_i32(&self, offset: usize) -> Result<i32> {
        self.read_u32(offset).map(|v| v as i32)
    }

    pub fn read_u64(&self, offset: usize) -> Result<u64> {
        self.check(offset, 8)?;
        let bytes: [u8; 8] = self.data[offset..offset + 8].try_into().unwrap();
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn read_i64(&self, offset: usize) -> Result<i64> {
        self.read_u64(offset).map(|v| v as i64)
    }

    pub fn read_f32(&self, offset: usize) -> Result<f32> {
        self.read_u32(offset).map(f32::from_bits)
    }

    pub fn read_f64(&self, offset: usize) -> Result<f64> {
        self.read_u64(offset).map(f64::from_bits)
    }

    pub fn read_bytes(&self, offset: usize, len: usize) -> Result<&[u8]> {
        self.check(offset, len)?;
        Ok(&self.data[offset..offset + len])
    }

    // ── Typed writes ─────────────────────────────────────────────────────────

    pub fn write_u8(&mut self, offset: usize, val: u8) -> Result<()> {
        self.check(offset, 1)?;
        self.data[offset] = val;
        Ok(())
    }

    pub fn write_u32(&mut self, offset: usize, val: u32) -> Result<()> {
        self.check(offset, 4)?;
        self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn write_i32(&mut self, offset: usize, val: i32) -> Result<()> {
        self.write_u32(offset, val as u32)
    }

    pub fn write_u64(&mut self, offset: usize, val: u64) -> Result<()> {
        self.check(offset, 8)?;
        self.data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn write_i64(&mut self, offset: usize, val: i64) -> Result<()> {
        self.write_u64(offset, val as u64)
    }

    pub fn write_f32(&mut self, offset: usize, val: f32) -> Result<()> {
        self.write_u32(offset, val.to_bits())
    }

    pub fn write_f64(&mut self, offset: usize, val: f64) -> Result<()> {
        self.write_u64(offset, val.to_bits())
    }

    pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<()> {
        self.check(offset, bytes.len())?;
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_size() {
        let m = Memory::new(2, None);
        assert_eq!(m.pages(), 2);
        assert_eq!(m.size(), 2 * PAGE_SIZE);
    }

    #[test]
    fn grow_within_limit() {
        let mut m = Memory::new(1, Some(4));
        let old = m.grow(2).unwrap();
        assert_eq!(old, 1);
        assert_eq!(m.pages(), 3);
    }

    #[test]
    fn grow_exceed_limit() {
        let mut m = Memory::new(1, Some(2));
        assert!(m.grow(5).is_err());
    }

    #[test]
    fn read_write_roundtrip() {
        let mut m = Memory::new(1, None);
        m.write_i32(0, -42).unwrap();
        assert_eq!(m.read_i32(0).unwrap(), -42);

        m.write_f64(16, std::f64::consts::PI).unwrap();
        assert!((m.read_f64(16).unwrap() - std::f64::consts::PI).abs() < 1e-15);
    }

    #[test]
    fn out_of_bounds() {
        let m = Memory::new(1, None);
        assert_eq!(m.read_u32(PAGE_SIZE - 2), Err(Trap::OutOfBounds));
    }

    #[test]
    fn zeroed_initial() {
        let m = Memory::new(1, None);
        for i in 0..PAGE_SIZE {
            assert_eq!(m.data[i], 0);
        }
    }
}
