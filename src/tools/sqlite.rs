use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// SQLite Data Types & Representation
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqlValue {
    pub fn to_string_repr(&self) -> String {
        match self {
            SqlValue::Null => "NULL".to_string(),
            SqlValue::Integer(i) => i.to_string(),
            SqlValue::Real(f) => {
                if f.fract() == 0.0 {
                    format!("{:.1}", f)
                } else {
                    f.to_string()
                }
            }
            SqlValue::Text(s) => s.clone(),
            SqlValue::Blob(b) => {
                format!("BLOB({} bytes, 0x{})", b.len(), hex_preview(b, 16))
            }
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            SqlValue::Null => Value::Null,
            SqlValue::Integer(i) => json!(i),
            SqlValue::Real(f) => json!(f),
            SqlValue::Text(s) => json!(s),
            SqlValue::Blob(b) => {
                let hex_str: String = b.iter().map(|byte| format!("{:02x}", byte)).collect();
                json!(format!("0x{}", hex_str))
            }
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            SqlValue::Integer(i) => Some(*i),
            SqlValue::Real(f) => Some(*f as i64),
            SqlValue::Text(s) => s.parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SqlValue::Integer(i) => Some(*i as f64),
            SqlValue::Real(f) => Some(*f),
            SqlValue::Text(s) => s.parse::<f64>().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SqlValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, SqlValue::Null)
    }

    pub fn compare(&self, other: &SqlValue) -> std::cmp::Ordering {
        match (self, other) {
            (SqlValue::Null, SqlValue::Null) => std::cmp::Ordering::Equal,
            (SqlValue::Null, _) => std::cmp::Ordering::Less,
            (_, SqlValue::Null) => std::cmp::Ordering::Greater,

            (SqlValue::Integer(a), SqlValue::Integer(b)) => a.cmp(b),
            (SqlValue::Real(a), SqlValue::Real(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (SqlValue::Integer(a), SqlValue::Real(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (SqlValue::Real(a), SqlValue::Integer(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),

            (SqlValue::Text(a), SqlValue::Text(b)) => a.cmp(b),
            (SqlValue::Blob(a), SqlValue::Blob(b)) => a.cmp(b),

            // SQLite type sorting affinity: NULL < Numbers < Text < Blob
            (SqlValue::Integer(_) | SqlValue::Real(_), SqlValue::Text(_)) => {
                std::cmp::Ordering::Less
            }
            (SqlValue::Text(_), SqlValue::Integer(_) | SqlValue::Real(_)) => {
                std::cmp::Ordering::Greater
            }
            (SqlValue::Blob(_), _) => std::cmp::Ordering::Greater,
            (_, SqlValue::Blob(_)) => std::cmp::Ordering::Less,
        }
    }
}

fn hex_preview(bytes: &[u8], max_bytes: usize) -> String {
    let take = bytes.len().min(max_bytes);
    let mut s = String::with_capacity(take * 2 + if bytes.len() > max_bytes { 3 } else { 0 });
    for b in &bytes[..take] {
        s.push_str(&format!("{:02x}", b));
    }
    if bytes.len() > max_bytes {
        s.push_str("...");
    }
    s
}

// ===========================================================================
// SQLite Database Header
// ===========================================================================

#[derive(Debug, Clone)]
pub struct SqliteHeader {
    pub page_size: u32,
    pub write_version: u8,
    pub read_version: u8,
    pub reserved_space: u8,
    pub max_payload_fraction: u8,
    pub min_payload_fraction: u8,
    pub leaf_payload_fraction: u8,
    pub file_change_counter: u32,
    pub db_size_in_pages: u32,
    pub first_freelist_trunk_page: u32,
    pub total_freelist_pages: u32,
    pub schema_cookie: u32,
    pub schema_format: u32,
    pub default_page_cache_size: u32,
    pub largest_root_page: u32,
    pub text_encoding: u32,
    pub user_version: u32,
    pub incremental_vacuum_mode: u32,
    pub application_id: u32,
    pub version_valid_for: u32,
    pub sqlite_version: u32,
}

impl SqliteHeader {
    pub fn parse(header: &[u8]) -> anyhow::Result<Self> {
        if header.len() < 100 {
            anyhow::bail!("File is too small to be a valid SQLite database (less than 100 bytes)");
        }
        if &header[0..16] != b"SQLite format 3\0" {
            anyhow::bail!("Invalid SQLite magic header: not a SQLite 3 database file");
        }

        let raw_page_size = u16::from_be_bytes([header[16], header[17]]);
        let page_size = if raw_page_size == 1 {
            65536
        } else if raw_page_size.is_power_of_two() && raw_page_size >= 512 {
            raw_page_size as u32
        } else {
            anyhow::bail!("Invalid SQLite page size: {}", raw_page_size);
        };

        Ok(Self {
            page_size,
            write_version: header[18],
            read_version: header[19],
            reserved_space: header[20],
            max_payload_fraction: header[21],
            min_payload_fraction: header[22],
            leaf_payload_fraction: header[23],
            file_change_counter: u32::from_be_bytes([
                header[24], header[25], header[26], header[27],
            ]),
            db_size_in_pages: u32::from_be_bytes([header[28], header[29], header[30], header[31]]),
            first_freelist_trunk_page: u32::from_be_bytes([
                header[32], header[33], header[34], header[35],
            ]),
            total_freelist_pages: u32::from_be_bytes([
                header[36], header[37], header[38], header[39],
            ]),
            schema_cookie: u32::from_be_bytes([header[40], header[41], header[42], header[43]]),
            schema_format: u32::from_be_bytes([header[44], header[45], header[46], header[47]]),
            default_page_cache_size: u32::from_be_bytes([
                header[48], header[49], header[50], header[51],
            ]),
            largest_root_page: u32::from_be_bytes([header[52], header[53], header[54], header[55]]),
            text_encoding: u32::from_be_bytes([header[56], header[57], header[58], header[59]]),
            user_version: u32::from_be_bytes([header[60], header[61], header[62], header[63]]),
            incremental_vacuum_mode: u32::from_be_bytes([
                header[64], header[65], header[66], header[67],
            ]),
            application_id: u32::from_be_bytes([header[68], header[69], header[70], header[71]]),
            version_valid_for: u32::from_be_bytes([header[92], header[93], header[94], header[95]]),
            sqlite_version: u32::from_be_bytes([header[96], header[97], header[98], header[99]]),
        })
    }

    pub fn encoding_str(&self) -> &'static str {
        match self.text_encoding {
            1 => "UTF-8",
            2 => "UTF-16le",
            3 => "UTF-16be",
            _ => "Unknown",
        }
    }

    pub fn sqlite_version_str(&self) -> String {
        if self.sqlite_version == 0 {
            "Unknown".to_string()
        } else {
            let major = self.sqlite_version / 1_000_000;
            let minor = (self.sqlite_version % 1_000_000) / 1_000;
            let patch = self.sqlite_version % 1_000;
            format!("{}.{}.{}", major, minor, patch)
        }
    }
}

// ===========================================================================
// SQLite Schema & Metadata
// ===========================================================================

#[derive(Debug, Clone)]
pub struct MasterEntry {
    pub object_type: String, // "table", "index", "view", "trigger"
    pub name: String,
    pub tbl_name: String,
    pub rootpage: u32,
    pub sql: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub cid: usize,
    pub name: String,
    pub data_type: String,
    pub notnull: bool,
    pub dflt_value: Option<String>,
    pub pk: bool,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub sql: Option<String>,
    pub rootpage: u32,
    pub is_view: bool,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub rowid: Option<i64>,
    pub values: Vec<SqlValue>,
}

// ===========================================================================
// Binary SQLite Database Parser
// ===========================================================================

pub struct SqliteReader<'a> {
    data: &'a [u8],
    pub header: SqliteHeader,
    pub usable_page_size: u32,
}

impl<'a> SqliteReader<'a> {
    pub fn new(data: &'a [u8]) -> anyhow::Result<Self> {
        let header = SqliteHeader::parse(data)?;
        let usable_page_size = header
            .page_size
            .saturating_sub(header.reserved_space as u32);
        if usable_page_size < 480 {
            anyhow::bail!("Usable page size is too small: {}", usable_page_size);
        }
        Ok(Self {
            data,
            header,
            usable_page_size,
        })
    }

    pub fn get_page(&self, page_num: u32) -> anyhow::Result<&'a [u8]> {
        if page_num == 0 {
            anyhow::bail!("Invalid page number 0 (pages are 1-indexed)");
        }
        let page_size = self.header.page_size as usize;
        let start = (page_num as usize - 1) * page_size;
        let end = start + page_size;
        if start >= self.data.len() {
            anyhow::bail!(
                "Page {} is out of file bounds (file size: {} bytes, page offset: {})",
                page_num,
                self.data.len(),
                start
            );
        }
        let actual_end = end.min(self.data.len());
        Ok(&self.data[start..actual_end])
    }

    pub fn read_varint(data: &[u8], offset: &mut usize) -> anyhow::Result<u64> {
        let mut result = 0u64;
        let mut bytes_read = 0;
        while bytes_read < 8 {
            if *offset >= data.len() {
                anyhow::bail!(
                    "Unexpected end of data while reading varint at offset {}",
                    *offset
                );
            }
            let b = data[*offset];
            *offset += 1;
            bytes_read += 1;
            result = (result << 7) | ((b & 0x7F) as u64);
            if (b & 0x80) == 0 {
                return Ok(result);
            }
        }
        if *offset >= data.len() {
            anyhow::bail!(
                "Unexpected end of data for 9th byte of varint at offset {}",
                *offset
            );
        }
        let b = data[*offset];
        *offset += 1;
        result = (result << 8) | (b as u64);
        Ok(result)
    }

    pub fn parse_record(&self, payload: &[u8]) -> anyhow::Result<Vec<SqlValue>> {
        if payload.is_empty() {
            return Ok(Vec::new());
        }

        let mut offset = 0;
        let header_len = Self::read_varint(payload, &mut offset)? as usize;
        let mut serial_types = Vec::new();

        while offset < header_len {
            let st = Self::read_varint(payload, &mut offset)?;
            serial_types.push(st);
        }

        let mut values = Vec::with_capacity(serial_types.len());
        let mut body_offset = header_len;

        for st in serial_types {
            match st {
                0 => values.push(SqlValue::Null),
                1 => {
                    if body_offset >= payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let val = payload[body_offset] as i8 as i64;
                        body_offset += 1;
                        values.push(SqlValue::Integer(val));
                    }
                }
                2 => {
                    if body_offset + 2 > payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let val =
                            i16::from_be_bytes([payload[body_offset], payload[body_offset + 1]])
                                as i64;
                        body_offset += 2;
                        values.push(SqlValue::Integer(val));
                    }
                }
                3 => {
                    if body_offset + 3 > payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let b0 = payload[body_offset] as i64;
                        let b1 = payload[body_offset + 1] as i64;
                        let b2 = payload[body_offset + 2] as i64;
                        let mut val = (b0 << 16) | (b1 << 8) | b2;
                        if (val & 0x800000) != 0 {
                            val |= !0xFFFFFF;
                        }
                        body_offset += 3;
                        values.push(SqlValue::Integer(val));
                    }
                }
                4 => {
                    if body_offset + 4 > payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let val = i32::from_be_bytes([
                            payload[body_offset],
                            payload[body_offset + 1],
                            payload[body_offset + 2],
                            payload[body_offset + 3],
                        ]) as i64;
                        body_offset += 4;
                        values.push(SqlValue::Integer(val));
                    }
                }
                5 => {
                    if body_offset + 6 > payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let mut buf = [0u8; 8];
                        buf[2..8].copy_from_slice(&payload[body_offset..body_offset + 6]);
                        let val = i64::from_be_bytes(buf);
                        let val = (val << 16) >> 16;
                        body_offset += 6;
                        values.push(SqlValue::Integer(val));
                    }
                }
                6 => {
                    if body_offset + 8 > payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let val = i64::from_be_bytes([
                            payload[body_offset],
                            payload[body_offset + 1],
                            payload[body_offset + 2],
                            payload[body_offset + 3],
                            payload[body_offset + 4],
                            payload[body_offset + 5],
                            payload[body_offset + 6],
                            payload[body_offset + 7],
                        ]);
                        body_offset += 8;
                        values.push(SqlValue::Integer(val));
                    }
                }
                7 => {
                    if body_offset + 8 > payload.len() {
                        values.push(SqlValue::Null);
                    } else {
                        let val = f64::from_be_bytes([
                            payload[body_offset],
                            payload[body_offset + 1],
                            payload[body_offset + 2],
                            payload[body_offset + 3],
                            payload[body_offset + 4],
                            payload[body_offset + 5],
                            payload[body_offset + 6],
                            payload[body_offset + 7],
                        ]);
                        body_offset += 8;
                        values.push(SqlValue::Real(val));
                    }
                }
                8 => values.push(SqlValue::Integer(0)),
                9 => values.push(SqlValue::Integer(1)),
                10 | 11 => values.push(SqlValue::Null), // internal/reserved
                st if st >= 12 && st % 2 == 0 => {
                    let len = ((st - 12) / 2) as usize;
                    if body_offset + len > payload.len() {
                        values.push(SqlValue::Blob(payload[body_offset..].to_vec()));
                        body_offset = payload.len();
                    } else {
                        let blob = payload[body_offset..body_offset + len].to_vec();
                        body_offset += len;
                        values.push(SqlValue::Blob(blob));
                    }
                }
                st if st >= 13 && st % 2 == 1 => {
                    let len = ((st - 13) / 2) as usize;
                    if body_offset + len > payload.len() {
                        let text = String::from_utf8_lossy(&payload[body_offset..]).to_string();
                        body_offset = payload.len();
                        values.push(SqlValue::Text(text));
                    } else {
                        let text =
                            String::from_utf8_lossy(&payload[body_offset..body_offset + len])
                                .to_string();
                        body_offset += len;
                        values.push(SqlValue::Text(text));
                    }
                }
                _ => values.push(SqlValue::Null),
            }
        }

        Ok(values)
    }

    fn read_overflow_chain(
        &self,
        mut overflow_page_num: u32,
        needed_bytes: usize,
    ) -> anyhow::Result<Vec<u8>> {
        let mut overflow_bytes = Vec::with_capacity(needed_bytes);
        let u = self.usable_page_size as usize;
        let mut visited = std::collections::HashSet::new();

        while overflow_page_num != 0 && overflow_bytes.len() < needed_bytes {
            if !visited.insert(overflow_page_num) {
                anyhow::bail!(
                    "Detected cycle in overflow page chain at page {}",
                    overflow_page_num
                );
            }
            let page_data = self.get_page(overflow_page_num)?;
            if page_data.len() < 4 {
                break;
            }
            let next_page =
                u32::from_be_bytes([page_data[0], page_data[1], page_data[2], page_data[3]]);
            let payload_in_page = &page_data[4..page_data.len().min(u)];
            let remaining = needed_bytes - overflow_bytes.len();
            let take = payload_in_page.len().min(remaining);
            overflow_bytes.extend_from_slice(&payload_in_page[..take]);
            overflow_page_num = next_page;
        }

        Ok(overflow_bytes)
    }

    fn read_table_leaf_cell(
        &self,
        page: &[u8],
        cell_offset: usize,
    ) -> anyhow::Result<(i64, Vec<u8>)> {
        if cell_offset >= page.len() {
            anyhow::bail!(
                "Cell offset {} beyond page size {}",
                cell_offset,
                page.len()
            );
        }
        let mut offset = cell_offset;
        let payload_size = Self::read_varint(page, &mut offset)? as usize;
        let row_id = Self::read_varint(page, &mut offset)? as i64;

        let u = self.usable_page_size as usize;
        let x = u.saturating_sub(35);
        let m = (((u.saturating_sub(12)) * 32) / 255).saturating_sub(23);

        let local_payload_size = if payload_size <= x {
            payload_size
        } else {
            let k = m + ((payload_size.saturating_sub(m)) % (u.saturating_sub(4)));
            if k <= x {
                k
            } else {
                m
            }
        };

        let available_in_cell = page.len().saturating_sub(offset);
        let actual_local_size = local_payload_size.min(available_in_cell);
        let mut payload = page[offset..offset + actual_local_size].to_vec();
        offset += actual_local_size;

        if payload_size > local_payload_size {
            if offset + 4 <= page.len() {
                let overflow_page_num = u32::from_be_bytes([
                    page[offset],
                    page[offset + 1],
                    page[offset + 2],
                    page[offset + 3],
                ]);
                let remaining_needed = payload_size.saturating_sub(payload.len());
                let overflow_data =
                    self.read_overflow_chain(overflow_page_num, remaining_needed)?;
                payload.extend_from_slice(&overflow_data);
            }
        }

        Ok((row_id, payload))
    }

    pub fn read_master_entries(&self) -> anyhow::Result<Vec<MasterEntry>> {
        let rows = self.read_table_rows(1)?;
        let mut entries = Vec::new();

        for row in rows {
            if row.values.len() >= 5 {
                let object_type = match &row.values[0] {
                    SqlValue::Text(s) => s.clone(),
                    _ => continue,
                };
                let name = match &row.values[1] {
                    SqlValue::Text(s) => s.clone(),
                    _ => continue,
                };
                let tbl_name = match &row.values[2] {
                    SqlValue::Text(s) => s.clone(),
                    _ => continue,
                };
                let rootpage = match &row.values[3] {
                    SqlValue::Integer(i) => *i as u32,
                    _ => 0,
                };
                let sql = match &row.values[4] {
                    SqlValue::Text(s) => Some(s.clone()),
                    SqlValue::Null => None,
                    _ => None,
                };

                entries.push(MasterEntry {
                    object_type,
                    name,
                    tbl_name,
                    rootpage,
                    sql,
                });
            }
        }

        Ok(entries)
    }

    pub fn read_table_rows(&self, root_page: u32) -> anyhow::Result<Vec<Row>> {
        let mut rows = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.traverse_table_btree(root_page, &mut rows, &mut visited)?;
        Ok(rows)
    }

    fn traverse_table_btree(
        &self,
        page_num: u32,
        rows: &mut Vec<Row>,
        visited: &mut std::collections::HashSet<u32>,
    ) -> anyhow::Result<()> {
        if page_num == 0 {
            return Ok(());
        }
        if !visited.insert(page_num) {
            anyhow::bail!("Detected cycle in B-Tree traversal at page {}", page_num);
        }

        let page = self.get_page(page_num)?;
        let page_header_offset = if page_num == 1 { 100 } else { 0 };

        if page.len() < page_header_offset + 8 {
            return Ok(());
        }

        let page_type = page[page_header_offset];
        let num_cells =
            u16::from_be_bytes([page[page_header_offset + 3], page[page_header_offset + 4]])
                as usize;

        match page_type {
            0x0D => {
                // Leaf Table Page
                let cell_pointer_offset = page_header_offset + 8;
                for i in 0..num_cells {
                    let ptr_pos = cell_pointer_offset + i * 2;
                    if ptr_pos + 2 > page.len() {
                        break;
                    }
                    let cell_offset =
                        u16::from_be_bytes([page[ptr_pos], page[ptr_pos + 1]]) as usize;
                    if let Ok((rowid, payload)) = self.read_table_leaf_cell(page, cell_offset) {
                        if let Ok(values) = self.parse_record(&payload) {
                            rows.push(Row {
                                rowid: Some(rowid),
                                values,
                            });
                        }
                    }
                }
            }
            0x05 => {
                // Interior Table Page
                if page.len() < page_header_offset + 12 {
                    return Ok(());
                }
                let rightmost_pointer = u32::from_be_bytes([
                    page[page_header_offset + 8],
                    page[page_header_offset + 9],
                    page[page_header_offset + 10],
                    page[page_header_offset + 11],
                ]);

                let cell_pointer_offset = page_header_offset + 12;
                let mut child_pages = Vec::with_capacity(num_cells + 1);

                for i in 0..num_cells {
                    let ptr_pos = cell_pointer_offset + i * 2;
                    if ptr_pos + 2 > page.len() {
                        break;
                    }
                    let cell_offset =
                        u16::from_be_bytes([page[ptr_pos], page[ptr_pos + 1]]) as usize;
                    if cell_offset + 4 <= page.len() {
                        let left_child = u32::from_be_bytes([
                            page[cell_offset],
                            page[cell_offset + 1],
                            page[cell_offset + 2],
                            page[cell_offset + 3],
                        ]);
                        child_pages.push(left_child);
                    }
                }
                child_pages.push(rightmost_pointer);

                for child in child_pages {
                    self.traverse_table_btree(child, rows, visited)?;
                }
            }
            _ => {
                // Non-table page or unsupported
            }
        }

        Ok(())
    }

    pub fn get_table_schema(&self, table_name: &str) -> anyhow::Result<Option<TableSchema>> {
        let entries = self.read_master_entries()?;
        for entry in entries {
            if entry.name.eq_ignore_ascii_case(table_name)
                && (entry.object_type == "table" || entry.object_type == "view")
            {
                let is_view = entry.object_type == "view";
                let columns = if let Some(sql) = &entry.sql {
                    parse_columns_from_ddl(sql)
                } else {
                    Vec::new()
                };

                return Ok(Some(TableSchema {
                    name: entry.name,
                    columns,
                    sql: entry.sql,
                    rootpage: entry.rootpage,
                    is_view,
                }));
            }
        }
        Ok(None)
    }
}

