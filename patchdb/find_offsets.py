#!/usr/bin/env python3
"""Multi-tier offset finder for Discord `discord_voice.node`.

The finder resolves the byte sites in `patchdb/tables.json` against an
arbitrary PE build. It is deliberately stricter than a plain AOB scanner:

* exact `(goal, md5)` tables win when the binary is known;
* cross-build consensus patterns are tried next;
* per-build context patterns are the fallback;
* relative anchors recover sites whose own pattern no longer matches;
* every candidate is verified against the expected original or patched
  bytes before it can be accepted.

The output can be a human table, a machine JSON scan, a schema-2 divergence
report accepted by `patchdb/autopatch.mjs`, or a patched copy. The input file
is never modified.

Usage:
    python3 patchdb/find_offsets.py scan <node> [--goal v6] [--report out.json]
    python3 patchdb/find_offsets.py learn --sample <md5>=<node> ... \
        --goal v6 --out patchdb/offset_profiles.json
    python3 patchdb/find_offsets.py selftest --sample <md5>=<node> ...
    python3 patchdb/find_offsets.py diff <clean> <patched>
    python3 patchdb/find_offsets.py inspect <node>
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import os
import re
import shutil
import struct
import sys
import tempfile
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Sequence

VERSION = "1.0.0"
PROFILE_SCHEMA = "stereo-wizard-offset-profiles-v1"
REPORT_SCHEMA = 2
DEFAULT_TABLES = Path(__file__).resolve().with_name("tables.json")
DEFAULT_PROFILES = Path(__file__).resolve().with_name("offset_profiles.json")

MIN_CONTEXT = 16
MAX_CONTEXT = 128
MAX_HITS = 4096
MAX_PATTERN_HITS = 100_000

# Canonical names are the names used by the newest table. Aliases are the
# names used by older tables for the same semantic site.
SITE_ALIASES: dict[str, list[str]] = {
    "CapturedAudioProcessor_APMBypass_NopJmp": ["MonoDownmix bypass (NOP+JMP)"],
    "CommitAudioCodec_ChannelCount_Imm02": ["CommitAudioCodec channel count 2"],
    "OpusSdp_StereoValueBypass_Jmp2": ["OpusSdp force stereo"],
    "CreateAudioFrame_ChannelAssign_Mov": ["CreateAudioFrame channel assign 2"],
    "OpusSdp_MaxPlaybackRate_Keep48k_Nop4": ["OpusSdp max playback rate 48k"],
    "HighPassFilter_Process_Entry_Ret": ["HighPassFilter::Process ret"],
    "CELT_Force": ["CELT force"],
    "CELT_DefaultMode": ["CELT default mode"],
    "OpusEncodeNative_AnalysisGate_Jmp": ["Opus analysis gate bypass"],
    "OpusEncodeFrame_FilterBypass_CopyGain": ["Opus frame copy/gain"],
    "OpusEncoder_Init_ForceCBR": ["OpusEncoder init force CBR"],
    "ApplySettings_BitrateBase_Mov510k": ["ApplySettings_BitrateBase_Mov248k"],
    "OpusEncoder_SetVBR_ForceCBR": ["OpusEncoder setVBR force CBR"],
}

ALIAS_TO_CANONICAL = {
    alias: canonical
    for canonical, aliases in SITE_ALIASES.items()
    for alias in aliases
}


# ---------------------------------------------------------------------------
# PE parsing
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Section:
    name: str
    vaddr: int
    vsize: int
    raw_offset: int
    raw_size: int
    characteristics: int

    def contains_rva(self, rva: int) -> bool:
        return self.vaddr <= rva < self.vaddr + max(self.vsize, self.raw_size)

    def contains_file(self, off: int) -> bool:
        return self.raw_offset <= off < self.raw_offset + self.raw_size


@dataclass
class PEInfo:
    path: Path
    data: bytes
    machine: int
    image_base: int
    timestamp: int
    size_of_headers: int
    sections: list[Section]
    exception_rva: int
    exception_size: int
    _functions: list[tuple[int, int]] | None = field(default=None, repr=False)

    @property
    def arch(self) -> str:
        return {
            0x014C: "x86",
            0x8664: "x86_64",
            0xAA64: "arm64",
            0x01C4: "arm",
        }.get(self.machine, f"machine_0x{self.machine:x}")

    @property
    def size(self) -> int:
        return len(self.data)

    def section_for_file(self, off: int) -> Section | None:
        for section in self.sections:
            if section.contains_file(off):
                return section
        return None

    def file_to_rva(self, off: int) -> int | None:
        if off < 0 or off >= len(self.data):
            return None
        if off < self.size_of_headers:
            return off
        section = self.section_for_file(off)
        if section is None:
            return None
        return off - section.raw_offset + section.vaddr

    def rva_to_file(self, rva: int) -> int | None:
        if rva < 0:
            return None
        if rva < self.size_of_headers:
            return rva if rva < len(self.data) else None
        for section in self.sections:
            if section.vaddr <= rva < section.vaddr + section.raw_size:
                off = rva - section.vaddr + section.raw_offset
                return off if 0 <= off < len(self.data) else None
        return None

    def functions(self) -> list[tuple[int, int]]:
        if self._functions is not None:
            return self._functions
        functions: list[tuple[int, int]] = []
        if not self.exception_rva or not self.exception_size:
            self._functions = functions
            return functions
        start = self.rva_to_file(self.exception_rva)
        if start is None:
            self._functions = functions
            return functions
        end = min(start + self.exception_size, len(self.data))
        for pos in range(start, end - 11, 12):
            begin, stop, _unwind = struct.unpack_from("<III", self.data, pos)
            if begin == 0 or stop <= begin:
                continue
            begin_off = self.rva_to_file(begin)
            stop_off = self.rva_to_file(stop)
            if begin_off is not None and stop_off is not None:
                functions.append((begin_off, stop_off))
        functions.sort()
        self._functions = functions
        return functions

    def function_for_file(self, off: int) -> tuple[int, int] | None:
        functions = self.functions()
        if not functions:
            return None
        starts = [item[0] for item in functions]
        index = bisect.bisect_right(starts, off) - 1
        if index < 0:
            return None
        begin, end = functions[index]
        return (begin, end) if begin <= off < end else None


def parse_pe(path: Path, data: bytes | None = None) -> PEInfo:
    if data is None:
        data = path.read_bytes()
    if len(data) < 0x40:
        raise ValueError(f"not a PE file: {path}")
    if data[:2] != b"MZ":
        raise ValueError(f"not a PE file (missing MZ): {path}")
    pe_off = struct.unpack_from("<I", data, 0x3C)[0]
    if pe_off + 24 > len(data) or data[pe_off : pe_off + 4] != b"PE\0\0":
        raise ValueError(f"not a PE file (missing PE signature): {path}")
    coff = pe_off + 4
    machine, nsections, timestamp, _sym_ptr, _nsyms, opt_size, _chars = (
        struct.unpack_from("<HHIIIHH", data, coff)
    )
    opt = coff + 20
    if opt + opt_size > len(data):
        raise ValueError(f"truncated optional header: {path}")
    magic = struct.unpack_from("<H", data, opt)[0]
    if magic == 0x10B:
        image_base = struct.unpack_from("<I", data, opt + 28)[0]
        dir_offset = opt + 0x60
        nrva_offset = opt + 0x5C
    elif magic == 0x20B:
        image_base = struct.unpack_from("<Q", data, opt + 24)[0]
        dir_offset = opt + 0x70
        nrva_offset = opt + 0x6C
    else:
        raise ValueError(f"unknown PE optional header magic 0x{magic:x}: {path}")
    size_of_headers = struct.unpack_from("<I", data, opt + 0x3C)[0]
    nrva = struct.unpack_from("<I", data, nrva_offset)[0]
    exception_rva = 0
    exception_size = 0
    if nrva > 3 and dir_offset + 4 * 8 <= opt + opt_size:
        exception_rva, exception_size = struct.unpack_from("<II", data, dir_offset + 3 * 8)
    section_table = opt + opt_size
    sections: list[Section] = []
    for index in range(nsections):
        pos = section_table + index * 40
        if pos + 40 > len(data):
            break
        raw_name = data[pos : pos + 8].split(b"\0", 1)[0]
        name = raw_name.decode("ascii", "replace")
        vsize, vaddr, raw_size, raw_offset = struct.unpack_from("<IIII", data, pos + 8)
        characteristics = struct.unpack_from("<I", data, pos + 36)[0]
        if raw_size == 0 or raw_offset >= len(data):
            continue
        sections.append(
            Section(
                name=name,
                vaddr=vaddr,
                vsize=vsize,
                raw_offset=raw_offset,
                raw_size=min(raw_size, len(data) - raw_offset),
                characteristics=characteristics,
            )
        )
    return PEInfo(
        path=path,
        data=data,
        machine=machine,
        image_base=image_base,
        timestamp=timestamp,
        size_of_headers=size_of_headers,
        sections=sections,
        exception_rva=exception_rva,
        exception_size=exception_size,
    )


ELF_MACHINE_TO_PE = {0x3E: 0x8664, 0x03: 0x014C, 0xB7: 0xAA64, 0x28: 0x01C4}


def parse_elf(path: Path, data: bytes | None = None) -> PEInfo:
    """Header-only ELF support: offset tables use raw file offsets, so the
    relocation scan needs no section/function metadata (PE extras degrade to
    no-ops)."""
    if data is None:
        data = path.read_bytes()
    if len(data) < 64 or data[:4] != b"\x7fELF":
        raise ValueError(f"not an ELF file: {path}")
    if data[4] != 2:
        raise ValueError(f"unsupported ELF class (only 64-bit): {path}")
    endian = "<" if data[5] == 1 else ">"
    machine = struct.unpack_from(f"{endian}H", data, 18)[0]
    return PEInfo(
        path=path,
        data=data,
        machine=ELF_MACHINE_TO_PE.get(machine, 0x8664),
        image_base=0,
        timestamp=0,
        size_of_headers=0,
        sections=[],
        exception_rva=0,
        exception_size=0,
    )


def parse_binary(path: Path, data: bytes | None = None) -> PEInfo:
    raw = data if data is not None else path.read_bytes()
    if raw[:4] == b"\x7fELF":
        return parse_elf(path, raw)
    if raw[:2] == b"MZ":
        return parse_pe(path, raw)
    raise ValueError(f"unsupported binary format: {path}")


def binary_os(pe: PEInfo) -> str:
    return "linux" if pe.data[:4] == b"\x7fELF" else "windows"


# ---------------------------------------------------------------------------
# AOB patterns
# ---------------------------------------------------------------------------


def md5_hex(data: bytes) -> str:
    return hashlib.md5(data).hexdigest()


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse_aob(text: str) -> list[int | None]:
    tokens = re.split(r"[\s,]+", text.strip())
    values: list[int | None] = []
    for token in tokens:
        if not token:
            continue
        if token in {"??", "?", "..", "**"}:
            values.append(None)
            continue
        if re.fullmatch(r"[0-9a-fA-F]{2}", token):
            values.append(int(token, 16))
            continue
        if re.fullmatch(r"[0-9a-fA-F]+", token) and len(token) % 2 == 0:
            values.extend(int(token[pos : pos + 2], 16) for pos in range(0, len(token), 2))
            continue
        raise ValueError(f"invalid AOB token {token!r}")
    if not values:
        raise ValueError("empty AOB pattern")
    return values


def aob_text(values: Sequence[int | None]) -> str:
    return " ".join("??" if value is None else f"{value:02x}" for value in values)


def aob_regex(values: Sequence[int | None]) -> re.Pattern[bytes]:
    parts: list[bytes] = []
    for value in values:
        parts.append(b"." if value is None else re.escape(bytes([value])))
    return re.compile(b"".join(parts), re.DOTALL)


def fixed_count(values: Sequence[int | None]) -> int:
    return sum(1 for value in values if value is not None)


def scan_aob(data: bytes, values: Sequence[int | None], limit: int = MAX_PATTERN_HITS) -> list[int]:
    if not any(value is None for value in values):
        needle = bytes(value for value in values if value is not None)
        hits: list[int] = []
        start = 0
        while len(hits) < limit:
            found = data.find(needle, start)
            if found < 0:
                break
            hits.append(found)
            start = found + 1
        return hits
    regex = aob_regex(values)
    hits = []
    for match in regex.finditer(data):
        hits.append(match.start())
        if len(hits) >= limit:
            break
    return hits


def find_all(data: bytes, needle: bytes, limit: int = MAX_PATTERN_HITS) -> list[int]:
    hits: list[int] = []
    start = 0
    while len(hits) < limit:
        found = data.find(needle, start)
        if found < 0:
            break
        hits.append(found)
        start = found + 1
    return hits


# ---------------------------------------------------------------------------
# Tables
# ---------------------------------------------------------------------------


def hex_bytes(value: str, label: str) -> bytes:
    text = value.strip()
    if len(text) % 2 != 0 or not re.fullmatch(r"[0-9a-fA-F]*", text):
        raise ValueError(f"{label}: invalid hex {value!r}")
    return bytes.fromhex(text)


@dataclass(frozen=True)
class TableSite:
    name: str
    kind: str
    offset: int | None
    orig: bytes
    patch: bytes
    pattern: str | None = None
    expect: bytes | None = None

    @classmethod
    def from_dict(cls, raw: dict) -> "TableSite":
        name = str(raw.get("name", "")).strip()
        if not name:
            raise ValueError("table site missing name")
        kind = str(raw.get("kind", "offset"))
        offset: int | None = None
        if "offset" in raw and raw["offset"] is not None:
            value = raw["offset"]
            offset = int(value, 16) if isinstance(value, str) else int(value)
        orig = hex_bytes(str(raw.get("orig", "")), f"{name}.orig")
        patch = hex_bytes(str(raw.get("patch", "")), f"{name}.patch")
        pattern = raw.get("pattern")
        expect_raw = raw.get("expect")
        expect = hex_bytes(str(expect_raw), f"{name}.expect") if expect_raw else None
        if kind == "offset":
            if offset is None:
                raise ValueError(f"{name}: offset site missing offset")
            if len(orig) != len(patch):
                raise ValueError(f"{name}: orig and patch length differ")
        return cls(
            name=name,
            kind=kind,
            offset=offset,
            orig=orig,
            patch=patch,
            pattern=str(pattern) if pattern else None,
            expect=expect,
        )


@dataclass
class Table:
    goal: str
    binary_md5: str
    discord: str
    os: str
    arch: str
    sites: list[TableSite]
    raw: dict


@dataclass
class PatchDb:
    schema: int
    db_version: int
    updated: str
    goals: list[dict]
    tables: list[Table]

    def goal_title(self, goal: str, lang: str = "en") -> str:
        for item in self.goals:
            if item.get("id") == goal:
                title = item.get("title", {})
                return str(title.get(lang) or title.get("en") or goal)
        return goal


def load_db(path: Path) -> PatchDb:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise ValueError(f"{path}: database root must be an object")
    tables: list[Table] = []
    for entry in raw.get("tables", []):
        if not isinstance(entry, dict):
            continue
        tables.append(
            Table(
                goal=str(entry.get("goal", "")),
                binary_md5=str(entry.get("binary_md5", "")).lower(),
                discord=str(entry.get("discord") or ""),
                os=str(entry.get("os") or ""),
                arch=str(entry.get("arch") or ""),
                sites=[TableSite.from_dict(site) for site in entry.get("sites", [])],
                raw=entry,
            )
        )
    return PatchDb(
        schema=int(raw.get("schema", 0)),
        db_version=int(raw.get("db_version", 0)),
        updated=str(raw.get("updated", "")),
        goals=list(raw.get("goals", [])),
        tables=tables,
    )


def version_key(discord: str) -> tuple[int, ...]:
    match = re.search(r"app-1\.0\.(\d+)", discord)
    return (int(match.group(1)),) if match else (0,)


def app_key(text: str) -> tuple[int, int, int] | None:
    match = re.search(r"app-(\d+)\.(\d+)\.(\d+)", text)
    return tuple(int(part) for part in match.groups()) if match else None


def node_os(path: Path) -> str:
    """Return "linux", "windows", or "" from the binary magic bytes."""
    try:
        with open(path, "rb") as handle:
            magic = handle.read(4)
    except OSError:
        return ""
    if magic == b"\x7fELF":
        return "linux"
    if magic[:2] == b"MZ":
        return "windows"
    return ""


def choose_reference(
    db: PatchDb, goal: str, node_path: Path, md5: str, ref_md5: str | None = None
) -> Table:
    """Pick a reference table; an explicit ref_md5 wins, otherwise the closest build on the node's platform."""
    candidates = [table for table in db.tables if table.goal == goal and table.sites]
    if not candidates:
        raise ValueError(f"no tables for goal {goal!r}")
    if ref_md5:
        wanted = ref_md5.lower()
        for table in db.tables:
            if table.goal == goal and table.binary_md5 == wanted and table.sites:
                return table
        raise ValueError(f"no {goal} table for md5 {wanted}")
    target_os = node_os(node_path)
    if target_os:
        same_os = [table for table in candidates if not table.os or table.os == target_os]
        if same_os:
            candidates = same_os
    for table in candidates:
        if table.binary_md5 == md5:
            return table
    node_version = app_key(str(node_path))
    if node_version is None:
        return max(candidates, key=lambda table: (version_key(table.discord), len(table.sites)))

    def rank(entry: tuple[int, Table]) -> tuple[tuple[int, int, int], int, int]:
        index, table = entry
        table_version = app_key(table.discord)
        distance = (
            tuple(abs(a - b) for a, b in zip(table_version, node_version))
            if table_version
            else (10**9, 10**9, 10**9)
        )
        return (distance, -len(table.sites), -index)

    return min(enumerate(candidates), key=rank)[1]


