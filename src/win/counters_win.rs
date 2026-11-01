//! Windows network counters: GetIfTable2 rows as AdapterInfo.

use crate::counters::AdapterInfo;

/// `IF_TYPE_SOFTWARE_LOOPBACK` from ipifcons.h.
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
/// Bit positions in `MIB_IF_ROW2.InterfaceAndOperStatusFlags`.
const FLAG_HARDWARE_INTERFACE: u8 = 1 << 0;
const FLAG_FILTER_INTERFACE: u8 = 1 << 1;

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Snapshot of every interface Windows knows about, with 64-bit octet counters
/// (IPv4 and IPv6 combined).
pub fn read_adapters() -> windows::core::Result<Vec<AdapterInfo>> {
    use windows::Win32::Foundation::{NO_ERROR, WIN32_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: GetIfTable2 allocates the table and writes its pointer; we free it below.
    let status: WIN32_ERROR = unsafe { GetIfTable2(&mut table) };
    if status != NO_ERROR {
        return Err(status.into());
    }
    // SAFETY: on success `table` points to a header followed by NumEntries rows,
    // valid until FreeMibTable. Rows are copied out before freeing.
    let list = unsafe {
        let t = &*table;
        let rows = std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize);
        let list = rows
            .iter()
            .map(|row| {
                let flags = row.InterfaceAndOperStatusFlags._bitfield;
                AdapterInfo {
                    guid: format!("{:?}", row.InterfaceGuid),
                    alias: wide_to_string(&row.Alias),
                    description: wide_to_string(&row.Description),
                    is_up: row.OperStatus == IfOperStatusUp,
                    is_hardware: flags & FLAG_HARDWARE_INTERFACE != 0,
                    is_loopback: row.Type == IF_TYPE_SOFTWARE_LOOPBACK,
                    is_filter: flags & FLAG_FILTER_INTERFACE != 0,
                    in_octets: row.InOctets,
                    out_octets: row.OutOctets,
                }
            })
            .collect();
        FreeMibTable(table as *const _);
        list
    };
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_adapters_smoke() {
        let list = read_adapters().expect("GetIfTable2");
        assert!(!list.is_empty());
        assert!(list.iter().all(|a| !a.guid.is_empty()));
    }
}
