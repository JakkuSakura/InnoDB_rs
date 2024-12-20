use std::fmt::Debug;

use anyhow::{anyhow, Error, Result};
use num_enum::TryFromPrimitive;
use tracing::error;

use crate::innodb::InnoDBError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[repr(u8)]
pub enum RecordType {
    Conventional = 0,
    NodePointer = 1,
    Infimum = 2,
    Supremum = 3,
}

#[derive(Debug, Clone, Copy)]
pub struct InfoFlags {
    pub min_rec: bool,
    pub deleted: bool,
}

impl InfoFlags {
    pub fn try_from_primitive(flags: u8) -> Result<InfoFlags> {
        if flags & (!0x3u8) != 0 {
            return Err(Error::msg("Unexpected bitfield value"));
        }

        Ok(InfoFlags {
            min_rec: (flags & 0x1) != 0,
            deleted: (flags & 0x2) != 0,
        })
    }
}

pub const RECORD_HEADER_FIXED_LENGTH: usize = 5;

#[derive(Debug, Clone)]
pub struct RecordHeader {
    pub info_flags: InfoFlags,   // 4 bit,
    pub num_records_owned: u8,   // 4-bit [Valid range 0-8]
    pub order: u16,              // 13 bits
    pub record_type: RecordType, // 3 bits
    pub next_record_offset: Option<u16>,
}

impl RecordHeader {
    pub fn try_from_offset(buffer: &[u8], offset: usize) -> Result<RecordHeader> {
        assert!(offset < u16::MAX as usize);
        if offset < RECORD_HEADER_FIXED_LENGTH {
            return Err(anyhow!(InnoDBError::InvalidLength));
        }
        let record_type_order = u16::from_be_bytes([buffer[offset - 4], buffer[offset - 3]]);
        let owned_flags = u8::from_be_bytes([buffer[offset - 5]]);
        Ok(RecordHeader {
            info_flags: InfoFlags::try_from_primitive(owned_flags >> 4)?,
            num_records_owned: owned_flags & 0xF,
            order: record_type_order >> 3,
            record_type: RecordType::try_from_primitive((record_type_order & 0x7) as u8)?,
            next_record_offset: (offset as u16)
                .checked_add_signed(i16::from_be_bytes([buffer[offset - 2], buffer[offset - 1]])),
        })
    }

    pub fn next_record_offset(&self) -> usize {
        self.next_record_offset.unwrap() as usize
    }
}

#[derive(Clone)]
pub struct Record<'a> {
    pub header: RecordHeader,
    pub offset: usize, // record starting offset in the buf, header is negative from that
    pub buf: &'a [u8],
}

impl<'a> Debug for Record<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Record")
            .field("header", &self.header)
            .field("offset", &self.offset)
            .finish()
    }
}

impl<'a> Record<'a> {
    pub fn try_from_offset(buffer: &'a [u8], offset: usize) -> Result<Record> {
        Ok(Record {
            header: RecordHeader::try_from_offset(buffer, offset)?,
            offset,
            buf: buffer,
        })
    }

    pub fn next(&self) -> Option<Record<'a>> {
        if self.header.record_type == RecordType::Supremum {
            return None;
        }
        match Self::try_from_offset(self.buf, self.header.next_record_offset()) {
            Ok(record) => Some(record),
            Err(e) => {
                error!("Non-Supremum record does not have next: {:?}", e);
                None
            }
        }
    }
}
