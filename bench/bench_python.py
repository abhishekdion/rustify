"""
bench/bench_python.py
─────────────────────
Baseline side of the Python-vs-Rust benchmark.

Reads the shared fixture file produced by make_fixtures.py and runs the three
decode workloads that the gateway performs on every poll:

    A. Modbus RTU FC 0x03 response parsing  (live PB-Modbus poll path)
    B. JK proprietary TLV parsing            (standard-TLV poll path)
    C. JK PB-Series 308-byte frame validation (PB-Series poll path)

Each workload prints an FNV-1a digest of its results. The Rust side computes
the same digest, so a matching digest proves the two implementations agree
bit-for-bit before any timing is compared.

Usage:
    python bench/bench_python.py [--fixtures bench/fixtures.bin] [--repeat 5]
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from dipulse_loader import load_protocol_modules  # noqa: E402

_frames_mod, _parser_mod, _models_mod = load_protocol_modules()
_crc16_modbus = _frames_mod._crc16_modbus
parse_modbus_read_response = _frames_mod.parse_modbus_read_response
parse_jk_response = _parser_mod.parse_jk_response

MASK = 0xFFFFFFFFFFFFFFFF


class Digest:
    """FNV-1a 64-bit over an explicit byte stream."""

    def __init__(self) -> None:
        self.h = 0xCBF29CE484222325

    def byte(self, b: int) -> None:
        self.h = ((self.h ^ (b & 0xFF)) * 0x100000001B3) & MASK

    def u16(self, v: int) -> None:
        self.byte(v & 0xFF)
        self.byte((v >> 8) & 0xFF)

    def u32(self, v: int) -> None:
        self.u16(v & 0xFFFF)
        self.u16((v >> 16) & 0xFFFF)

    def hex(self) -> str:
        return f"{self.h:016x}"


def load_fixtures(path: str):
    with open(path, "rb") as fh:
        data = fh.read()
    assert data[:7] == b"PY2RSBF", "bad fixture magic"
    version = data[7]
    assert version == 1, f"unsupported fixture version {version}"
    off = 8
    groups = []
    for _ in range(3):
        (count,) = struct.unpack_from("<I", data, off)
        off += 4
        frames = []
        for _ in range(count):
            (length,) = struct.unpack_from("<I", data, off)
            off += 4
            frames.append(data[off:off + length])
            off += length
        groups.append(frames)
    return groups


# ---------------------------------------------------------------------------
# Workloads
# ---------------------------------------------------------------------------

def workload_modbus(frames: list[bytes]) -> str:
    d = Digest()
    for frame in frames:
        ok, regs, err = parse_modbus_read_response(frame, expected_slave=0x01)
        d.byte(1 if ok else 0)
        if regs:
            for r in regs:
                d.u16(r)
    return d.hex()


def workload_tlv(frames: list[bytes]) -> str:
    d = Digest()
    for frame in frames:
        tel = parse_jk_response(frame)
        if tel is None:
            d.byte(0xFF)
            continue
        d.byte(1 if tel.checksum_ok else 0)
        d.u16(tel.num_cells)
        for mv in tel.cell_voltages_mv:
            d.u16(mv)
        d.u32(int(round(tel.voltage * 1000)))
        d.u32(int(round(tel.current * 1000)))
        d.u32(int(round(tel.soc * 1000)))
        d.u32(int(round(tel.temp_c * 100)))
        d.u16(len(tel.invalid_fields))
    return d.hex()


def workload_pb(frames: list[bytes]) -> str:
    d = Digest()
    for frame in frames:
        crc_full = _crc16_modbus(frame[:306])
        crc_body = _crc16_modbus(frame[4:306])
        crc_le = frame[306] | (frame[307] << 8)
        crc_be = (frame[306] << 8) | frame[307]
        ok = crc_full in (crc_le, crc_be) or crc_body in (crc_le, crc_be)
        d.byte(1 if ok else 0)
        d.u16(crc_full & 0xFFFF)
        d.u16(crc_body & 0xFFFF)
    return d.hex()


WORKLOADS = {
    "modbus": workload_modbus,
    "tlv": workload_tlv,
    "pb": workload_pb,
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--fixtures", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures.bin"))
    ap.add_argument("--repeat", type=int, default=5)
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    groups = load_fixtures(args.fixtures)
    names = ["modbus", "tlv", "pb"]

    results = {}
    for name, frames in zip(names, groups):
        fn = WORKLOADS[name]
        digest = fn(frames)          # warm-up + parity digest
        times = []
        for _ in range(args.repeat):
            t0 = time.perf_counter()
            fn(frames)
            times.append(time.perf_counter() - t0)
        results[name] = {
            "frames": len(frames),
            "digest": digest,
            "best_s": min(times),
            "median_s": statistics.median(times),
            "per_frame_us": min(times) / max(1, len(frames)) * 1e6,
        }

    if args.json:
        print(json.dumps(results, indent=2))
    else:
        print(f"{'workload':<10} {'frames':>8} {'best (ms)':>12} {'per frame (us)':>16}  digest")
        for name, r in results.items():
            print(
                f"{name:<10} {r['frames']:>8} {r['best_s']*1000:>12.3f} "
                f"{r['per_frame_us']:>16.3f}  {r['digest']}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