// ===========================================================================
// SQL DDL Parser
// ===========================================================================

pub fn parse_columns_from_ddl(sql: &str) -> Vec<ColumnDef> {
    let mut columns = Vec::new();
    let trimmed = sql.trim();

    // Find the opening parenthesis of column definitions
    let open_paren = match trimmed.find('(') {
        Some(pos) => pos,
        None => return columns,
    };
    let close_paren = match trimmed.rfind(')') {
        Some(pos) => pos,
        None => return columns,
    };

    if close_paren <= open_paren {
        return columns;
    }

    let inner = &trimmed[open_paren + 1..close_paren];
    let parts = split_sql_tokens(inner, ',');

    let mut cid = 0;
    for part in parts {
        let col_sql = part.trim();
        if col_sql.is_empty() {
            continue;
        }

        let upper = col_sql.to_uppercase();
        // Check if this is a table-level constraint
        if upper.starts_with("PRIMARY KEY")
            || upper.starts_with("FOREIGN KEY")
            || upper.starts_with("UNIQUE")
            || upper.starts_with("CHECK")
            || upper.starts_with("CONSTRAINT")
        {
            // Table level constraint, not a column
            continue;
        }

        let words: Vec<&str> = col_sql.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }

        let name = clean_identifier(words[0]);
        let data_type = if words.len() > 1 {
            let mut type_str = words[1].to_string();
            // Collect compound types like VARCHAR(255) or DOUBLE PRECISION
            for w in &words[2..] {
                let w_upper = w.to_uppercase();
                if w_upper.starts_with("PRIMARY")
                    || w_upper.starts_with("NOT")
                    || w_upper.starts_with("NULL")
                    || w_upper.starts_with("DEFAULT")
                    || w_upper.starts_with("CHECK")
                    || w_upper.starts_with("UNIQUE")
                    || w_upper.starts_with("REFERENCES")
                    || w_upper.starts_with("AUTOINCREMENT")
                {
                    break;
                }
                type_str.push(' ');
                type_str.push_str(w);
            }
            type_str
        } else {
            "ANY".to_string()
        };

        let is_pk = upper.contains("PRIMARY KEY");
        let is_not_null = upper.contains("NOT NULL") || is_pk;

        let dflt_value = if let Some(def_pos) = upper.find("DEFAULT") {
            let rest = col_sql[def_pos + 7..].trim();
            let def_val = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('\'')
                .to_string();
            Some(def_val)
        } else {
            None
        };

        columns.push(ColumnDef {
            cid,
            name,
            data_type,
            notnull: is_not_null,
            dflt_value,
            pk: is_pk,
        });
        cid += 1;
    }

    columns
}