def find_table(db: PatchDb, md5: str, goal: str) -> Table | None:
    for table in db.tables:
        if table.goal == goal and table.binary_md5 == md5.lower():
            return table
    return None


# ---------------------------------------------------------------------------
# Profiles
# ---------------------------------------------------------------------------


@dataclass
class ProfileSite:
    name: str
    aliases: list[str]
    pattern: str
    target_offset: int
    expected: bytes
    patch: bytes
    source_md5: str | None
    context: int
    kind: str
    sources: list[str]
    fixed: int
    unique: bool

    @classmethod
    def from_dict(cls, raw: dict) -> "ProfileSite":
        name = str(raw.get("name", "")).strip()
        pattern = str(raw.get("pattern", "")).strip()
        if not name or not pattern:
            raise ValueError("profile site missing name/pattern")
        target_offset = int(raw.get("target_offset", 0))
        expected = hex_bytes(str(raw.get("expected", "")), f"{name}.expected")
        patch = hex_bytes(str(raw.get("patch", "")), f"{name}.patch")
        values = parse_aob(pattern)
        if target_offset < 0 or target_offset + len(expected) > len(values):
            raise ValueError(f"{name}: target_offset outside pattern")
        return cls(
            name=name,
            aliases=[str(item) for item in raw.get("aliases", [])],
            pattern=pattern,
            target_offset=target_offset,
            expected=expected,
            patch=patch,
            source_md5=str(raw["source_md5"]).lower() if raw.get("source_md5") else None,
            context=int(raw.get("context", 0)),
            kind=str(raw.get("kind", "exact")),
            sources=[str(item).lower() for item in raw.get("sources", [])],
            fixed=int(raw.get("fixed", fixed_count(values))),
            unique=bool(raw.get("unique", True)),
        )


