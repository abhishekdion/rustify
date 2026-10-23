//! Benchmark harness for the py2rs-transpiled JK BMS protocol code.
//!
//! Reads the same fixture file as `bench/bench_python.py` and runs the same
//! three decode workloads, so the two sides can be compared directly. Each
//! workload prints an FNV-1a 64-bit digest of its results; the digests must
//! match the Python output exactly, which proves the transpiled code computes
//! the same answers before any timing is compared.

use std::fs;
use std::time::Instant;

use dipulse_protocol::gateway__app__drivers__bms__jk__frames as frames;
use dipulse_protocol::gateway__app__drivers__bms__jk__parser as parser;

struct Digest {
    h: u64,
}

impl Digest {
    fn new() -> Self {
        Digest {
            h: 0xcbf2_9ce4_8422_2325,
        }
    }
    fn byte(&mut self, b: u64) {
        self.h = (self.h ^ (b & 0xFF)).wrapping_mul(0x100_0000_01b3);
    }
    fn u16(&mut self, v: u64) {
        self.byte(v);
        self.byte(v >> 8);
    }
    fn u32(&mut self, v: u64) {
        self.u16(v & 0xFFFF);
        self.u16(v >> 16);
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.h)
    }
}

fn load_fixtures(path: &str) -> Vec<Vec<Vec<u8>>> {
    let data = fs::read(path).expect("cannot read fixtures");
    assert_eq!(&data[0..7], b"PY2RSBF", "bad fixture magic");
    assert_eq!(data[7], 1, "unsupported fixture version");
    let mut off = 8usize;
    let mut groups = Vec::new();
    for _ in 0..3 {
        let count = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        off += 4;
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count {
            let len = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
            off += 4;
            frames.push(data[off..off + len].to_vec());
            off += len;
        }
        groups.push(frames);
    }
    groups
}

// ---------------------------------------------------------------------------
// Workloads (mirroring bench/bench_python.py)
// ---------------------------------------------------------------------------

fn workload_modbus(frames_in: &[Vec<u8>]) -> String {
    let mut d = Digest::new();
    for f in frames_in {
        let (ok, regs, _err) = frames::parse_modbus_read_response(f, Some(1));
        d.byte(if ok { 1 } else { 0 });
        if let Some(regs) = regs {
            for r in regs {
                d.u16(r as u64);
            }
        }
    }
    d.hex()
}

fn workload_tlv(frames_in: &[Vec<u8>]) -> String {
    let mut d = Digest::new();
    for f in frames_in {
        match parser::parse_jk_response(f) {
            None => d.byte(0xFF),
            Some(tel) => {
                d.byte(if tel.checksum_ok { 1 } else { 0 });
                d.u16(tel.num_cells as u64);
                for mv in &tel.cell_voltages_mv {
                    d.u16(*mv as u64);
                }
                d.u32((tel.voltage * 1000.0).round() as u64);
                d.u32((tel.current * 1000.0).round() as u64);
                d.u32((tel.soc * 1000.0).round() as u64);
                d.u32((tel.temp_c * 100.0).round() as u64);
                d.u16(tel.invalid_fields.len() as u64);
            }
        }
    }
    d.hex()
}

fn workload_pb(frames_in: &[Vec<u8>]) -> String {
    let mut d = Digest::new();
    for f in frames_in {
        let crc_full = frames::_crc16_modbus(&f[..306]);
        let crc_body = frames::_crc16_modbus(&f[4..306]);
        let crc_le = (f[306] as i64) | ((f[307] as i64) << 8);
        let crc_be = ((f[306] as i64) << 8) | (f[307] as i64);
        let ok = crc_full == crc_le
            || crc_full == crc_be
            || crc_body == crc_le
            || crc_body == crc_be;
        d.byte(if ok { 1 } else { 0 });
        d.u16((crc_full as u64) & 0xFFFF);
        d.u16((crc_body as u64) & 0xFFFF);
    }
    d.hex()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "bench/fixtures.bin".to_string());
    let repeat: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);

    let groups = load_fixtures(&path);
    let workloads: [(&str, fn(&[Vec<u8>]) -> String); 3] = [
        ("modbus", workload_modbus),
        ("tlv", workload_tlv),
        ("pb", workload_pb),
    ];

    println!(
        "{:<10} {:>8} {:>12} {:>16}  digest",
        "workload", "frames", "best (ms)", "per frame (us)"
    );
    for (i, (name, f)) in workloads.iter().enumerate() {
        let frames_in = &groups[i];
        let digest = f(frames_in);
        let mut best = f64::MAX;
        for _ in 0..repeat {
            let t0 = Instant::now();
            f(frames_in);
            let dt = t0.elapsed().as_secs_f64();
            if dt < best {
                best = dt;
            }
        }
        println!(
            "{:<10} {:>8} {:>12.3} {:>16.3}  {}",
            name,
            frames_in.len(),
            best * 1000.0,
            best / frames_in.len().max(1) as f64 * 1e6,
            digest
        );
    }
}