fn split_sql_tokens(input: &str, delimiter: char) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut paren_depth: usize = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            current.push(c);
        } else if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            current.push(c);
        } else if !in_single_quote && !in_double_quote {
            if c == '(' {
                paren_depth += 1;
                current.push(c);
            } else if c == ')' {
                paren_depth = paren_depth.saturating_sub(1);
                current.push(c);
            } else if c == delimiter && paren_depth == 0 {
                tokens.push(current.trim().to_string());
                current.clear();
            } else {
                current.push(c);
            }
        } else {
            current.push(c);
        }
        i += 1;
    }

    if !current.trim().is_empty() {
        tokens.push(current.trim().to_string());
    }

    tokens
}

fn clean_identifier(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('`') && s.ends_with('`'))
        || (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('[') && s.ends_with(']'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ===========================================================================
// SQL Query Parser & Evaluator
// ===========================================================================

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlValue>>,
}

#[derive(Debug, Clone)]
enum Expr {
    Column(String),
    Literal(SqlValue),
    BinaryOp {
        left: Box<Expr>,
        op: String,
        right: Box<Expr>,
    },
    UnaryOp {
        op: String,
        expr: Box<Expr>,
    },
    IsNull(Box<Expr>),
    IsNotNull(Box<Expr>),
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
    },
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negated: bool,
    },
    Like {
        expr: Box<Expr>,
        pattern: Box<Expr>,
        negated: bool,
    },
    FunctionCall {
        name: String,
        arg: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone)]
struct SelectItem {
    expr: Expr,
    alias: Option<String>,
}

#[derive(Debug, Clone)]
struct OrderByItem {
    expr: Expr,
    descending: bool,
}

#[derive(Debug, Clone)]
struct SelectQuery {
    distinct: bool,
    items: Vec<SelectItem>,
    table: Option<String>,
    where_clause: Option<Expr>,
    order_by: Vec<OrderByItem>,
    limit: Option<usize>,
    offset: Option<usize>,
}

pub struct SqlEngine<'a> {
    reader: &'a SqliteReader<'a>,
}

impl<'a> SqlEngine<'a> {
    pub fn new(reader: &'a SqliteReader<'a>) -> Self {
        Self { reader }
    }

    pub fn execute(&self, sql_query: &str) -> anyhow::Result<QueryResult> {
        validate_read_only_query(sql_query)?;

        let stripped = strip_sql_comments(sql_query);
        let trimmed = stripped.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            anyhow::bail!("Empty SQL query");
        }