@dataclass
class Profile:
    id: str
    kind: str
    source_md5: str | None
    discord: str
    sites: list[ProfileSite]


def load_profiles(path: Path) -> list[Profile]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise ValueError(f"{path}: profile root must be an object")
    if raw.get("schema") != PROFILE_SCHEMA:
        raise ValueError(f"{path}: unsupported profile schema {raw.get('schema')!r}")
    profiles: list[Profile] = []
    for entry in raw.get("profiles", []):
        profiles.append(
            Profile(
                id=str(entry.get("id", "profile")),
                kind=str(entry.get("kind", "exact")),
                source_md5=str(entry["source_md5"]).lower() if entry.get("source_md5") else None,
                discord=str(entry.get("discord") or ""),
                sites=[ProfileSite.from_dict(site) for site in entry.get("sites", [])],
            )
        )
    return profiles


def profile_priority(site: ProfileSite, ref_md5: str) -> int:
    if site.kind == "consensus":
        return 3
    if site.source_md5 and site.source_md5 == ref_md5:
        return 2
    return 1


def profile_lookup(profiles: Sequence[Profile], name: str, ref_md5: str) -> list[ProfileSite]:
    canonical = ALIAS_TO_CANONICAL.get(name, name)
    found: dict[tuple, ProfileSite] = {}
    for profile in profiles:
        for site in profile.sites:
            names = {site.name, *site.aliases}
            if name not in names and canonical not in names:
                continue
            key = (site.pattern, site.target_offset, site.expected, site.patch)
            current = found.get(key)
            if current is None or profile_priority(site, ref_md5) > profile_priority(current, ref_md5):
                found[key] = site
    return sorted(
        found.values(),
        key=lambda site: (profile_priority(site, ref_md5), site.fixed, site.context),
        reverse=True,
    )


