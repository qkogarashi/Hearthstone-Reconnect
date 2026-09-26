use std::net::Ipv4Addr;

use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP_STATE_DELETE_TCB, MIB_TCP_STATE_ESTAB, MIB_TCPROW_LH, SetTcpEntry,
    TCP_TABLE_OWNER_PID_ALL,
};
use windows::Win32::Networking::WinSock::AF_INET;

use crate::process::get_pid_by_name;

const HEARTHSTONE_EXE: &str = "Hearthstone.exe";
const GAME_PORT: u16 = 1119;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Reconnecting,
    NotInGame,
    NotRunning,
    Failed,
}

#[repr(C)]
struct MibTcpTableOwnerPid {
    dw_num_entries: u32,
    table: [MibTcpRowOwnerPid; 1],
}

#[repr(C)]
#[derive(Clone)]
struct MibTcpRowOwnerPid {
    dw_state: u32,
    dw_local_addr: u32,
    dw_local_port: u32,
    dw_remote_addr: u32,
    dw_remote_port: u32,
    dw_owning_pid: u32,
}

fn raw_to_port(raw: u32) -> u16 {
    u16::from_be((raw & 0xFFFF) as u16)
}

fn raw_to_ip(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(raw.to_ne_bytes())
}

// 34.x.x.x  — Google Cloud
// 35.x.x.x  — Google Cloud
// 52.x.x.x  — AWS
// 18.x.x.x  — AWS
// 137.x.x.x — Microsoft Azure (CDN)
// 99.x.x.x  — AWS CloudFront
fn is_persistent_server(ip: Ipv4Addr) -> bool {
    matches!(ip.octets()[0], 34 | 35 | 52 | 18 | 137 | 99)
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
    pub is_game_server: bool,
    raw_local_addr: u32,
    raw_local_port: u32,
    raw_remote_addr: u32,
    raw_remote_port: u32,
}

pub fn get_hs_connections() -> Option<Vec<Connection>> {
    let pid = get_pid_by_name(HEARTHSTONE_EXE)?;

    let mut sz: u32 = 0;
    let mut result = Vec::new();
    unsafe {
        let mut buf: Vec<u64> = Vec::new();
        let ok = loop {
            let ret = GetExtendedTcpTable(
                Some(buf.as_mut_ptr() as *mut _),
                &mut sz,
                false,
                u32::from(AF_INET.0),
                TCP_TABLE_OWNER_PID_ALL,
                0,
            );
            if ret == ERROR_INSUFFICIENT_BUFFER.0 {
                buf.resize(sz as usize / 8 + 64, 0);
                continue;
            }
            break ret == 0;
        };
        if ok {
            let table = &*(buf.as_ptr() as *const MibTcpTableOwnerPid);
            let max_rows = (buf.len() * 8 - 4) / size_of::<MibTcpRowOwnerPid>();
            let rows = (table.dw_num_entries as usize).min(max_rows);

            for i in 0..rows {
                let row = &*table.table.as_ptr().add(i);
                if row.dw_owning_pid != pid || row.dw_state != MIB_TCP_STATE_ESTAB.0 as u32 {
                    continue;
                }

                let ip = raw_to_ip(row.dw_remote_addr);
                if ip.is_loopback() || ip.is_unspecified() {
                    continue;
                }

                let port = raw_to_port(row.dw_remote_port);
                result.push(Connection {
                    is_game_server: port == GAME_PORT && !is_persistent_server(ip),
                    remote_ip: ip,
                    remote_port: port,
                    raw_local_addr: row.dw_local_addr,
                    raw_local_port: row.dw_local_port,
                    raw_remote_addr: row.dw_remote_addr,
                    raw_remote_port: row.dw_remote_port,
                });
            }
        }
    }
    result.sort_by_key(|c| std::cmp::Reverse(c.is_game_server));
    Some(result)
}

pub fn kill_connection(conn: &Connection) -> Outcome {
    let ok = unsafe {
        let mut k = MIB_TCPROW_LH::default();
        k.Anonymous.dwState = MIB_TCP_STATE_DELETE_TCB.0 as u32;
        k.dwLocalAddr = conn.raw_local_addr;
        k.dwLocalPort = conn.raw_local_port;
        k.dwRemoteAddr = conn.raw_remote_addr;
        k.dwRemotePort = conn.raw_remote_port;
        SetTcpEntry(&k) == 0
    };
    if ok {
        Outcome::Reconnecting
    } else {
        Outcome::Failed
    }
}

pub fn do_smart_reconnect() -> Outcome {
    let Some(conns) = get_hs_connections() else {
        return Outcome::NotRunning;
    };
    let results: Vec<Outcome> = conns
        .iter()
        .filter(|c| c.is_game_server)
        .map(kill_connection)
        .collect();
    if results.is_empty() {
        Outcome::NotInGame
    } else if results.contains(&Outcome::Reconnecting) {
        Outcome::Reconnecting
    } else {
        Outcome::Failed
    }
}