        // Check for PRAGMA or dot commands
        let upper = trimmed.to_uppercase();
        if upper.starts_with(".TABLES") || upper == "SHOW TABLES" || upper == "SHOW SCHEMAS" {
            return self.execute_show_tables();
        }
        if upper.starts_with(".SCHEMA") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let table = parts.get(1).map(|s| clean_identifier(s));
            return self.execute_show_schema(table.as_deref());
        }
        if upper.starts_with("DESCRIBE ") || upper.starts_with("DESC ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if let Some(table) = parts.get(1) {
                let cleaned = clean_identifier(table);
                return self.execute_pragma_table_info(&cleaned);
            }
        }
        if upper.starts_with("PRAGMA TABLE_INFO(") && upper.ends_with(')') {
            let inside = &trimmed[18..trimmed.len() - 1];
            let table = clean_identifier(inside);
            return self.execute_pragma_table_info(&table);
        }
        if upper.starts_with("PRAGMA TABLE_INFO") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if let Some(table) = parts.get(2) {
                let cleaned = clean_identifier(table);
                return self.execute_pragma_table_info(&cleaned);
            }
        }
        if upper.starts_with("PRAGMA DATABASE_LIST") {
            return Ok(QueryResult {
                columns: vec!["seq".to_string(), "name".to_string(), "file".to_string()],
                rows: vec![vec![
                    SqlValue::Integer(0),
                    SqlValue::Text("main".to_string()),
                    SqlValue::Text("".to_string()),
                ]],
            });
        }

        if upper.starts_with("SELECT") {
            let query = parse_select_query(trimmed)?;
            self.execute_select(query)
        } else {
            anyhow::bail!("Only read-only SELECT, PRAGMA table_info, .tables, and .schema queries are supported by the SQLite inspection tool");
        }
    }

    fn execute_show_tables(&self) -> anyhow::Result<QueryResult> {
        let entries = self.reader.read_master_entries()?;
        let mut rows = Vec::new();
        for entry in entries {
            if entry.object_type == "table" && !entry.name.starts_with("sqlite_") {
                rows.push(vec![SqlValue::Text(entry.name)]);
            }
        }
        Ok(QueryResult {
            columns: vec!["name".to_string()],
            rows,
        })
    }

    fn execute_show_schema(&self, table: Option<&str>) -> anyhow::Result<QueryResult> {
        let entries = self.reader.read_master_entries()?;
        let mut rows = Vec::new();
        for entry in entries {
            if let Some(target) = table {
                if !entry.name.eq_ignore_ascii_case(target) {
                    continue;
                }
            }
            if let Some(sql) = &entry.sql {
                rows.push(vec![
                    SqlValue::Text(entry.object_type),
                    SqlValue::Text(entry.name),
                    SqlValue::Text(sql.clone()),
                ]);
            }
        }
        Ok(QueryResult {
            columns: vec!["type".to_string(), "name".to_string(), "sql".to_string()],
            rows,
        })
    }

    fn execute_pragma_table_info(&self, table_name: &str) -> anyhow::Result<QueryResult> {
        let schema = self
            .reader
            .get_table_schema(table_name)?
            .ok_or_else(|| anyhow::anyhow!("Table '{}' not found", table_name))?;

        let mut rows = Vec::new();
        for col in schema.columns {
            rows.push(vec![
                SqlValue::Integer(col.cid as i64),
                SqlValue::Text(col.name),
                SqlValue::Text(col.data_type),
                SqlValue::Integer(if col.notnull { 1 } else { 0 }),
                match col.dflt_value {
                    Some(v) => SqlValue::Text(v),
                    None => SqlValue::Null,
                },
                SqlValue::Integer(if col.pk { 1 } else { 0 }),
            ]);
        }

        Ok(QueryResult {
            columns: vec![
                "cid".to_string(),
                "name".to_string(),
                "type".to_string(),
                "notnull".to_string(),
                "dflt_value".to_string(),
                "pk".to_string(),
            ],
            rows,
        })
    }

    fn execute_select(&self, query: SelectQuery) -> anyhow::Result<QueryResult> {
        // Handle queries with no FROM table (e.g., SELECT 1 + 1, SELECT 'hello')
        let table_name = match &query.table {
            Some(t) => t.as_str(),
            None => {
                let mut out_cols = Vec::new();
                let mut out_row = Vec::new();
                for (i, item) in query.items.iter().enumerate() {
                    let col_name = item
                        .alias
                        .clone()
                        .unwrap_or_else(|| format!("col_{}", i + 1));
                    out_cols.push(col_name);
                    let val = eval_expr(&item.expr, &[], &HashMap::new(), None)?;
                    out_row.push(val);
                }
                return Ok(QueryResult {
                    columns: out_cols,
                    rows: vec![out_row],
                });
            }
        };

        // Check if querying sqlite_master or sqlite_schema
        if table_name.eq_ignore_ascii_case("sqlite_master")
            || table_name.eq_ignore_ascii_case("sqlite_schema")
        {
            return self.execute_select_sqlite_master(query);
        }

        // Get table schema
        let schema = self.reader.get_table_schema(table_name)?.ok_or_else(|| {
            anyhow::anyhow!("Table or view '{}' not found in database", table_name)
        })?;

        // Read raw rows
        let raw_rows = self.reader.read_table_rows(schema.rootpage)?;

        // Map column names to indexes
        let mut col_map = HashMap::new();
        for col in &schema.columns {
            col_map.insert(col.name.to_lowercase(), col.cid);
        }

        // Find primary key integer alias column (e.g. INTEGER PRIMARY KEY)
        let ipk_cid = schema
            .columns
            .iter()
            .position(|c| c.pk && c.data_type.eq_ignore_ascii_case("INTEGER"));

        // Evaluate WHERE filter
        let mut filtered_rows = Vec::new();
        for row in raw_rows {
            // Adjust row values if column is INTEGER PRIMARY KEY and value in payload is NULL or missing
            let mut values = row.values.clone();
            if let Some(pk_idx) = ipk_cid {
                if let Some(rowid) = row.rowid {
                    while values.len() <= pk_idx {
                        values.push(SqlValue::Null);
                    }
                    if values[pk_idx].is_null() {
                        values[pk_idx] = SqlValue::Integer(rowid);
                    }
                }
            }

            if let Some(where_expr) = &query.where_clause {
                let matched = eval_bool_expr(where_expr, &values, &col_map, row.rowid)?;
                if !matched {
                    continue;
                }
            }
            filtered_rows.push((row.rowid, values));
        }

        // Check if query is an aggregate query (COUNT(*), SUM, etc.)
        let is_aggregate = query.items.iter().any(|item| is_aggregate_expr(&item.expr));

        if is_aggregate {
            return self.execute_aggregate_select(query, filtered_rows, &col_map);
        }

        // Sort rows if ORDER BY present
        if !query.order_by.is_empty() {
            filtered_rows.sort_by(|a, b| {
                for order_item in &query.order_by {
                    let val_a =
                        eval_expr(&order_item.expr, &a.1, &col_map, a.0).unwrap_or(SqlValue::Null);
                    let val_b =
                        eval_expr(&order_item.expr, &b.1, &col_map, b.0).unwrap_or(SqlValue::Null);
                    let mut ord = val_a.compare(&val_b);
                    if order_item.descending {
                        ord = ord.reverse();
                    }
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Apply OFFSET and LIMIT
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(usize::MAX);
        let page_rows = filtered_rows.into_iter().skip(offset).take(limit);

        // Project columns
        let mut out_cols = Vec::new();
        let mut projected_rows = Vec::new();

        // Build output column names
        for (i, item) in query.items.iter().enumerate() {
            if let Expr::Column(c) = &item.expr {
                if c == "*" {
                    for col in &schema.columns {
                        out_cols.push(col.name.clone());
                    }
                    continue;
                }
            }
            let name = if let Some(a) = &item.alias {
                a.clone()
            } else {
                expr_to_column_name(&item.expr, i)
            };
            out_cols.push(name);
        }

        // Build projected rows
        for (rowid, values) in page_rows {
            let mut row_vals = Vec::new();
            for item in &query.items {
                if let Expr::Column(c) = &item.expr {
                    if c == "*" {
                        for col in &schema.columns {
                            let val = values.get(col.cid).cloned().unwrap_or(SqlValue::Null);
                            row_vals.push(val);
                        }
                        continue;
                    }
                }
                let val = eval_expr(&item.expr, &values, &col_map, rowid)?;
                row_vals.push(val);
            }
            projected_rows.push(row_vals);
        }

        // Apply DISTINCT if requested
        if query.distinct {
            let mut unique_rows = Vec::new();
            for r in projected_rows {
                if !unique_rows.contains(&r) {
                    unique_rows.push(r);
                }
            }
            projected_rows = unique_rows;
        }

        Ok(QueryResult {
            columns: out_cols,
            rows: projected_rows,
        })
    }

    fn execute_select_sqlite_master(&self, query: SelectQuery) -> anyhow::Result<QueryResult> {
        let entries = self.reader.read_master_entries()?;
        let mut rows = Vec::new();
        for entry in entries {
            let row = vec![
                SqlValue::Text(entry.object_type),
                SqlValue::Text(entry.name),
                SqlValue::Text(entry.tbl_name),
                SqlValue::Integer(entry.rootpage as i64),
                match entry.sql {
                    Some(s) => SqlValue::Text(s),
                    None => SqlValue::Null,
                },
            ];
            rows.push(row);
        }

        let col_map = HashMap::from([
            ("type".to_string(), 0),
            ("name".to_string(), 1),
            ("tbl_name".to_string(), 2),
            ("rootpage".to_string(), 3),
            ("sql".to_string(), 4),
        ]);

        let mut filtered_rows = Vec::new();
        for r in rows {
            if let Some(where_expr) = &query.where_clause {
                if !eval_bool_expr(where_expr, &r, &col_map, None)? {
                    continue;
                }
            }
            filtered_rows.push(r);
        }

        let mut out_cols = Vec::new();
        let mut projected_rows = Vec::new();

        let master_cols = ["type", "name", "tbl_name", "rootpage", "sql"];

        for (i, item) in query.items.iter().enumerate() {
            if let Expr::Column(c) = &item.expr {
                if c == "*" {
                    for mc in master_cols {
                        out_cols.push(mc.to_string());
                    }
                    continue;
                }
            }
            let name = item
                .alias
                .clone()
                .unwrap_or_else(|| expr_to_column_name(&item.expr, i));
            out_cols.push(name);
        }

        for r in filtered_rows {
            let mut row_vals = Vec::new();
            for item in &query.items {
                if let Expr::Column(c) = &item.expr {
                    if c == "*" {
                        for (idx, _) in master_cols.iter().enumerate() {
                            row_vals.push(r.get(idx).cloned().unwrap_or(SqlValue::Null));
                        }
                        continue;
                    }
                }
                let val = eval_expr(&item.expr, &r, &col_map, None)?;
                row_vals.push(val);
            }
            projected_rows.push(row_vals);
        }

        Ok(QueryResult {
            columns: out_cols,
            rows: projected_rows,
        })
    }

    fn execute_aggregate_select(
        &self,
        query: SelectQuery,
        rows: Vec<(Option<i64>, Vec<SqlValue>)>,
        col_map: &HashMap<String, usize>,
    ) -> anyhow::Result<QueryResult> {
        let mut out_cols = Vec::new();
        let mut out_values = Vec::new();

        for (i, item) in query.items.iter().enumerate() {
            let col_name = item
                .alias
                .clone()
                .unwrap_or_else(|| expr_to_column_name(&item.expr, i));
            out_cols.push(col_name);

            let val = match &item.expr {
                Expr::FunctionCall { name, arg } => {
                    let fn_upper = name.to_uppercase();
                    match fn_upper.as_str() {
                        "COUNT" => match arg {
                            None => SqlValue::Integer(rows.len() as i64),
                            Some(inner) => {
                                if let Expr::Column(c) = &**inner {
                                    if c == "*" {
                                        SqlValue::Integer(rows.len() as i64)
                                    } else {
                                        let count = rows
                                            .iter()
                                            .filter(|(rowid, vals)| {
                                                eval_expr(inner, vals, col_map, *rowid)
                                                    .map(|v| !v.is_null())
                                                    .unwrap_or(false)
                                            })
                                            .count();
                                        SqlValue::Integer(count as i64)
                                    }
                                } else {
                                    SqlValue::Integer(rows.len() as i64)
                                }
                            }
                        },
                        "SUM" => {
                            if let Some(inner) = arg {
                                let mut sum_f = 0.0;
                                let mut sum_i = 0i64;
                                let mut is_float = false;
                                for (rowid, vals) in &rows {
                                    if let Ok(v) = eval_expr(inner, vals, col_map, *rowid) {
                                        match v {
                                            SqlValue::Integer(i) => {
                                                sum_i += i;
                                                sum_f += i as f64;
                                            }
                                            SqlValue::Real(f) => {
                                                is_float = true;
                                                sum_f += f;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                if is_float {
                                    SqlValue::Real(sum_f)
                                } else {
                                    SqlValue::Integer(sum_i)
                                }
                            } else {
                                SqlValue::Null
                            }
                        }
                        "AVG" => {
                            if let Some(inner) = arg {
                                let mut sum = 0.0;
                                let mut count = 0;
                                for (rowid, vals) in &rows {
                                    if let Ok(v) = eval_expr(inner, vals, col_map, *rowid) {
                                        if let Some(num) = v.as_f64() {
                                            sum += num;
                                            count += 1;
                                        }
                                    }
                                }
                                if count > 0 {
                                    SqlValue::Real(sum / count as f64)
                                } else {
                                    SqlValue::Null
                                }
                            } else {
                                SqlValue::Null
                            }
                        }
                        "MIN" => {
                            if let Some(inner) = arg {
                                let mut min_val: Option<SqlValue> = None;
                                for (rowid, vals) in &rows {
                                    if let Ok(v) = eval_expr(inner, vals, col_map, *rowid) {
                                        if !v.is_null() {
                                            min_val = match min_val {
                                                None => Some(v),
                                                Some(curr) => {
                                                    if v.compare(&curr) == std::cmp::Ordering::Less
                                                    {
                                                        Some(v)
                                                    } else {
                                                        Some(curr)
                                                    }
                                                }
                                            };
                                        }
                                    }
                                }
                                min_val.unwrap_or(SqlValue::Null)
                            } else {
                                SqlValue::Null
                            }
                        }
                        "MAX" => {
                            if let Some(inner) = arg {
                                let mut max_val: Option<SqlValue> = None;
                                for (rowid, vals) in &rows {
                                    if let Ok(v) = eval_expr(inner, vals, col_map, *rowid) {
                                        if !v.is_null() {
                                            max_val = match max_val {
                                                None => Some(v),
                                                Some(curr) => {
                                                    if v.compare(&curr)
                                                        == std::cmp::Ordering::Greater
                                                    {
                                                        Some(v)
                                                    } else {
                                                        Some(curr)
                                                    }
                                                }
                                            };
                                        }
                                    }
                                }
                                max_val.unwrap_or(SqlValue::Null)
                            } else {
                                SqlValue::Null
                            }
                        }
                        _ => SqlValue::Null,
                    }
                }
                _ => {
                    // Non-aggregate column in aggregate query: take value from first row if available
                    if let Some((rowid, vals)) = rows.first() {
                        eval_expr(&item.expr, vals, col_map, *rowid).unwrap_or(SqlValue::Null)
                    } else {
                        SqlValue::Null
                    }
                }
            };
            out_values.push(val);
        }

        Ok(QueryResult {
            columns: out_cols,
            rows: vec![out_values],
        })
    }
}

fn is_aggregate_expr(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, .. } => {
            let u = name.to_uppercase();
            matches!(u.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
        }
        Expr::BinaryOp { left, right, .. } => is_aggregate_expr(left) || is_aggregate_expr(right),
        Expr::UnaryOp { expr, .. } => is_aggregate_expr(expr),
        _ => false,
    }
}

fn expr_to_column_name(expr: &Expr, index: usize) -> String {
    match expr {
        Expr::Column(name) => name.clone(),
        Expr::FunctionCall { name, arg } => match arg {
            Some(a) => format!("{}({})", name, expr_to_column_name(a, 0)),
            None => format!("{}()", name),
        },
        Expr::Literal(val) => val.to_string_repr(),
        _ => format!("col_{}", index + 1),
    }
}

fn eval_expr(
    expr: &Expr,
    values: &[SqlValue],
    col_map: &HashMap<String, usize>,
    rowid: Option<i64>,
) -> anyhow::Result<SqlValue> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Column(name) => {
            let clean = clean_identifier(name);
            let lower = clean.to_lowercase();
            if lower == "rowid" || lower == "oid" || lower == "_rowid_" {
                return Ok(match rowid {
                    Some(id) => SqlValue::Integer(id),
                    None => SqlValue::Null,
                });
            }
            if let Some(&cid) = col_map.get(&lower) {
                Ok(values.get(cid).cloned().unwrap_or(SqlValue::Null))
            } else {
                // Check if table.column syntax
                if let Some(pos) = lower.find('.') {
                    let col_part = &lower[pos + 1..];
                    if let Some(&cid) = col_map.get(col_part) {
                        return Ok(values.get(cid).cloned().unwrap_or(SqlValue::Null));
                    }
                }
                Ok(SqlValue::Null)
            }
        }
        Expr::BinaryOp { left, op, right } => {
            let l = eval_expr(left, values, col_map, rowid)?;
            let r = eval_expr(right, values, col_map, rowid)?;
            match op.as_str() {
                "+" => match (l, r) {
                    (SqlValue::Integer(a), SqlValue::Integer(b)) => Ok(SqlValue::Integer(a + b)),
                    (SqlValue::Real(a), SqlValue::Real(b)) => Ok(SqlValue::Real(a + b)),
                    (SqlValue::Integer(a), SqlValue::Real(b)) => Ok(SqlValue::Real(a as f64 + b)),
                    (SqlValue::Real(a), SqlValue::Integer(b)) => Ok(SqlValue::Real(a + b as f64)),
                    _ => Ok(SqlValue::Null),
                },
                "-" => match (l, r) {
                    (SqlValue::Integer(a), SqlValue::Integer(b)) => Ok(SqlValue::Integer(a - b)),
                    (SqlValue::Real(a), SqlValue::Real(b)) => Ok(SqlValue::Real(a - b)),
                    (SqlValue::Integer(a), SqlValue::Real(b)) => Ok(SqlValue::Real(a as f64 - b)),
                    (SqlValue::Real(a), SqlValue::Integer(b)) => Ok(SqlValue::Real(a - b as f64)),
                    _ => Ok(SqlValue::Null),
                },
                "*" => match (l, r) {
                    (SqlValue::Integer(a), SqlValue::Integer(b)) => Ok(SqlValue::Integer(a * b)),
                    (SqlValue::Real(a), SqlValue::Real(b)) => Ok(SqlValue::Real(a * b)),
                    (SqlValue::Integer(a), SqlValue::Real(b)) => Ok(SqlValue::Real(a as f64 * b)),
                    (SqlValue::Real(a), SqlValue::Integer(b)) => Ok(SqlValue::Real(a * b as f64)),
                    _ => Ok(SqlValue::Null),
                },
                "/" => match (l, r) {
                    (SqlValue::Integer(a), SqlValue::Integer(b)) => {
                        if b == 0 {
                            Ok(SqlValue::Null)
                        } else {
                            Ok(SqlValue::Integer(a / b))
                        }
                    }
                    (SqlValue::Real(a), SqlValue::Real(b)) => {
                        if b == 0.0 {
                            Ok(SqlValue::Null)
                        } else {
                            Ok(SqlValue::Real(a / b))
                        }
                    }
                    (SqlValue::Integer(a), SqlValue::Real(b)) => {
                        if b == 0.0 {
                            Ok(SqlValue::Null)
                        } else {
                            Ok(SqlValue::Real(a as f64 / b))
                        }
                    }
                    (SqlValue::Real(a), SqlValue::Integer(b)) => {
                        if b == 0 {
                            Ok(SqlValue::Null)
                        } else {
                            Ok(SqlValue::Real(a / b as f64))
                        }
                    }
                    _ => Ok(SqlValue::Null),
                },
                "||" => {
                    let s1 = l.to_string_repr();
                    let s2 = r.to_string_repr();
                    Ok(SqlValue::Text(format!("{}{}", s1, s2)))
                }
                _ => Ok(SqlValue::Null),
            }
        }
        Expr::UnaryOp { op, expr } => {
            let v = eval_expr(expr, values, col_map, rowid)?;
            if op == "-" {
                match v {
                    SqlValue::Integer(i) => Ok(SqlValue::Integer(-i)),
                    SqlValue::Real(f) => Ok(SqlValue::Real(-f)),
                    _ => Ok(SqlValue::Null),
                }
            } else {
                Ok(v)
            }
        }
        Expr::FunctionCall { name, arg } => {
            let fn_upper = name.to_uppercase();
            match fn_upper.as_str() {
                "UPPER" => {
                    if let Some(a) = arg {
                        let v = eval_expr(a, values, col_map, rowid)?;
                        Ok(match v {
                            SqlValue::Text(s) => SqlValue::Text(s.to_uppercase()),
                            _ => v,
                        })
                    } else {
                        Ok(SqlValue::Null)
                    }
                }
                "LOWER" => {
                    if let Some(a) = arg {
                        let v = eval_expr(a, values, col_map, rowid)?;
                        Ok(match v {
                            SqlValue::Text(s) => SqlValue::Text(s.to_lowercase()),
                            _ => v,
                        })
                    } else {
                        Ok(SqlValue::Null)
                    }
                }
                "LENGTH" => {
                    if let Some(a) = arg {
                        let v = eval_expr(a, values, col_map, rowid)?;
                        Ok(match v {
                            SqlValue::Text(s) => SqlValue::Integer(s.len() as i64),
                            SqlValue::Blob(b) => SqlValue::Integer(b.len() as i64),
                            _ => SqlValue::Null,
                        })
                    } else {
                        Ok(SqlValue::Null)
                    }
                }
                "HEX" => {
                    if let Some(a) = arg {
                        let v = eval_expr(a, values, col_map, rowid)?;
                        match v {
                            SqlValue::Blob(b) => {
                                let hex_s: String =
                                    b.iter().map(|byte| format!("{:02X}", byte)).collect();
                                Ok(SqlValue::Text(hex_s))
                            }
                            SqlValue::Text(s) => {
                                let hex_s: String = s
                                    .as_bytes()
                                    .iter()
                                    .map(|byte| format!("{:02X}", byte))
                                    .collect();
                                Ok(SqlValue::Text(hex_s))
                            }
                            _ => Ok(SqlValue::Null),
                        }
                    } else {
                        Ok(SqlValue::Null)
                    }
                }
                _ => Ok(SqlValue::Null),
            }
        }
        _ => Ok(SqlValue::Null),
    }
}

fn eval_bool_expr(
    expr: &Expr,
    values: &[SqlValue],
    col_map: &HashMap<String, usize>,
    rowid: Option<i64>,
) -> anyhow::Result<bool> {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            let op_upper = op.to_uppercase();
            if op_upper == "AND" {
                let l = eval_bool_expr(left, values, col_map, rowid)?;
                if !l {
                    return Ok(false);
                }
                return eval_bool_expr(right, values, col_map, rowid);
            }
            if op_upper == "OR" {
                let l = eval_bool_expr(left, values, col_map, rowid)?;
                if l {
                    return Ok(true);
                }
                return eval_bool_expr(right, values, col_map, rowid);
            }

            let l = eval_expr(left, values, col_map, rowid)?;
            let r = eval_expr(right, values, col_map, rowid)?;

            if l.is_null() || r.is_null() {
                return Ok(false);
            }

            let ord = l.compare(&r);
            match op.as_str() {
                "=" | "==" => Ok(ord == std::cmp::Ordering::Equal),
                "!=" | "<>" => Ok(ord != std::cmp::Ordering::Equal),
                "<" => Ok(ord == std::cmp::Ordering::Less),
                "<=" => Ok(ord == std::cmp::Ordering::Less || ord == std::cmp::Ordering::Equal),
                ">" => Ok(ord == std::cmp::Ordering::Greater),
                ">=" => Ok(ord == std::cmp::Ordering::Greater || ord == std::cmp::Ordering::Equal),
                _ => Ok(false),
            }
        }
        Expr::UnaryOp { op, expr } => {
            if op.eq_ignore_ascii_case("NOT") {
                let b = eval_bool_expr(expr, values, col_map, rowid)?;
                Ok(!b)
            } else {
                Ok(false)
            }
        }
        Expr::IsNull(inner) => {
            let v = eval_expr(inner, values, col_map, rowid)?;
            Ok(v.is_null())
        }
        Expr::IsNotNull(inner) => {
            let v = eval_expr(inner, values, col_map, rowid)?;
            Ok(!v.is_null())
        }
        Expr::Like {
            expr: inner,
            pattern,
            negated,
        } => {
            let v = eval_expr(inner, values, col_map, rowid)?;
            let p = eval_expr(pattern, values, col_map, rowid)?;
            if v.is_null() || p.is_null() {
                return Ok(false);
            }
            let s = v.to_string_repr();
            let pat = p.to_string_repr();
            let matched = sql_like_match(&pat, &s);
            Ok(if *negated { !matched } else { matched })
        }
        Expr::Between {
            expr: inner,
            low,
            high,
            negated,
        } => {
            let v = eval_expr(inner, values, col_map, rowid)?;
            let l = eval_expr(low, values, col_map, rowid)?;
            let h = eval_expr(high, values, col_map, rowid)?;
            if v.is_null() || l.is_null() || h.is_null() {
                return Ok(false);
            }
            let ge_low = v.compare(&l) != std::cmp::Ordering::Less;
            let le_high = v.compare(&h) != std::cmp::Ordering::Greater;
            let in_range = ge_low && le_high;
            Ok(if *negated { !in_range } else { in_range })
        }
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let v = eval_expr(inner, values, col_map, rowid)?;
            if v.is_null() {
                return Ok(false);
            }
            let mut found = false;
            for item in list {
                let target = eval_expr(item, values, col_map, rowid)?;
                if v == target {
                    found = true;
                    break;
                }
            }
            Ok(if *negated { !found } else { found })
        }
        _ => {
            // Truthy test
            let v = eval_expr(expr, values, col_map, rowid)?;
            match v {
                SqlValue::Null => Ok(false),
                SqlValue::Integer(i) => Ok(i != 0),
                SqlValue::Real(f) => Ok(f != 0.0),
                SqlValue::Text(s) => Ok(!s.is_empty()),
                SqlValue::Blob(b) => Ok(!b.is_empty()),
            }
        }
    }
}

fn sql_like_match(pattern: &str, text: &str) -> bool {
    let pat_lower = pattern.to_lowercase();
    let text_lower = text.to_lowercase();
    let pat_chars: Vec<char> = pat_lower.chars().collect();
    let text_chars: Vec<char> = text_lower.chars().collect();
    like_rec(&pat_chars, &text_chars, 0, 0)
}

fn like_rec(pat: &[char], text: &[char], p_idx: usize, t_idx: usize) -> bool {
    if p_idx == pat.len() {
        return t_idx == text.len();
    }
    if pat[p_idx] == '%' {
        for next_t in t_idx..=text.len() {
            if like_rec(pat, text, p_idx + 1, next_t) {
                return true;
            }
        }
        return false;
    }
    if t_idx == text.len() {
        return false;
    }
    if pat[p_idx] == '_' || pat[p_idx] == text[t_idx] {
        return like_rec(pat, text, p_idx + 1, t_idx + 1);
    }
    false
}

// Simple SQL tokenizer & Parser
// ===========================================================================
// SQL Safety Guardrails & Pre-processors
// ===========================================================================

pub fn strip_sql_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let n = chars.len();

    while i < n {
        let c = chars[i];

        // String literals with single or double quotes
        if c == '\'' || c == '"' {
            let quote = c;
            out.push(quote);
            i += 1;
            while i < n {
                let sc = chars[i];
                out.push(sc);
                if sc == quote {
                    // Check escaped quote '' or ""
                    if i + 1 < n && chars[i + 1] == quote {
                        i += 1;
                        out.push(chars[i]);
                    } else {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            continue;
        }

        // Single-line comment --
        if c == '-' && i + 1 < n && chars[i + 1] == '-' {
            i += 2;
            while i < n && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            out.push(' ');
            continue;
        }

        // Multi-line comment /* ... */
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < n {
                i += 2; // skip */
            } else {
                i = n;
            }
            out.push(' ');
            continue;
        }

        out.push(c);
        i += 1;
    }

    out
}

pub fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let n = chars.len();

    while i < n {
        let c = chars[i];
        if c == '\'' || c == '"' {
            let quote = c;
            current.push(quote);
            i += 1;
            while i < n {
                let sc = chars[i];
                current.push(sc);
                if sc == quote {
                    if i + 1 < n && chars[i + 1] == quote {
                        i += 1;
                        current.push(chars[i]);
                    } else {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            continue;
        }

        if c == ';' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                stmts.push(trimmed.to_string());
                current.clear();
            }
            i += 1;
            continue;
        }

        current.push(c);
        i += 1;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed.to_string());
    }

    stmts
}

pub fn validate_read_only_query(sql: &str) -> anyhow::Result<()> {
    let stripped = strip_sql_comments(sql);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Empty SQL query");
    }

    let statements = split_sql_statements(&stripped);
    if statements.is_empty() {
        anyhow::bail!("Empty SQL query");
    }
    if statements.len() > 1 {
        anyhow::bail!("Multi-statement SQL queries are not permitted in read-only inspection mode");
    }

    let stmt = statements[0].trim();
    let first_word = stmt.split_whitespace().next().unwrap_or("").to_uppercase();

    // Check forbidden modification / administrative commands
    const FORBIDDEN_COMMANDS: &[&str] = &[
        "INSERT",
        "UPDATE",
        "DELETE",
        "DROP",
        "ALTER",
        "CREATE",
        "REPLACE",
        "TRUNCATE",
        "ATTACH",
        "DETACH",
        "VACUUM",
        "REINDEX",
        "GRANT",
        "REVOKE",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "SAVEPOINT",
        "RELEASE",
        "UPSERT",
    ];

    for &forbidden in FORBIDDEN_COMMANDS {
        if first_word == forbidden {
            anyhow::bail!(
                "Modification query '{}' is forbidden. Only read-only queries (SELECT, .tables, .schema, PRAGMA table_info) are permitted.",
                forbidden
            );
        }
    }

    // Check for PRAGMA modifications
    if first_word == "PRAGMA" {
        let upper_stmt = stmt.to_uppercase();
        if upper_stmt.contains('=') {
            anyhow::bail!(
                "PRAGMA statement modifying database settings is forbidden in read-only mode"
            );
        }
    }

    let allowed_prefixes = [
        "SELECT", "EXPLAIN", "PRAGMA", "SHOW", "DESCRIBE", "DESC", ".TABLES", ".SCHEMA",
    ];
    if !allowed_prefixes.iter().any(|p| first_word == *p) {
        anyhow::bail!(
            "Unsupported or unsafe query command '{}'. Only read-only queries (SELECT, .tables, .schema, PRAGMA table_info) are permitted.",
            first_word
        );
    }

    Ok(())
}