# ---------------------------------------------------------------------------
# Profile learning
# ---------------------------------------------------------------------------


def build_window_pattern(
    data: bytes,
    offset: int,
    length: int,
    context: int,
    wildcard_ranges: Sequence[tuple[int, int]],
    consensus_windows: Sequence[bytes] | None = None,
) -> tuple[list[int | None], int, int]:
    start = max(0, offset - context)
    end = min(len(data), offset + length + context)
    if consensus_windows is not None:
        window_length = end - start
        for window in consensus_windows:
            if len(window) != window_length:
                raise ValueError("consensus window length mismatch")
    values: list[int | None] = []
    for position in range(start, end):
        if any(begin <= position < stop for begin, stop in wildcard_ranges):
            values.append(None)
            continue
        if consensus_windows is None:
            values.append(data[position])
            continue
        sample = [window[position - start] for window in consensus_windows]
        values.append(sample[0] if all(byte == sample[0] for byte in sample) else None)
    return values, offset - start, start


def learn_profiles(
    samples: Sequence[tuple[str, Path]],
    db: PatchDb,
    goal: str,
    min_context: int = MIN_CONTEXT,
    max_context: int = MAX_CONTEXT,
) -> dict:
    build_profiles: list[dict] = []
    sample_data: list[tuple[str, bytes, Table, list[tuple[int, int]]]] = []
    for md5, path in samples:
        data = path.read_bytes()
        actual = md5_hex(data)
        if actual != md5.lower():
            raise ValueError(f"{path}: md5 {actual} does not match requested {md5}")
        table = find_table(db, actual, goal)
        if table is None:
            raise ValueError(f"{path}: no {goal} table for md5 {actual}")
        ranges: list[tuple[int, int]] = []
        for site in table.sites:
            if site.offset is None:
                continue
            if site.offset + len(site.orig) > len(data):
                raise ValueError(f"{table.discord} {site.name}: offset outside file")
            found = data[site.offset : site.offset + len(site.orig)]
            if found != site.orig:
                raise ValueError(
                    f"{table.discord} {site.name}: expected clean bytes {site.orig.hex()}, "
                    f"found {found.hex()} at 0x{site.offset:x}"
                )
            ranges.append((site.offset, site.offset + len(site.orig)))
        sample_data.append((actual, data, table, ranges))

    for md5, data, table, ranges in sample_data:
        entries: list[dict] = []
        for site in table.sites:
            if site.offset is None:
                continue
            chosen: tuple[list[int | None], int, int, int, bool] | None = None
            for context in range(min_context, max_context + 1, 8):
                values, target_offset, start = build_window_pattern(
                    data, site.offset, len(site.orig), context, ranges
                )
                hits = scan_aob(data, values)
                if len(hits) == 1 and hits[0] == start:
                    chosen = (values, target_offset, context, start, True)
                    break
            if chosen is None:
                context = max_context
                values, target_offset, start = build_window_pattern(
                    data, site.offset, len(site.orig), context, ranges
                )
                hits = scan_aob(data, values)
                chosen = (values, target_offset, context, start, len(hits) == 1 and hits[0] == start)
            values, target_offset, context, _start, unique = chosen
            entries.append(
                {
                    "name": site.name,
                    "aliases": [],
                    "pattern": aob_text(values),
                    "target_offset": target_offset,
                    "expected": site.orig.hex(),
                    "patch": site.patch.hex(),
                    "source_md5": md5,
                    "context": context,
                    "kind": "exact",
                    "sources": [md5],
                    "fixed": fixed_count(values),
                    "unique": unique,
                }
            )
        build_profiles.append(
            {
                "id": f"build-{md5}",
                "kind": "exact",
                "source_md5": md5,
                "discord": table.discord,
                "sites": entries,
            }
        )

    by_md5 = {md5: (data, table) for md5, data, table, _ranges in sample_data}
    consensus_entries: list[dict] = []
    for canonical, aliases in SITE_ALIASES.items():
        group: list[tuple[str, bytes, Table, TableSite]] = []
        for md5, data, table in (
            (md5, data, table) for md5, (data, table) in by_md5.items()
        ):
            match = None
            for site in table.sites:
                if site.name == canonical or site.name in aliases:
                    match = site
                    break
            if match is not None:
                group.append((md5, data, table, match))
        if len(group) < 2:
            continue
        expected = group[0][3].orig
        patch = group[0][3].patch
        if any(item[3].orig != expected or item[3].patch != patch for item in group):
            continue
        chosen: tuple[list[int | None], int, int, bool] | None = None
        for context in range(min_context, max_context + 1, 8):
            windows: list[bytes] = []
            ranges: list[tuple[int, int]] = []
            starts: list[int] = []
            for _md5, data, table, site in group:
                assert site.offset is not None
                start = max(0, site.offset - context)
                end = min(len(data), site.offset + len(site.orig) + context)
                windows.append(data[start:end])
                starts.append(start)
                ranges.extend(
                    (item.offset, item.offset + len(item.orig))
                    for item in table.sites
                    if item.offset is not None
                )
            if len({len(window) for window in windows}) != 1:
                continue
            reference = group[0]
            assert reference[3].offset is not None
            values, target_offset, _start = build_window_pattern(
                reference[1],
                reference[3].offset,
                len(expected),
                context,
                ranges,
                consensus_windows=windows,
            )
            unique = True
            for (_md5, data, _table, site), window in zip(group, windows):
                assert site.offset is not None
                start = site.offset - context
                if start < 0:
                    unique = False
                    break
                hits = scan_aob(data, values)
                if len(hits) != 1 or hits[0] != start:
                    unique = False
                    break
            if unique:
                chosen = (values, target_offset, context, True)
                break
        if chosen is None:
            continue
        values, target_offset, context, unique = chosen
        consensus_entries.append(
            {
                "name": canonical,
                "aliases": aliases,
                "pattern": aob_text(values),
                "target_offset": target_offset,
                "expected": expected.hex(),
                "patch": patch.hex(),
                "source_md5": None,
                "context": context,
                "kind": "consensus",
                "sources": [item[0] for item in group],
                "fixed": fixed_count(values),
                "unique": unique,
            }
        )

    profiles: list[dict] = []
    if consensus_entries:
        profiles.append(
            {
                "id": "consensus",
                "kind": "consensus",
                "source_md5": None,
                "discord": "",
                "sites": consensus_entries,
            }
        )
    profiles.extend(build_profiles)
    return {
        "schema": PROFILE_SCHEMA,
        "tool": "stereo-wizard-offset-finder",
        "version": VERSION,
        "generated": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "goal": goal,
        "min_context": min_context,
        "aliases": SITE_ALIASES,
        "profiles": profiles,
    }


# ---------------------------------------------------------------------------
# Scanning
# ---------------------------------------------------------------------------


