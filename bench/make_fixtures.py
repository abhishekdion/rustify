"""
bench/make_fixtures.py
──────────────────────
Build the benchmark fixture set once, from the *original* Dipulse Python code,
so that the Python and Rust sides measure exactly the same bytes.

Three frame families are produced:

  1. Modbus RTU FC 0x03 read responses (the live PB-Modbus poll path).
     16-cell block and 40-word summary block.
  2. JK proprietary TLV responses (the standard-TLV poll path).
  3. JK PB-Series 308-byte frames (the PB-Series poll path), which cost two
     CRC-16 passes over 306 bytes each.

Only *response* frames are synthesised. No write frames are built and no
serial hardware is touched — this is a decode-path benchmark.

Usage:
    python bench/make_fixtures.py [--out bench/fixtures.bin] [--count 512]
"""

from __future__ import annotations

import argparse
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from dipulse_loader import load_protocol_modules  # noqa: E402

_frames_mod, _parser_mod, _models_mod = load_protocol_modules()
_crc16_modbus = _frames_mod._crc16_modbus

MAGIC = b"PY2RSBF"
VERSION = 1


def _modbus_response(slave: int, registers: list[int]) -> bytes:
    """Build a well-formed FC 0x03 read response frame."""
    body = bytearray([slave, 0x03, len(registers) * 2])
    for reg in registers:
        body += struct.pack(">H", reg & 0xFFFF)
    crc = _crc16_modbus(bytes(body))
    return bytes(body) + bytes([crc & 0xFF, (crc >> 8) & 0xFF])


def _tlv_response(cell_mv: list[int], temp_deci_c: int, pack_dv: int,
                  current_raw: int, soc: int, firmware: str) -> bytes:
    """Build a well-formed JK proprietary TLV response frame."""
    tlv = bytearray()

    # 0x79 cell voltages
    tlv += bytes([0x79, len(cell_mv) * 2])
    for mv in cell_mv:
        tlv += struct.pack(">H", mv)

    # 0x80 average temperature (0.1 degC)
    tlv += bytes([0x80, 0x02]) + struct.pack(">H", temp_deci_c)
    # 0x81 total pack voltage (0.01 V)
    tlv += bytes([0x81, 0x02]) + struct.pack(">H", pack_dv)
    # 0x82 total current (0.01 A, offset 32768)
    tlv += bytes([0x82, 0x02]) + struct.pack(">H", current_raw)
    # 0x83 state of charge
    tlv += bytes([0x83, 0x01, soc])
    # 0x84 remaining capacity (0.001 Ah)
    tlv += bytes([0x84, 0x04]) + struct.pack(">I", 95_500)
    # 0x85 nominal capacity
    tlv += bytes([0x85, 0x04]) + struct.pack(">I", 100_000)
    # 0x8B warning bitfield
    tlv += bytes([0x8B, 0x02]) + struct.pack(">H", 0)
    # 0x8C status byte
    tlv += bytes([0x8C, 0x01, 0x05])
    # 0x8E state of health
    tlv += bytes([0x8E, 0x02]) + struct.pack(">H", 92)
    # 0xB4 firmware
    fw = firmware.encode("ascii")
    tlv += bytes([0xB4, len(fw)]) + fw
    # 0xB6 serial number
    sn = b"JKPB1A16S10P"
    tlv += bytes([0xB6, len(sn)]) + sn

    body = bytearray([0x4E, 0x57, 0x00, 0x00, 0, 0, 0, 0, 0x06, 0x03, 0x00, 0x68])
    body += tlv
    checksum = sum(body) & 0xFFFFFFFF
    return bytes(body) + struct.pack(">I", checksum)


def _pb_series_frame(seed: int) -> bytes:
    """Build a 308-byte JK PB-Series frame with a valid trailing CRC-16."""
    frame = bytearray(308)
    frame[0:4] = bytes([0x55, 0xAA, 0xEB, 0x90])
    frame[4] = 0x02
    frame[5] = 0x00
    for i in range(24):
        mv = 3300 + ((seed * 7 + i * 13) % 120)
        frame[6 + i * 2] = mv & 0xFF
        frame[7 + i * 2] = (mv >> 8) & 0xFF
    struct.pack_into("<I", frame, 112, 52837)          # battery voltage mV
    struct.pack_into("<I", frame, 116, 352)             # battery power W
    struct.pack_into("<H", frame, 124, 299)             # T1 (0.1 degC)
    struct.pack_into("<H", frame, 126, 304)             # T2 (0.1 degC)
    struct.pack_into("<H", frame, 128, 315)             # MOS (0.1 degC)
    frame[134] = 64                                     # SOC %
    struct.pack_into("<I", frame, 135, 107_400)         # remaining mAh
    struct.pack_into("<I", frame, 139, 150_000)         # nominal mAh
    struct.pack_into("<I", frame, 143, 12)              # cycle count
    crc = _crc16_modbus(bytes(frame[:306]))
    frame[306] = crc & 0xFF
    frame[307] = (crc >> 8) & 0xFF
    return bytes(frame)


def build(count: int) -> bytes:
    modbus_frames: list[bytes] = []
    tlv_frames: list[bytes] = []
    pb_frames: list[bytes] = []

    for i in range(count):
        # Modbus: 16-cell block (0x1200) and 40-word summary block (0x1248).
        cells = [3300 + ((i * 3 + c * 11) % 140) for c in range(16)]
        modbus_frames.append(_modbus_response(0x01, cells))

        summary = [0] * 40
        summary[0] = 231                       # power board temp (s16, 0.1C)
        summary[8] = 253                       # battery temp 1
        summary[9] = 247                       # battery temp 2
        summary[33] = 299                      # CMOS temp (verified 0.1C)
        summary[38] = (352_500 >> 16) & 0xFFFF  # battery power mW (u32 BE)
        summary[39] = 352_500 & 0xFFFF
        modbus_frames.append(_modbus_response(0x01, summary))

        # JK TLV frame.
        cell_mv = [3350 + ((i + c) % 40) for c in range(16)]
        tlv_frames.append(
            _tlv_response(
                cell_mv=cell_mv,
                temp_deci_c=250 + (i % 20),
                pack_dv=5360,
                current_raw=32768 + (i % 500),
                soc=85,
                firmware="JK-B2A20S20P_V11.XW",
            )
        )

        # PB-Series frame.
        pb_frames.append(_pb_series_frame(i))

    out = bytearray()
    out += MAGIC
    out += bytes([VERSION])
    for group in (modbus_frames, tlv_frames, pb_frames):
        out += struct.pack("<I", len(group))
        for f in group:
            out += struct.pack("<I", len(f))
            out += f
    return bytes(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures.bin"))
    ap.add_argument("--count", type=int, default=512)
    args = ap.parse_args()

    data = build(args.count)
    with open(args.out, "wb") as fh:
        fh.write(data)
    print(f"wrote {args.out} ({len(data)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