// Simple SQL tokenizer & Parser
fn parse_select_query(sql: &str) -> anyhow::Result<SelectQuery> {
    let tokens = tokenize_sql(sql);
    let mut parser = QueryParser::new(tokens);
    parser.parse()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Keyword(String),
    Ident(String),
    StringLit(String),
    NumberLit(String),
    Symbol(char),
    DoubleSymbol(String),
}

fn tokenize_sql(input: &str) -> Vec<Token> {
    let clean_input = strip_sql_comments(input);
    let mut tokens = Vec::new();
    let chars: Vec<char> = clean_input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // Two-character symbols: <=, >=, !=, <>, ||, ==
        if i + 1 < chars.len() {
            let next = chars[i + 1];
            let pair = format!("{}{}", c, next);
            if matches!(pair.as_str(), "<=" | ">=" | "!=" | "<>" | "||" | "==") {
                tokens.push(Token::DoubleSymbol(pair));
                i += 2;
                continue;
            }
        }

        // Single character symbols
        if matches!(
            c,
            '(' | ')' | ',' | '.' | '=' | '<' | '>' | '+' | '-' | '*' | '/' | ';'
        ) {
            tokens.push(Token::Symbol(c));
            i += 1;
            continue;
        }

        // String literals: 'text' or "text"
        if c == '\'' || c == '"' {
            let quote_char = c;
            i += 1;
            let mut s = String::new();
            while i < chars.len() {
                if chars[i] == quote_char {
                    if i + 1 < chars.len() && chars[i + 1] == quote_char {
                        s.push(quote_char);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                s.push(chars[i]);
                i += 1;
            }
            if quote_char == '\'' {
                tokens.push(Token::StringLit(s));
            } else {
                tokens.push(Token::Ident(s));
            }
            continue;
        }

        // Backticks `ident` or square brackets [ident]
        if c == '`' || c == '[' {
            let close_c = if c == '`' { '`' } else { ']' };
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != close_c {
                s.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == close_c {
                i += 1;
            }
            tokens.push(Token::Ident(s));
            continue;
        }

        // Numbers
        if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let mut num_str = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                num_str.push(chars[i]);
                i += 1;
            }
            tokens.push(Token::NumberLit(num_str));
            continue;
        }

        // Identifiers / Keywords
        if c.is_alphanumeric() || c == '_' {
            let mut word = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                word.push(chars[i]);
                i += 1;
            }
            let upper = word.to_uppercase();
            if is_sql_keyword(&upper) {
                tokens.push(Token::Keyword(upper));
            } else {
                tokens.push(Token::Ident(word));
            }
            continue;
        }

        i += 1;
    }

    tokens
}