@dataclass
class Candidate:
    offset: int
    found: bytes
    score: int
    method: str
    profile: str
    kind: str
    pattern: str
    target_offset: int
    hits: int
    fixed: int
    valid: bool
    patched: bool


@dataclass
class SiteResult:
    site: TableSite
    status: str
    new_offset: int | None
    found: bytes
    unique: bool
    method: str
    score: int
    hits: int
    candidates: list[Candidate] = field(default_factory=list)
    note: str = ""

    @property
    def ok(self) -> bool:
        return self.status == "ok" and self.unique and self.new_offset is not None


def score_candidate(
    pe: PEInfo,
    site: TableSite,
    entry: ProfileSite,
    hit_start: int,
    hit_count: int,
) -> Candidate | None:
    target = hit_start + entry.target_offset
    if target < 0 or target + len(site.orig) > len(pe.data):
        return None
    found = pe.data[target : target + len(site.orig)]
    valid = found == site.orig
    patched = found == site.patch
    base = {"consensus": 140, "exact": 90, "orig": 50}.get(entry.kind, 60)
    if entry.kind == "exact" and entry.source_md5 and entry.source_md5 == md5_hex(pe.data):
        base = 120
    score = base + min(entry.fixed, 128) // 8
    section = pe.section_for_file(target)
    if section is not None:
        if section.name == ".text":
            score += 15
        elif section.name in {".rdata", ".data"}:
            score += 5
    if pe.function_for_file(target) is not None:
        score += 10
    if valid:
        score += 40
    elif patched:
        score += 5
    else:
        score -= 250
    if hit_count > 1:
        score -= min(120, 15 * (hit_count - 1))
    return Candidate(
        offset=target,
        found=found,
        score=score,
        method=entry.kind,
        profile=entry.source_md5 or entry.kind,
        kind=entry.kind,
        pattern=entry.pattern,
        target_offset=entry.target_offset,
        hits=hit_count,
        fixed=entry.fixed,
        valid=valid,
        patched=patched,
    )


def scan_site(
    pe: PEInfo,
    site: TableSite,
    entries: Sequence[ProfileSite],
    ref_md5: str,
) -> tuple[list[Candidate], list[str]]:
    candidates: list[Candidate] = []
    misses: list[str] = []
    for entry in entries:
        try:
            values = parse_aob(entry.pattern)
        except ValueError as exc:
            misses.append(f"{entry.pattern[:24]}: {exc}")
            continue
        hits = scan_aob(pe.data, values)
        if not hits:
            misses.append(f"{entry.kind}: no hit")
            continue
        if len(hits) > MAX_HITS:
            misses.append(f"{entry.kind}: too many hits ({len(hits)})")
            continue
        for hit in hits:
            candidate = score_candidate(pe, site, entry, hit, len(hits))
            if candidate is not None:
                candidates.append(candidate)
    if not entries and site.kind == "offset" and site.offset is not None:
        hits = find_all(pe.data, site.orig)
        for hit in hits[:MAX_HITS]:
            entry = ProfileSite(
                name=site.name,
                aliases=[],
                pattern=site.orig.hex(),
                target_offset=0,
                expected=site.orig,
                patch=site.patch,
                source_md5=None,
                context=0,
                kind="orig",
                sources=[],
                fixed=len(site.orig),
                unique=len(hits) == 1,
            )
            candidate = score_candidate(pe, site, entry, hit, len(hits))
            if candidate is not None:
                candidates.append(candidate)
    deduped: dict[int, Candidate] = {}
    for candidate in candidates:
        current = deduped.get(candidate.offset)
        if current is None or candidate.score > current.score:
            deduped[candidate.offset] = candidate
        else:
            current.hits = max(current.hits, candidate.hits)
    return sorted(deduped.values(), key=lambda item: item.score, reverse=True), misses


def reference_result(pe: PEInfo, site: TableSite, method: str = "table") -> SiteResult:
    assert site.offset is not None
    found = pe.data[site.offset : site.offset + len(site.orig)]
    if found == site.orig:
        return SiteResult(site, "ok", site.offset, found, True, method, 999, 1)
    if found == site.patch:
        return SiteResult(site, "patched", site.offset, found, True, method, 999, 1)
    return SiteResult(
        site,
        "mismatch",
        None,
        found,
        False,
        method,
        0,
        0,
        note="bytes at reference offset match neither orig nor patch",
    )


def layout_consistency(
    site: TableSite,
    candidate: Candidate,
    anchors: Sequence[SiteResult],
) -> float:
    if site.offset is None:
        return 0.0
    score = 0.0
    for anchor in anchors:
        if anchor.site.offset is None or anchor.new_offset is None:
            continue
        expected_delta = site.offset - anchor.site.offset
        actual_delta = candidate.offset - anchor.new_offset
        difference = abs(expected_delta - actual_delta)
        if difference == 0:
            score += 2.0
        elif difference <= 0x20:
            score += 0.5
        else:
            score -= 1.0
    return score


def relative_estimates(
    pe: PEInfo,
    site: TableSite,
    anchors: Sequence[SiteResult],
) -> list[tuple[int, str]]:
    if site.offset is None:
        return []
    estimates: dict[int, str] = {}
    for anchor in anchors:
        if anchor.site.offset is None or anchor.new_offset is None:
            continue
        estimate = anchor.new_offset + (site.offset - anchor.site.offset)
        if estimate < 0 or estimate + len(site.orig) > len(pe.data):
            continue
        found = pe.data[estimate : estimate + len(site.orig)]
        if found == site.orig:
            estimates[estimate] = "relative-orig"
        elif found == site.patch:
            estimates[estimate] = "relative-patched"
    return [(offset, method) for offset, method in estimates.items()]


def scan_binary(
    pe: PEInfo,
    ref: Table,
    profiles: Sequence[Profile],
    force_scan: bool = False,
) -> list[SiteResult]:
    md5 = md5_hex(pe.data)
    if md5 == ref.binary_md5 and not force_scan:
        return [reference_result(pe, site) for site in ref.sites]

    results: list[SiteResult] = []
    raw_candidates: dict[str, list[Candidate]] = {}
    misses: dict[str, list[str]] = {}
    for site in ref.sites:
        if site.kind != "offset" or site.offset is None:
            results.append(
                SiteResult(site, "missing", None, b"", False, "none", 0, 0, note="pattern-only site")
            )
            continue
        entries = profile_lookup(profiles, site.name, ref.binary_md5)
        candidates, site_misses = scan_site(pe, site, entries, ref.binary_md5)
        raw_candidates[site.name] = candidates
        misses[site.name] = site_misses
        valid = [candidate for candidate in candidates if candidate.valid]
        patched = [candidate for candidate in candidates if candidate.patched]
        if len(valid) == 1:
            candidate = valid[0]
            results.append(
                SiteResult(
                    site,
                    "ok",
                    candidate.offset,
                    candidate.found,
                    True,
                    candidate.method,
                    candidate.score,
                    candidate.hits,
                    candidates,
                )
            )
        elif len(valid) > 1:
            results.append(
                SiteResult(
                    site,
                    "ambiguous",
                    None,
                    valid[0].found,
                    False,
                    "ambiguous",
                    valid[0].score,
                    valid[0].hits,
                    candidates,
                    note=f"{len(valid)} verified candidates",
                )
            )
        elif len(patched) == 1:
            candidate = patched[0]
            results.append(
                SiteResult(
                    site,
                    "patched",
                    candidate.offset,
                    candidate.found,
                    True,
                    candidate.method,
                    candidate.score,
                    candidate.hits,
                    candidates,
                )
            )
        elif candidates:
            results.append(
                SiteResult(
                    site,
                    "mismatch",
                    None,
                    candidates[0].found,
                    False,
                    "mismatch",
                    candidates[0].score,
                    candidates[0].hits,
                    candidates,
                    note="candidate found but bytes match neither orig nor patch",
                )
            )
        else:
            results.append(
                SiteResult(
                    site,
                    "missing",
                    None,
                    b"",
                    False,
                    "none",
                    0,
                    0,
                    candidates,
                    note="; ".join(site_misses[:3]),
                )
            )

    anchors = [result for result in results if result.ok]
    for result in results:
        if result.status in {"ok", "patched"}:
            continue
        candidates = [candidate for candidate in result.candidates if candidate.valid]
        if candidates:
            ranked = sorted(
                ((layout_consistency(result.site, candidate, anchors), candidate) for candidate in candidates),
                key=lambda item: item[0],
                reverse=True,
            )
            best_score, best = ranked[0]
            second_score = ranked[1][0] if len(ranked) > 1 else float("-inf")
            if best_score > 0 and (len(ranked) == 1 or best_score - second_score >= 1.5):
                result.status = "ok"
                result.new_offset = best.offset
                result.found = best.found
                result.unique = True
                result.method = "pattern+layout"
                result.score = best.score
                result.hits = best.hits
                result.note = f"layout consistency {best_score:.1f}"
                anchors.append(result)
                continue
        if result.site.offset is None:
            continue
        estimates = relative_estimates(pe, result.site, anchors)
        if len(estimates) == 1:
            offset, method = estimates[0]
            found = pe.data[offset : offset + len(result.site.orig)]
            result.new_offset = offset
            result.found = found
            result.unique = True
            result.method = method
            result.score = 70
            result.status = "ok" if found == result.site.orig else "patched"
            result.note = "single relative anchor"
            anchors.append(result)
            continue
        if len(estimates) > 1:
            result.status = "ambiguous"
            result.note = f"{len(estimates)} relative estimates"
            continue
        if len(result.site.orig) >= 4:
            hits = find_all(pe.data, result.site.orig)
            if len(hits) == 1:
                offset = hits[0]
                result.new_offset = offset
                result.found = result.site.orig
                result.unique = True
                result.method = "orig-unique"
                result.score = 50
                result.status = "ok"
                result.note = "unique original-byte search"
                anchors.append(result)

    for result in results:
        if result.status in {"ok", "patched"}:
            continue
        valid = [candidate for candidate in result.candidates if candidate.valid]
        if len(valid) == 1:
            candidate = valid[0]
            result.status = "ok"
            result.new_offset = candidate.offset
            result.found = candidate.found
            result.unique = True
            result.method = candidate.method
            result.score = candidate.score
            result.hits = candidate.hits
    return results


# ---------------------------------------------------------------------------
# Reports, apply, diff
# ---------------------------------------------------------------------------


def redact_path(path: Path) -> str:
    text = str(path)
    home = os.environ.get("HOME", "")
    if home and text.startswith(home):
        return "~" + text[len(home) :]
    return text


def discord_dir_from_path(path: Path) -> str:
    for part in path.parts:
        if re.fullmatch(r"app-1\.0\.\d+", part):
            return part
    return ""


CLIENT_DIR_KINDS = {
    "discord": "stable",
    "discordptb": "ptb",
    "discordcanary": "canary",
    "vencord": "vencord",
    "equicord": "vencord",
}


def client_kind_from_path(path: Path) -> str:
    """Return the client kind from recognized install-directory names."""
    for part in reversed(path.parts):
        kind = CLIENT_DIR_KINDS.get(part.lower())
        if kind is not None:
            return kind
    return "stable"


def neighborhood(pe: PEInfo, offset: int | None) -> str | None:
    if offset is None or offset < 16 or offset + 16 > len(pe.data):
        return None
    return pe.data[offset - 16 : offset + 16].hex()