fn is_sql_keyword(word: &str) -> bool {
    matches!(
        word,
        "SELECT"
            | "DISTINCT"
            | "FROM"
            | "WHERE"
            | "AND"
            | "OR"
            | "NOT"
            | "IS"
            | "NULL"
            | "LIKE"
            | "ILIKE"
            | "IN"
            | "BETWEEN"
            | "ORDER"
            | "BY"
            | "ASC"
            | "DESC"
            | "LIMIT"
            | "OFFSET"
            | "AS"
            | "GROUP"
            | "HAVING"
    )
}

struct QueryParser {
    tokens: Vec<Token>,
    pos: usize,
}

impl QueryParser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let t = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    fn match_keyword(&mut self, kw: &str) -> bool {
        if let Some(Token::Keyword(k)) = self.peek() {
            if k == kw {
                self.advance();
                return true;
            }
        }
        false
    }

    fn match_symbol(&mut self, sym: char) -> bool {
        if let Some(Token::Symbol(s)) = self.peek() {
            if *s == sym {
                self.advance();
                return true;
            }
        }
        false
    }

    fn parse(&mut self) -> anyhow::Result<SelectQuery> {
        if !self.match_keyword("SELECT") {
            anyhow::bail!("Expected SELECT at start of query");
        }

        let distinct = self.match_keyword("DISTINCT");

        // Parse select items
        let mut items = Vec::new();
        loop {
            let expr = self.parse_expr()?;
            let mut alias = None;
            if self.match_keyword("AS") {
                if let Some(Token::Ident(id)) = self.advance() {
                    alias = Some(id);
                }
            } else if let Some(Token::Ident(id)) = self.peek().cloned() {
                // Check if next token is NOT a keyword like FROM
                if !is_sql_keyword(&id.to_uppercase()) {
                    self.advance();
                    alias = Some(id);
                }
            }

            items.push(SelectItem { expr, alias });

            if self.match_symbol(',') {
                continue;
            }
            break;
        }

        let mut table = None;
        if self.match_keyword("FROM") {
            if let Some(Token::Ident(t)) = self.advance() {
                table = Some(t);
            } else {
                anyhow::bail!("Expected table name after FROM");
            }
        }

        let mut where_clause = None;
        if self.match_keyword("WHERE") {
            where_clause = Some(self.parse_expr()?);
        }

        // ORDER BY
        let mut order_by = Vec::new();
        if self.match_keyword("ORDER") {
            if !self.match_keyword("BY") {
                anyhow::bail!("Expected BY after ORDER");
            }
            loop {
                let expr = self.parse_expr()?;
                let mut descending = false;
                if self.match_keyword("DESC") {
                    descending = true;
                } else if self.match_keyword("ASC") {
                    descending = false;
                }
                order_by.push(OrderByItem { expr, descending });
                if self.match_symbol(',') {
                    continue;
                }
                break;
            }
        }

        // LIMIT & OFFSET
        let mut limit = None;
        let mut offset = None;

        if self.match_keyword("LIMIT") {
            if let Some(Token::NumberLit(n)) = self.advance() {
                limit = n.parse::<usize>().ok();
            }
            if self.match_keyword("OFFSET") {
                if let Some(Token::NumberLit(n)) = self.advance() {
                    offset = n.parse::<usize>().ok();
                }
            } else if self.match_symbol(',') {
                // SQLite syntax: LIMIT offset, limit
                let first = limit;
                if let Some(Token::NumberLit(n)) = self.advance() {
                    offset = first;
                    limit = n.parse::<usize>().ok();
                }
            }
        } else if self.match_keyword("OFFSET") {
            if let Some(Token::NumberLit(n)) = self.advance() {
                offset = n.parse::<usize>().ok();
            }
        }
        // Consume optional trailing semicolon(s)
        while self.match_symbol(';') {}

        if self.pos < self.tokens.len() {
            anyhow::bail!("Unexpected trailing tokens after SELECT query");
        }

        Ok(SelectQuery {
            distinct,
            items,
            table,
            where_clause,
            order_by,
            limit,
            offset,
        })
    }

    fn parse_expr(&mut self) -> anyhow::Result<Expr> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> anyhow::Result<Expr> {
        let mut left = self.parse_and_expr()?;
        while self.match_keyword("OR") {
            let right = self.parse_and_expr()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: "OR".to_string(),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> anyhow::Result<Expr> {
        let mut left = self.parse_comparison_expr()?;
        while self.match_keyword("AND") {
            let right = self.parse_comparison_expr()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: "AND".to_string(),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison_expr(&mut self) -> anyhow::Result<Expr> {
        let left = self.parse_arith_expr()?;

        // IS [NOT] NULL
        if self.match_keyword("IS") {
            let negated = self.match_keyword("NOT");
            if self.match_keyword("NULL") {
                return Ok(if negated {
                    Expr::IsNotNull(Box::new(left))
                } else {
                    Expr::IsNull(Box::new(left))
                });
            }
        }

        // [NOT] LIKE / ILIKE
        let not_before = self.match_keyword("NOT");
        if self.match_keyword("LIKE") || self.match_keyword("ILIKE") {
            let pattern = self.parse_arith_expr()?;
            return Ok(Expr::Like {
                expr: Box::new(left),
                pattern: Box::new(pattern),
                negated: not_before,
            });
        }

        // [NOT] BETWEEN a AND b
        if self.match_keyword("BETWEEN") {
            let low = self.parse_arith_expr()?;
            if !self.match_keyword("AND") {
                anyhow::bail!("Expected AND after BETWEEN");
            }
            let high = self.parse_arith_expr()?;
            return Ok(Expr::Between {
                expr: Box::new(left),
                low: Box::new(low),
                high: Box::new(high),
                negated: not_before,
            });
        }

        // [NOT] IN (...)
        if self.match_keyword("IN") {
            if !self.match_symbol('(') {
                anyhow::bail!("Expected '(' after IN");
            }
            let mut list = Vec::new();
            while !self.match_symbol(')') {
                list.push(self.parse_expr()?);
                if self.match_symbol(',') {
                    continue;
                }
            }
            return Ok(Expr::InList {
                expr: Box::new(left),
                list,
                negated: not_before,
            });
        }

        // Standard comparison symbols: =, !=, <>, <, <=, >, >=
        if let Some(Token::Symbol(s)) = self.peek().cloned() {
            if matches!(s, '=' | '<' | '>') {
                self.advance();
                let op = s.to_string();
                let right = self.parse_arith_expr()?;
                return Ok(Expr::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                });
            }
        }

        if let Some(Token::DoubleSymbol(ds)) = self.peek() {
            if matches!(ds.as_str(), "<=" | ">=" | "!=" | "<>" | "==") {
                let op = ds.clone();
                self.advance();
                let right = self.parse_arith_expr()?;
                return Ok(Expr::BinaryOp {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                });
            }
        }

        Ok(left)
    }

    fn parse_arith_expr(&mut self) -> anyhow::Result<Expr> {
        let mut left = self.parse_factor_expr()?;
        while let Some(Token::Symbol(s)) = self.peek().cloned() {
            if s == '+' || s == '-' {
                self.advance();
                let right = self.parse_factor_expr()?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: s.to_string(),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_factor_expr(&mut self) -> anyhow::Result<Expr> {
        let mut left = self.parse_primary_expr()?;
        while let Some(Token::Symbol(s)) = self.peek().cloned() {
            if s == '*' || s == '/' {
                self.advance();
                let right = self.parse_primary_expr()?;
                left = Expr::BinaryOp {
                    left: Box::new(left),
                    op: s.to_string(),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_primary_expr(&mut self) -> anyhow::Result<Expr> {
        // Parenthesized expression
        if self.match_symbol('(') {
            let expr = self.parse_expr()?;
            if !self.match_symbol(')') {
                anyhow::bail!("Unclosed parenthesis in expression");
            }
            return Ok(expr);
        }

        // Unary NOT or minus
        if self.match_keyword("NOT") {
            let expr = self.parse_primary_expr()?;
            return Ok(Expr::UnaryOp {
                op: "NOT".to_string(),
                expr: Box::new(expr),
            });
        }
        if self.match_symbol('-') {
            let expr = self.parse_primary_expr()?;
            return Ok(Expr::UnaryOp {
                op: "-".to_string(),
                expr: Box::new(expr),
            });
        }

        if self.match_keyword("NULL") {
            return Ok(Expr::Literal(SqlValue::Null));
        }

        // Wildcard *
        if self.match_symbol('*') {
            return Ok(Expr::Column("*".to_string()));
        }

        // String literal
        if let Some(Token::StringLit(s)) = self.peek().cloned() {
            self.advance();
            return Ok(Expr::Literal(SqlValue::Text(s)));
        }

        // Number literal
        if let Some(Token::NumberLit(n)) = self.peek().cloned() {
            self.advance();
            if n.contains('.') {
                if let Ok(f) = n.parse::<f64>() {
                    return Ok(Expr::Literal(SqlValue::Real(f)));
                }
            } else if let Ok(i) = n.parse::<i64>() {
                return Ok(Expr::Literal(SqlValue::Integer(i)));
            }
            return Ok(Expr::Literal(SqlValue::Text(n)));
        }

        // Identifier or function call
        if let Some(Token::Ident(id)) = self.peek().cloned() {
            self.advance();

            // Check if table.column
            if self.match_symbol('.') {
                if self.match_symbol('*') {
                    return Ok(Expr::Column(format!("{}.*", id)));
                }
                if let Some(Token::Ident(col)) = self.advance() {
                    return Ok(Expr::Column(format!("{}.{}", id, col)));
                }
            }

            // Check if function call e.g. COUNT(*) or UPPER(col)
            if self.match_symbol('(') {
                let mut arg = None;
                if !self.match_symbol(')') {
                    if self.match_symbol('*') {
                        arg = Some(Box::new(Expr::Column("*".to_string())));
                    } else {
                        arg = Some(Box::new(self.parse_expr()?));
                    }
                    if !self.match_symbol(')') {
                        anyhow::bail!("Expected ')' in function call for {}", id);
                    }
                }
                return Ok(Expr::FunctionCall { name: id, arg });
            }

            return Ok(Expr::Column(id));
        }

        anyhow::bail!("Unexpected token in expression: {:?}", self.peek())
    }
}

// ===========================================================================
// Formatters
// ===========================================================================

pub fn format_table(res: &QueryResult) -> String {
    if res.columns.is_empty() {
        return "Empty result set (0 columns)".to_string();
    }

    let mut col_widths: Vec<usize> = res.columns.iter().map(|c| c.len().max(4)).collect();

    for row in &res.rows {
        for (i, val) in row.iter().enumerate() {
            if i < col_widths.len() {
                let s = val.to_string_repr();
                col_widths[i] = col_widths[i].max(s.len());
            }
        }
    }

    let mut out = String::new();

    // Top border
    out.push('+');
    for &w in &col_widths {
        out.push_str(&format!("{}+", "-".repeat(w + 2)));
    }
    out.push('\n');

    // Header row
    out.push('|');
    for (i, col) in res.columns.iter().enumerate() {
        let w = col_widths[i];
        out.push_str(&format!(" {:<w$} |", col, w = w));
    }
    out.push('\n');

    // Header separator
    out.push('+');
    for &w in &col_widths {
        out.push_str(&format!("{}+", "=".repeat(w + 2)));
    }
    out.push('\n');

    // Rows
    if res.rows.is_empty() {
        out.push_str("| (0 rows returned)");
        let total_width: usize = col_widths.iter().map(|w| w + 3).sum();
        let pad = total_width.saturating_sub(19);
        out.push_str(&" ".repeat(pad));
        out.push_str("|\n");
    } else {
        for row in &res.rows {
            out.push('|');
            for (i, w) in col_widths.iter().enumerate() {
                let val_str = row.get(i).map(|v| v.to_string_repr()).unwrap_or_default();
                out.push_str(&format!(" {:<w$} |", val_str, w = *w));
            }
            out.push('\n');
        }
    }

    // Bottom border
    out.push('+');
    for &w in &col_widths {
        out.push_str(&format!("{}+", "-".repeat(w + 2)));
    }
    out.push('\n');

    out.push_str(&format!(
        "({} row{} returned)",
        res.rows.len(),
        if res.rows.len() == 1 { "" } else { "s" }
    ));

    out
}

pub fn format_json(res: &QueryResult) -> String {
    let mut json_rows = Vec::new();
    for row in &res.rows {
        let mut map = serde_json::Map::new();
        for (i, col) in res.columns.iter().enumerate() {
            let val = row.get(i).map(|v| v.to_json()).unwrap_or(Value::Null);
            map.insert(col.clone(), val);
        }
        json_rows.push(Value::Object(map));
    }
    serde_json::to_string_pretty(&json_rows).unwrap_or_else(|_| "[]".to_string())
}

pub fn format_csv(res: &QueryResult) -> String {
    let mut out = String::new();
    // Header
    let headers: Vec<String> = res.columns.iter().map(|c| csv_escape(c)).collect();
    out.push_str(&headers.join(","));
    out.push('\n');

    // Rows
    for row in &res.rows {
        let row_vals: Vec<String> = row
            .iter()
            .map(|v| csv_escape(&v.to_string_repr()))
            .collect();
        out.push_str(&row_vals.join(","));
        out.push('\n');
    }

    out
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

// ===========================================================================
// SqliteTool Implementation
// ===========================================================================

#[derive(Default, Debug, Clone)]
pub struct SqliteTool;

impl SqliteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SqliteTool {
    fn name(&self) -> &str {
        "sqlite"
    }

    fn description(&self) -> &str {
        "Inspect and query local SQLite database files (.sqlite, .db, .sqlite3) in pure Rust. Execute SQL SELECT queries, view database schema, list tables, inspect columns, view database metadata, or dump table data."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the SQLite database file (.sqlite, .db, .sqlite3, etc.)."
                },
                "query": {
                    "type": "string",
                    "description": "SQL query to execute (e.g., 'SELECT * FROM users WHERE age > 21 LIMIT 10')."
                },
                "action": {
                    "type": "string",
                    "enum": ["query", "tables", "schema", "describe", "info", "indexes", "dump"],
                    "description": "Action to perform: 'query' (execute SQL), 'tables' (list tables with counts), 'schema' (show DDL schema), 'describe' (show table structure), 'info' (database header metadata), 'indexes' (list all indexes), 'dump' (export table data). Defaults to 'query' if query is provided, or 'tables' otherwise."
                },
                "table": {
                    "type": "string",
                    "description": "Table name for 'schema', 'describe', or 'dump' actions."
                },
                "format": {
                    "type": "string",
                    "enum": ["table", "json", "csv"],
                    "description": "Output format: 'table' (default, ASCII table), 'json', or 'csv'."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of rows to return (default: 100)."
                },
                "offset": {
                    "type": "integer",
                    "description": "Number of rows to skip (default: 0)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .or_else(|| args.get("db_path").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let full_path = resolve_path(path_str, &ctx.cwd);

        if !full_path.exists() {
            anyhow::bail!("SQLite database file not found: '{}'", full_path.display());
        }

        if full_path.is_dir() {
            anyhow::bail!(
                "Specified path is a directory, not a SQLite file: '{}'",
                full_path.display()
            );
        }

        let file_bytes = tokio::fs::read(&full_path).await.map_err(|e| {
            anyhow::anyhow!("Failed to read SQLite file '{}': {e}", full_path.display())
        })?;

        let reader = SqliteReader::new(&file_bytes).map_err(|e| {
            anyhow::anyhow!("Failed to parse SQLite file '{}': {e}", full_path.display())
        })?;

        let engine = SqlEngine::new(&reader);

        let query_param = args.get("query").and_then(|v| v.as_str()).map(|s| s.trim());
        let action_param = args
            .get("action")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_lowercase());
        let table_param = args.get("table").and_then(|v| v.as_str()).map(|s| s.trim());
        let format_param = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("table");

        let action = match (action_param.as_deref(), query_param) {
            (Some(act), _) => act.to_string(),
            (None, Some(q)) if !q.is_empty() => "query".to_string(),
            (None, None) => "tables".to_string(),
            (None, Some(_)) => "tables".to_string(),
        };

        match action.as_str() {
            "query" | "sql" => {
                let sql = query_param.ok_or_else(|| anyhow::anyhow!("Action 'query' requires 'query' parameter"))?;
                let mut res = engine.execute(sql)?;

                if let Some(limit_val) = args.get("limit").and_then(|v| v.as_u64()) {
                    let limit = limit_val as usize;
                    if res.rows.len() > limit {
                        res.rows.truncate(limit);
                    }
                }

                Ok(format_result(&res, format_param))
            }
            "tables" => {
                let entries = reader.read_master_entries()?;
                let mut table_rows = Vec::new();

                for entry in &entries {
                    if entry.object_type == "table" || entry.object_type == "view" {
                        let is_sys = entry.name.starts_with("sqlite_");
                        let row_count = if entry.object_type == "table" && entry.rootpage > 0 {
                            reader.read_table_rows(entry.rootpage).map(|r| r.len()).unwrap_or(0)
                        } else {
                            0
                        };

                        let col_count = if let Some(sql) = &entry.sql {
                            parse_columns_from_ddl(sql).len()
                        } else {
                            0
                        };

                        table_rows.push(vec![
                            SqlValue::Text(entry.object_type.clone()),
                            SqlValue::Text(entry.name.clone()),
                            SqlValue::Integer(col_count as i64),
                            SqlValue::Integer(row_count as i64),
                            SqlValue::Integer(entry.rootpage as i64),
                            SqlValue::Text(if is_sys { "system" } else { "user" }.to_string()),
                        ]);
                    }
                }

                let res = QueryResult {
                    columns: vec![
                        "type".to_string(),
                        "name".to_string(),
                        "columns".to_string(),
                        "rows".to_string(),
                        "rootpage".to_string(),
                        "scope".to_string(),
                    ],
                    rows: table_rows,
                };

                Ok(format_result(&res, format_param))
            }
            "schema" => {
                let entries = reader.read_master_entries()?;
                let mut schema_rows = Vec::new();

                for entry in entries {
                    if let Some(target) = table_param {
                        if !entry.name.eq_ignore_ascii_case(target) && !entry.tbl_name.eq_ignore_ascii_case(target) {
                            continue;
                        }
                    }

                    if let Some(sql) = entry.sql {
                        schema_rows.push(vec![
                            SqlValue::Text(entry.object_type),
                            SqlValue::Text(entry.name),
                            SqlValue::Text(entry.tbl_name),
                            SqlValue::Text(sql),
                        ]);
                    }
                }

                if schema_rows.is_empty() {
                    if let Some(t) = table_param {
                        anyhow::bail!("No schema found for table '{}'", t);
                    }
                }

                let res = QueryResult {
                    columns: vec![
                        "type".to_string(),
                        "name".to_string(),
                        "table".to_string(),
                        "sql".to_string(),
                    ],
                    rows: schema_rows,
                };

                Ok(format_result(&res, format_param))
            }
            "describe" | "columns" => {
                let target_table = table_param.ok_or_else(|| anyhow::anyhow!("Action 'describe' requires 'table' parameter"))?;
                let schema = reader.get_table_schema(target_table)?
                    .ok_or_else(|| anyhow::anyhow!("Table '{}' not found", target_table))?;

                let mut rows = Vec::new();
                for col in schema.columns {
                    rows.push(vec![
                        SqlValue::Integer(col.cid as i64),
                        SqlValue::Text(col.name),
                        SqlValue::Text(col.data_type),
                        SqlValue::Integer(if col.notnull { 1 } else { 0 }),
                        match col.dflt_value {
                            Some(v) => SqlValue::Text(v),
                            None => SqlValue::Null,
                        },
                        SqlValue::Integer(if col.pk { 1 } else { 0 }),
                    ]);
                }

                let res = QueryResult {
                    columns: vec![
                        "cid".to_string(),
                        "name".to_string(),
                        "type".to_string(),
                        "notnull".to_string(),
                        "default".to_string(),
                        "pk".to_string(),
                    ],
                    rows,
                };

                Ok(format_result(&res, format_param))
            }
            "indexes" => {
                let entries = reader.read_master_entries()?;
                let mut idx_rows = Vec::new();

                for entry in entries {
                    if entry.object_type == "index" {
                        if let Some(target) = table_param {
                            if !entry.tbl_name.eq_ignore_ascii_case(target) {
                                continue;
                            }
                        }

                        let is_unique = entry.sql.as_deref().map(|s| s.to_uppercase().contains("UNIQUE")).unwrap_or(false);

                        idx_rows.push(vec![
                            SqlValue::Text(entry.name),
                            SqlValue::Text(entry.tbl_name),
                            SqlValue::Integer(if is_unique { 1 } else { 0 }),
                            SqlValue::Integer(entry.rootpage as i64),
                            match entry.sql {
                                Some(s) => SqlValue::Text(s),
                                None => SqlValue::Text("(autoindex)".to_string()),
                            },
                        ]);
                    }
                }

                let res = QueryResult {
                    columns: vec![
                        "index_name".to_string(),
                        "table_name".to_string(),
                        "unique".to_string(),
                        "rootpage".to_string(),
                        "sql".to_string(),
                    ],
                    rows: idx_rows,
                };

                Ok(format_result(&res, format_param))
            }
            "info" | "metadata" => {
                let h = &reader.header;
                let file_size = file_bytes.len();
                let entries = reader.read_master_entries().unwrap_or_default();
                let tables_count = entries.iter().filter(|e| e.object_type == "table").count();
                let views_count = entries.iter().filter(|e| e.object_type == "view").count();
                let indexes_count = entries.iter().filter(|e| e.object_type == "index").count();
                let triggers_count = entries.iter().filter(|e| e.object_type == "trigger").count();

                let metadata_rows = vec![
                    vec![SqlValue::Text("SQLite Version".to_string()), SqlValue::Text(h.sqlite_version_str())],
                    vec![SqlValue::Text("File Size".to_string()), SqlValue::Text(format!("{} bytes ({:.2} KB)", file_size, file_size as f64 / 1024.0))],
                    vec![SqlValue::Text("Page Size".to_string()), SqlValue::Text(format!("{} bytes", h.page_size))],
                    vec![SqlValue::Text("Usable Page Size".to_string()), SqlValue::Text(format!("{} bytes", reader.usable_page_size))],
                    vec![SqlValue::Text("Total Pages".to_string()), SqlValue::Integer((file_size / h.page_size as usize) as i64)],
                    vec![SqlValue::Text("Encoding".to_string()), SqlValue::Text(h.encoding_str().to_string())],
                    vec![SqlValue::Text("Schema Format".to_string()), SqlValue::Integer(h.schema_format as i64)],
                    vec![SqlValue::Text("Schema Cookie".to_string()), SqlValue::Integer(h.schema_cookie as i64)],
                    vec![SqlValue::Text("User Version".to_string()), SqlValue::Integer(h.user_version as i64)],
                    vec![SqlValue::Text("Freelist Pages".to_string()), SqlValue::Integer(h.total_freelist_pages as i64)],
                    vec![SqlValue::Text("File Change Counter".to_string()), SqlValue::Integer(h.file_change_counter as i64)],
                    vec![SqlValue::Text("WAL Mode (Read/Write Version)".to_string()), SqlValue::Text(format!("{}/{}", h.read_version, h.write_version))],
                    vec![SqlValue::Text("Tables Count".to_string()), SqlValue::Integer(tables_count as i64)],
                    vec![SqlValue::Text("Views Count".to_string()), SqlValue::Integer(views_count as i64)],
                    vec![SqlValue::Text("Indexes Count".to_string()), SqlValue::Integer(indexes_count as i64)],
                    vec![SqlValue::Text("Triggers Count".to_string()), SqlValue::Integer(triggers_count as i64)],
                ];

                let res = QueryResult {
                    columns: vec!["Property".to_string(), "Value".to_string()],
                    rows: metadata_rows,
                };

                Ok(format_result(&res, format_param))
            }
            "dump" => {
                let target_table = table_param.ok_or_else(|| anyhow::anyhow!("Action 'dump' requires 'table' parameter"))?;
                let query = format!("SELECT * FROM {}", target_table);
                let res = engine.execute(&query)?;
                Ok(format_result(&res, format_param))
            }
            other => anyhow::bail!("Unknown action '{}'. Supported actions: query, tables, schema, describe, info, indexes, dump", other),
        }
    }
}

fn format_result(res: &QueryResult, format_type: &str) -> String {
    match format_type.to_lowercase().as_str() {
        "json" => format_json(res),
        "csv" => format_csv(res),
        _ => format_table(res),
    }
}

// ===========================================================================
// Unit & Integration Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_varint_parsing() {
        // Single byte varint: 42
        let buf = [0x2A];
        let mut offset = 0;
        let v = SqliteReader::read_varint(&buf, &mut offset).unwrap();
        assert_eq!(v, 42);
        assert_eq!(offset, 1);

        // Multi-byte varint: 300 = (0x82, 0x2C) -> (2 << 7) | 44 = 300
        let buf = [0x82, 0x2C];
        let mut offset = 0;
        let v = SqliteReader::read_varint(&buf, &mut offset).unwrap();
        assert_eq!(v, 300);
        assert_eq!(offset, 2);
    }

    #[test]
    fn test_record_parsing() {
        // Create mock header + reader
        let mut db_bytes = vec![0u8; 4096];
        db_bytes[0..16].copy_from_slice(b"SQLite format 3\0");
        db_bytes[16..18].copy_from_slice(&4096u16.to_be_bytes());
        db_bytes[18] = 1;
        db_bytes[19] = 1;
        db_bytes[21] = 64;
        db_bytes[22] = 32;
        db_bytes[23] = 32;
        db_bytes[56..60].copy_from_slice(&1u32.to_be_bytes()); // UTF-8

        let reader = SqliteReader::new(&db_bytes).unwrap();

        // Record with:
        // Col 1: Integer 42 (serial type 1, value 42)
        // Col 2: Text "hello" (serial type 13 + 2*5 = 23, value "hello")
        // Col 3: Null (serial type 0)
        // Header length: 4 bytes (varint 4, st 1, st 23, st 0)
        let payload = vec![
            4, 1, 23, 0,  // header
            42, // int 42
            b'h', b'e', b'l', b'l', b'o', // text hello
        ];

        let values = reader.parse_record(&payload).unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], SqlValue::Integer(42));
        assert_eq!(values[1], SqlValue::Text("hello".to_string()));
        assert_eq!(values[2], SqlValue::Null);
    }

    #[test]
    fn test_sql_like_match() {
        assert!(sql_like_match("hello", "hello"));
        assert!(sql_like_match("%world%", "hello world today"));
        assert!(sql_like_match("a%c", "abc"));
        assert!(sql_like_match("a_c", "abc"));
        assert!(!sql_like_match("a_c", "abbc"));
        assert!(sql_like_match("%.com", "user@example.com"));
        assert!(sql_like_match("%", "anything"));
    }

    #[test]
    fn test_ddl_parser() {
        let sql = "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email VARCHAR(255) DEFAULT 'none', score REAL, active BOOLEAN)";
        let cols = parse_columns_from_ddl(sql);
        assert_eq!(cols.len(), 5);
        assert_eq!(cols[0].name, "id");
        assert_eq!(cols[0].data_type, "INTEGER");
        assert!(cols[0].pk);
        assert_eq!(cols[1].name, "name");
        assert_eq!(cols[1].data_type, "TEXT");
        assert!(cols[1].notnull);
        assert_eq!(cols[2].name, "email");
        assert_eq!(cols[2].data_type, "VARCHAR(255)");
        assert_eq!(cols[2].dflt_value, Some("none".to_string()));
    }

    #[tokio::test]
    async fn test_sqlite_tool_real_database() {
        // Create a temporary SQLite database using python3 if available, or skip
        let temp = NamedTempFile::new().unwrap();
        let db_path = temp.path().to_str().unwrap().to_string();

        let python_code = format!(
            "import sqlite3\nconn = sqlite3.connect('{}')\nc = conn.cursor()\nc.execute('CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INT, score REAL)')\nc.execute('INSERT INTO users VALUES (1, \"Alice\", 30, 95.5)')\nc.execute('INSERT INTO users VALUES (2, \"Bob\", 25, 88.0)')\nc.execute('INSERT INTO users VALUES (3, \"Charlie\", 35, 72.3)')\nconn.commit()\nconn.close()",
            db_path
        );

        let py_res = std::process::Command::new("python3")
            .arg("-c")
            .arg(&python_code)
            .output();

        if let Ok(output) = py_res {
            if output.status.success() {
                let tool = SqliteTool::new();
                let ctx = ToolContext {
                    cwd: PathBuf::from("."),
                    env: HashMap::new(),
                };

                // 1. Test tables action
                let res = tool
                    .execute(json!({ "path": db_path, "action": "tables" }), &ctx)
                    .await
                    .unwrap();
                assert!(res.contains("users"));

                // 2. Test query action: SELECT *
                let res = tool
                    .execute(
                        json!({ "path": db_path, "query": "SELECT * FROM users ORDER BY age ASC" }),
                        &ctx,
                    )
                    .await
                    .unwrap();
                assert!(res.contains("Alice"));
                assert!(res.contains("Bob"));
                assert!(res.contains("Charlie"));

                // 3. Test query action with WHERE filter
                let res = tool.execute(json!({ "path": db_path, "query": "SELECT name, age FROM users WHERE age > 26" }), &ctx).await.unwrap();
                assert!(res.contains("Alice"));
                assert!(res.contains("Charlie"));
                assert!(!res.contains("Bob"));

                // 4. Test aggregate query: COUNT(*), AVG(score)
                let res = tool.execute(json!({ "path": db_path, "query": "SELECT COUNT(*), AVG(score) FROM users" }), &ctx).await.unwrap();
                assert!(res.contains("3"));

                // 5. Test JSON format
                let res = tool.execute(json!({ "path": db_path, "query": "SELECT name FROM users WHERE id = 1", "format": "json" }), &ctx).await.unwrap();
                assert!(res.contains("\"name\": \"Alice\""));

                // 6. Test CSV format
                let res = tool.execute(json!({ "path": db_path, "query": "SELECT id, name FROM users", "format": "csv" }), &ctx).await.unwrap();
                assert!(res.contains("id,name"));
                assert!(res.contains("1,Alice"));

                // 7. Test info action
                let res = tool
                    .execute(json!({ "path": db_path, "action": "info" }), &ctx)
                    .await
                    .unwrap();
                assert!(res.contains("Page Size"));
                assert!(res.contains("Tables Count"));

                // 8. Test describe action
                let res = tool
                    .execute(
                        json!({ "path": db_path, "action": "describe", "table": "users" }),
                        &ctx,
                    )
                    .await
                    .unwrap();
                assert!(res.contains("name"));
                assert!(res.contains("age"));
                assert!(res.contains("score"));
            }
        }
    }

    #[test]
    fn test_strip_sql_comments() {
        assert_eq!(
            strip_sql_comments("SELECT * -- line comment\nFROM users"),
            "SELECT *   FROM users"
        );
        assert_eq!(
            strip_sql_comments("SELECT /* block comment */ name FROM users"),
            "SELECT   name FROM users"
        );
        assert_eq!(
            strip_sql_comments("SELECT 'hello -- not a comment' FROM users"),
            "SELECT 'hello -- not a comment' FROM users"
        );
        assert_eq!(
            strip_sql_comments("SELECT \"/* not a comment */\" FROM users"),
            "SELECT \"/* not a comment */\" FROM users"
        );
    }

    #[test]
    fn test_query_safety_guardrails_rejections() {
        // Forbidden modification commands
        assert!(validate_read_only_query("INSERT INTO users VALUES (1, 'Eve')").is_err());
        assert!(validate_read_only_query("UPDATE users SET score = 100").is_err());
        assert!(validate_read_only_query("DELETE FROM users WHERE id = 1").is_err());
        assert!(validate_read_only_query("DROP TABLE users").is_err());
        assert!(validate_read_only_query("ALTER TABLE users ADD COLUMN age INT").is_err());
        assert!(validate_read_only_query("CREATE TABLE secrets (id INT)").is_err());
        assert!(validate_read_only_query("REPLACE INTO users VALUES (1, 'Eve')").is_err());
        assert!(validate_read_only_query("ATTACH DATABASE 'malicious.db' AS evil").is_err());
        assert!(validate_read_only_query("DETACH DATABASE evil").is_err());
        assert!(validate_read_only_query("VACUUM").is_err());
        assert!(validate_read_only_query("REINDEX").is_err());

        // Multi-statement injection
        assert!(validate_read_only_query("SELECT 1; DROP TABLE users;").is_err());
        assert!(validate_read_only_query("SELECT * FROM users; DELETE FROM users;").is_err());

        // Unsafe pragma modifications
        assert!(validate_read_only_query("PRAGMA writable_schema = ON").is_err());
        assert!(validate_read_only_query("PRAGMA journal_mode = WAL").is_err());

        // Empty query
        assert!(validate_read_only_query("").is_err());
        assert!(validate_read_only_query("   -- just a comment\n  ").is_err());
    }

    #[test]
    fn test_query_safety_guardrails_allowed() {
        assert!(validate_read_only_query("SELECT * FROM users").is_ok());
        assert!(validate_read_only_query(
            "SELECT name, age FROM users WHERE age > 21 ORDER BY age DESC LIMIT 10"
        )
        .is_ok());
        assert!(validate_read_only_query("-- read users\nSELECT * FROM users").is_ok());
        assert!(validate_read_only_query("/* read query */ SELECT id, score FROM users;").is_ok());
        assert!(validate_read_only_query("PRAGMA table_info(users)").is_ok());
        assert!(validate_read_only_query("PRAGMA database_list").is_ok());
        assert!(validate_read_only_query(".tables").is_ok());
        assert!(validate_read_only_query(".schema users").is_ok());
        assert!(validate_read_only_query("SHOW TABLES").is_ok());
        assert!(validate_read_only_query("DESCRIBE users").is_ok());
    }

    #[test]
    fn test_formatters_table_json_csv() {
        let res = QueryResult {
            columns: vec!["id".to_string(), "name".to_string(), "active".to_string()],
            rows: vec![
                vec![
                    SqlValue::Integer(1),
                    SqlValue::Text("Alice".to_string()),
                    SqlValue::Integer(1),
                ],
                vec![
                    SqlValue::Integer(2),
                    SqlValue::Text("Bob, \"The Builder\"".to_string()),
                    SqlValue::Null,
                ],
            ],
        };

        // Tabular format
        let table_out = format_table(&res);
        assert!(table_out.contains("| id"));
        assert!(table_out.contains("| name"));
        assert!(table_out.contains("| Alice"));
        assert!(table_out.contains("(2 rows returned)"));

        // JSON format
        let json_out = format_json(&res);
        assert!(json_out.contains("\"id\": 1"));
        assert!(json_out.contains("\"name\": \"Alice\""));
        assert!(json_out.contains("\"active\": null"));

        // CSV format
        let csv_out = format_csv(&res);
        assert!(csv_out.contains("id,name,active"));
        assert!(csv_out.contains("1,Alice,1"));
        assert!(csv_out.contains("2,\"Bob, \"\"The Builder\"\"\",NULL"));
    }

    #[test]
    fn test_tool_trait_metadata() {
        let tool = SqliteTool::new();
        assert_eq!(tool.name(), "sqlite");
        assert!(tool.description().contains("SQLite"));
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["path"].is_object());
        assert!(params["properties"]["query"].is_object());
        assert!(params["properties"]["action"].is_object());
        assert!(params["properties"]["format"].is_object());
    }
}