def build_report(
    pe: PEInfo,
    ref: Table,
    db: PatchDb,
    results: Sequence[SiteResult],
    goal: str,
    lang: str = "en",
) -> dict:
    ts = int(time.time())
    node_md5 = md5_hex(pe.data)
    sites = []
    for result in results:
        state = "ok" if result.status == "ok" else "patched" if result.status == "patched" else "mismatch"
        sites.append(
            {
                "name": result.site.name,
                "state": state,
                "expected_orig": result.site.orig.hex(),
                "expected_patch": result.site.patch.hex(),
                "found": result.found.hex() if result.found else "",
                "neighborhood": neighborhood(pe, result.new_offset or result.site.offset),
                "offset": result.site.offset,
                "new_offset": result.new_offset if state == "ok" else None,
                "unique": bool(result.unique and state == "ok"),
                "method": result.method,
                "score": result.score,
                "hits": result.hits,
                "note": result.note,
            }
        )
    return {
        "app_version": VERSION,
        "schema": REPORT_SCHEMA,
        "ts_unix": ts,
        "ts_iso": datetime.fromtimestamp(ts, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "report_id": f"{ts}-{node_md5[:12]}",
        "os": binary_os(pe),
        "arch": pe.arch,
        "goal": goal,
        "goal_title": db.goal_title(goal, lang),
        "lang": lang,
        "node_path": redact_path(pe.path),
        "node_md5": node_md5,
        "node_size": len(pe.data),
        "discord_dir": discord_dir_from_path(pe.path),
        "client_kind": client_kind_from_path(pe.path),
        "db_version": db.db_version,
        "table_kind": "matched" if node_md5 == ref.binary_md5 else "none",
        "source_md5": ref.binary_md5,
        "sites": sites,
    }


def build_scan_json(
    pe: PEInfo,
    ref: Table,
    results: Sequence[SiteResult],
    goal: str,
) -> dict:
    return {
        "schema": 1,
        "tool": "stereo-wizard-offset-finder",
        "version": VERSION,
        "goal": goal,
        "node": {
            "path": redact_path(pe.path),
            "md5": md5_hex(pe.data),
            "sha256": sha256_hex(pe.data),
            "size": len(pe.data),
            "arch": pe.arch,
            "image_base": f"0x{pe.image_base:x}",
            "timestamp": pe.timestamp,
            "sections": [
                {
                    "name": section.name,
                    "vaddr": f"0x{section.vaddr:x}",
                    "raw_offset": f"0x{section.raw_offset:x}",
                    "raw_size": section.raw_size,
                }
                for section in pe.sections
            ],
        },
        "reference": {
            "md5": ref.binary_md5,
            "discord": ref.discord,
            "goal": ref.goal,
            "sites": len(ref.sites),
        },
        "sites": [
            {
                "name": result.site.name,
                "status": result.status,
                "offset": result.site.offset,
                "rva": pe.file_to_rva(result.new_offset) if result.new_offset is not None else None,
                "new_offset": result.new_offset,
                "method": result.method,
                "score": result.score,
                "hits": result.hits,
                "unique": result.unique,
                "expected_orig": result.site.orig.hex(),
                "expected_patch": result.site.patch.hex(),
                "found": result.found.hex() if result.found else "",
                "note": result.note,
            }
            for result in results
        ],
        "summary": {
            "sites": len(results),
            "ok": sum(1 for result in results if result.status == "ok"),
            "patched": sum(1 for result in results if result.status == "patched"),
            "ambiguous": sum(1 for result in results if result.status == "ambiguous"),
            "missing": sum(1 for result in results if result.status == "missing"),
            "mismatch": sum(1 for result in results if result.status == "mismatch"),
        },
    }


def apply_results(
    pe: PEInfo,
    results: Sequence[SiteResult],
    force: bool = False,
) -> tuple[bytes, list[str]]:
    output = bytearray(pe.data)
    skipped: list[str] = []
    for result in results:
        if not result.ok:
            if force:
                skipped.append(result.site.name)
                continue
            raise ValueError(f"{result.site.name}: not resolved uniquely ({result.status})")
        assert result.new_offset is not None
        offset = result.new_offset
        current = bytes(output[offset : offset + len(result.site.orig)])
        if current == result.site.patch:
            continue
        if current != result.site.orig:
            raise ValueError(
                f"{result.site.name}: bytes changed after scan at 0x{offset:x}: {current.hex()}"
            )
        output[offset : offset + len(result.site.patch)] = result.site.patch
    return bytes(output), skipped


def diff_binaries(clean: bytes, patched: bytes, gap: int = 8) -> list[dict]:
    limit = min(len(clean), len(patched))
    ranges: list[tuple[int, int]] = []
    start: int | None = None
    last = -1
    for index in range(limit):
        if clean[index] != patched[index]:
            if start is None:
                start = index
            elif index - last > gap:
                ranges.append((start, last + 1))
                start = index
            last = index
    if start is not None:
        ranges.append((start, last + 1))
    if len(clean) != len(patched):
        ranges.append((limit, max(len(clean), len(patched))))
    return [
        {
            "offset": begin,
            "length": end - begin,
            "clean": clean[begin:end].hex(),
            "patched": patched[begin:end].hex(),
        }
        for begin, end in ranges
    ]


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def print_scan(pe: PEInfo, ref: Table, results: Sequence[SiteResult]) -> None:
    print(f"file:      {pe.path}")
    print(f"md5:       {md5_hex(pe.data)}")
    print(f"pe:        {pe.arch} image_base=0x{pe.image_base:x} sections={len(pe.sections)}")
    print(f"reference: {ref.goal} {ref.discord or '(unknown)'} {ref.binary_md5}")
    print(f"sites:     {len(results)}")
    print()
    header = f"{'STATUS':9} {'OFFSET':10} {'RVA':10} {'METHOD':14} {'SCORE':5} NAME"
    print(header)
    print("-" * len(header))
    for result in results:
        offset = f"0x{result.new_offset:08x}" if result.new_offset is not None else "-"
        rva_value = pe.file_to_rva(result.new_offset) if result.new_offset is not None else None
        rva = f"0x{rva_value:08x}" if rva_value is not None else "-"
        print(
            f"{result.status:9} {offset:10} {rva:10} {result.method:14} "
            f"{result.score:5} {result.site.name}"
        )
        if result.note:
            print(f"{'':9} note: {result.note}")
    ok = sum(1 for result in results if result.status == "ok")
    patched = sum(1 for result in results if result.status == "patched")
    ambiguous = sum(1 for result in results if result.status == "ambiguous")
    missing = sum(1 for result in results if result.status == "missing")
    mismatch = sum(1 for result in results if result.status == "mismatch")
    print()
    print(
        f"summary: {ok}/{len(results)} ok, {patched} patched, "
        f"{ambiguous} ambiguous, {missing} missing, {mismatch} mismatch"
    )


def parse_sample(text: str) -> tuple[str, Path]:
    if "=" not in text:
        raise argparse.ArgumentTypeError("sample must be <md5>=<path>")
    md5, path = text.split("=", 1)
    md5 = md5.strip().lower()
    if not re.fullmatch(r"[0-9a-f]{32}", md5):
        raise argparse.ArgumentTypeError(f"invalid md5: {md5!r}")
    return md5, Path(path).expanduser()


def parse_md5_pair(text: str) -> tuple[str, str]:
    if "=" not in text:
        raise argparse.ArgumentTypeError("expected patched value must be <source-md5>=<patched-md5>")
    source, patched = (part.strip().lower() for part in text.split("=", 1))
    if not re.fullmatch(r"[0-9a-f]{32}", source) or not re.fullmatch(r"[0-9a-f]{32}", patched):
        raise argparse.ArgumentTypeError(f"invalid md5 pair: {text!r}")
    return source, patched


def cmd_scan(args: argparse.Namespace) -> int:
    node = Path(args.node).expanduser()
    data = node.read_bytes()
    pe = parse_binary(node, data)
    db = load_db(Path(args.tables))
    md5 = md5_hex(data)
    ref = choose_reference(db, args.goal, node, md5, args.ref_md5)
    profiles = load_profiles(Path(args.profiles))
    results = scan_binary(pe, ref, profiles, force_scan=args.force_scan)
    if not args.quiet:
        print_scan(pe, ref, results)
    if args.json:
        Path(args.json).write_text(
            json.dumps(build_scan_json(pe, ref, results, args.goal), indent=2) + "\n",
            encoding="utf-8",
        )
        if not args.quiet:
            print(f"scan json: {args.json}")
    if args.report:
        report = build_report(pe, ref, db, results, args.goal, args.lang)
        Path(args.report).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        if not args.quiet:
            print(f"report:    {args.report}")
    exit_code = 0
    if any(not result.ok for result in results):
        exit_code = 3
    if args.apply:
        try:
            output, skipped = apply_results(pe, results, force=args.force)
        except ValueError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 2
        out_path = Path(args.apply).expanduser()
        if out_path.resolve() == node.resolve():
            print("error: --apply must not overwrite the input", file=sys.stderr)
            return 2
        if out_path.exists() and not args.overwrite:
            print(f"error: output exists: {out_path} (use --overwrite)", file=sys.stderr)
            return 2
        out_path.write_bytes(output)
        out_md5 = md5_hex(output)
        if not args.quiet:
            print(f"applied:   {out_path} md5={out_md5}")
            if skipped:
                print(f"skipped:   {', '.join(skipped)}")
        if args.expect_md5 and out_md5 != args.expect_md5.lower():
            print(
                f"error: patched md5 {out_md5} != expected {args.expect_md5.lower()}",
                file=sys.stderr,
            )
            return 4
    return exit_code


def cmd_learn(args: argparse.Namespace) -> int:
    db = load_db(Path(args.tables))
    profiles = learn_profiles(
        args.sample,
        db,
        args.goal,
        min_context=args.context,
        max_context=args.max_context,
    )
    out = Path(args.out)
    out.write_text(json.dumps(profiles, indent=2) + "\n", encoding="utf-8")
    exact = sum(
        len(profile["sites"]) for profile in profiles["profiles"] if profile["kind"] == "exact"
    )
    consensus = sum(
        len(profile["sites"]) for profile in profiles["profiles"] if profile["kind"] == "consensus"
    )
    if not args.quiet:
        print(f"profiles:  {out}")
        print(f"exact:     {exact} sites across {sum(1 for p in profiles['profiles'] if p['kind'] == 'exact')} builds")
        print(f"consensus: {consensus} sites")
        for profile in profiles["profiles"]:
            bad = [site["name"] for site in profile["sites"] if not site["unique"]]
            if bad:
                print(f"warning:   {profile['id']}: {len(bad)} non-unique pattern(s): {', '.join(bad[:5])}")
    return 0


def cmd_selftest(args: argparse.Namespace) -> int:
    db = load_db(Path(args.tables))
    profiles = load_profiles(Path(args.profiles))
    failures = 0
    for md5, path in args.sample:
        data = path.read_bytes()
        actual = md5_hex(data)
        if actual != md5:
            print(f"FAIL {path}: md5 {actual} != {md5}")
            failures += 1
            continue
        table = find_table(db, actual, args.goal)
        if table is None:
            print(f"FAIL {path}: no {args.goal} table for {actual}")
            failures += 1
            continue
        pe = parse_binary(path, data)
        results = scan_binary(pe, table, profiles, force_scan=True)
        bad = [result for result in results if not result.ok]
        if bad:
            failures += len(bad)
            print(f"FAIL {table.discord} {actual}: {len(bad)}/{len(results)} sites not resolved")
            for result in bad:
                print(f"  {result.status:9} {result.site.name}: {result.note}")
        else:
            print(f"PASS {table.discord} {actual}: {len(results)}/{len(results)} sites")
        if args.expect_patched:
            expected = dict(args.expect_patched)
            if actual in expected:
                try:
                    output, _skipped = apply_results(pe, results)
                except ValueError as exc:
                    print(f"FAIL apply {table.discord}: {exc}")
                    failures += 1
                    continue
                out_md5 = md5_hex(output)
                if out_md5 == expected[actual]:
                    print(f"PASS patched md5 {out_md5}")
                else:
                    print(f"FAIL patched md5 {out_md5} != {expected[actual]}")
                    failures += 1
    return 0 if failures == 0 else 1


def cmd_diff(args: argparse.Namespace) -> int:
    clean_path = Path(args.clean).expanduser()
    patched_path = Path(args.patched).expanduser()
    clean = clean_path.read_bytes()
    patched = patched_path.read_bytes()
    ranges = diff_binaries(clean, patched, gap=args.gap)
    if args.json:
        Path(args.json).write_text(json.dumps(ranges, indent=2) + "\n", encoding="utf-8")
    if not args.quiet:
        print(f"clean:   {clean_path} ({len(clean)} bytes)")
        print(f"patched: {patched_path} ({len(patched)} bytes)")
        print(f"ranges:  {len(ranges)}")
        print()
        for item in ranges:
            print(
                f"0x{item['offset']:08x} len={item['length']:4} "
                f"clean={item['clean']} patched={item['patched']}"
            )
    return 0


def cmd_inspect(args: argparse.Namespace) -> int:
    path = Path(args.node).expanduser()
    data = path.read_bytes()
    pe = parse_binary(path, data)
    info = {
        "path": str(path),
        "md5": md5_hex(data),
        "sha256": sha256_hex(data),
        "size": len(data),
        "machine": f"0x{pe.machine:04x}",
        "arch": pe.arch,
        "image_base": f"0x{pe.image_base:x}",
        "timestamp": pe.timestamp,
        "size_of_headers": pe.size_of_headers,
        "functions": len(pe.functions()),
        "sections": [
            {
                "name": section.name,
                "vaddr": f"0x{section.vaddr:x}",
                "vsize": section.vsize,
                "raw_offset": f"0x{section.raw_offset:x}",
                "raw_size": section.raw_size,
            }
            for section in pe.sections
        ],
    }
    if args.json:
        Path(args.json).write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    if not args.quiet:
        print(json.dumps(info, indent=2))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", action="version", version=VERSION)
    sub = parser.add_subparsers(dest="command", required=True)

    scan = sub.add_parser("scan", help="resolve a node against a reference table")
    scan.add_argument("node")
    scan.add_argument("--tables", default=str(DEFAULT_TABLES))
    scan.add_argument("--profiles", default=str(DEFAULT_PROFILES))
    scan.add_argument("--goal", default="v6")
    scan.add_argument("--ref-md5", default=None)
    scan.add_argument("--json", default=None)
    scan.add_argument("--report", default=None)
    scan.add_argument("--apply", default=None)
    scan.add_argument("--expect-md5", default=None)
    scan.add_argument("--force-scan", action="store_true", help="ignore the exact md5 table")
    scan.add_argument("--force", action="store_true", help="apply resolved sites even if others fail")
    scan.add_argument("--overwrite", action="store_true")
    scan.add_argument("--lang", default="en", choices=["en", "pt"])
    scan.add_argument("--quiet", action="store_true")
    scan.set_defaults(func=cmd_scan)

    learn = sub.add_parser("learn", help="build or extend the profile bank")
    learn.add_argument("--sample", action="append", type=parse_sample, required=True)
    learn.add_argument("--tables", default=str(DEFAULT_TABLES))
    learn.add_argument("--goal", default="v6")
    learn.add_argument("--out", default=str(DEFAULT_PROFILES))
    learn.add_argument("--context", type=int, default=MIN_CONTEXT)
    learn.add_argument("--max-context", type=int, default=MAX_CONTEXT)
    learn.add_argument("--quiet", action="store_true")
    learn.set_defaults(func=cmd_learn)

    selftest = sub.add_parser("selftest", help="verify the finder against known clean nodes")
    selftest.add_argument("--sample", action="append", type=parse_sample, required=True)
    selftest.add_argument("--tables", default=str(DEFAULT_TABLES))
    selftest.add_argument("--profiles", default=str(DEFAULT_PROFILES))
    selftest.add_argument("--goal", default="v6")
    selftest.add_argument("--expect-patched", action="append", type=parse_md5_pair, default=[])
    selftest.set_defaults(func=cmd_selftest)

    diff = sub.add_parser("diff", help="show changed byte ranges between two nodes")
    diff.add_argument("clean")
    diff.add_argument("patched")
    diff.add_argument("--gap", type=int, default=8)
    diff.add_argument("--json", default=None)
    diff.add_argument("--quiet", action="store_true")
    diff.set_defaults(func=cmd_diff)

    inspect = sub.add_parser("inspect", help="show PE metadata for a node")
    inspect.add_argument("node")
    inspect.add_argument("--json", default=None)
    inspect.add_argument("--quiet", action="store_true")
    inspect.set_defaults(func=cmd_inspect)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return int(args.func(args))
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
