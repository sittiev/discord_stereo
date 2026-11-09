#!/usr/bin/env python3
"""Map a Patch-DB offset table onto another build of the same platform.

Unlike find_offsets.py this tool does not need AOB profiles: it matches the
reference and target by *function symbol* (`nm` + `c++filt`), keeps the
in-function delta, and verifies the expected original/patch bytes before
accepting a site. When the delta moved it searches the target function body
for the original byte sequence. Byte-verified results can be emitted as a
schema-2 report that `patchdb/autopatch.mjs` consumes.

Works best on symbol-rich ELF builds (Discord Linux ships unstripped nodes).
PE builds work when their symbols are available.

Usage:
  python3 patchdb/map_sites.py --tables patchdb/tables.json --goal v5 \\
    --ref-md5 0bd81054ee44228a03928e84044a4d32 --ref-node /tmp/lx161.node \\
    --node /tmp/lx168.node --mapping /tmp/mapping.json [--report /tmp/report.json]

Exit codes: 0 finished (mapping carries per-site status), 2 input error.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import re
import struct
import subprocess
import sys
import time
from pathlib import Path

ELF_MACHINE = {0x3E: "x86_64", 0x03: "x86", 0xB7: "arm64", 0x28: "arm"}
PE_MACHINE = {0x8664: "x86_64", 0x014C: "x86", 0xAA64: "arm64", 0x01C4: "arm"}


def load_symbols(path: Path):
    try:
        nm = subprocess.run(
            ["nm", "--defined-only", str(path)], capture_output=True, text=True, check=True
        ).stdout
    except (OSError, subprocess.CalledProcessError) as exc:
        raise SystemExit(f"error: nm failed on {path}: {exc}")
    demangled = subprocess.run(
        ["c++filt"], input=nm, capture_output=True, text=True, check=True
    ).stdout
    syms = []
    for line in demangled.splitlines():
        parts = line.split(" ", 2)
        if len(parts) < 3:
            continue
        try:
            addr = int(parts[0], 16)
        except ValueError:
            continue
        syms.append((addr, parts[2].strip()))
    syms.sort()
    by_name: dict[str, int] = {}
    for addr, name in syms:
        by_name.setdefault(name, addr)
    return [addr for addr, _ in syms], syms, by_name


def symbol_at(addrs, syms, offset: int):
    index = bisect.bisect_right(addrs, offset) - 1
    return syms[index] if index >= 0 else (0, "?")


def binary_info(path: Path, data: bytes) -> dict:
    if data[:4] == b"\x7fELF" and len(data) > 20:
        machine = struct.unpack_from("<H", data, 18)[0]
        return {"os": "linux", "arch": ELF_MACHINE.get(machine, f"machine_0x{machine:x}")}
    if data[:2] == b"MZ" and len(data) > 0x40:
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        machine = struct.unpack_from("<H", data, pe + 4)[0] if pe + 6 <= len(data) else 0
        return {"os": "windows", "arch": PE_MACHINE.get(machine, f"machine_0x{machine:x}")}
    return {"os": "unknown", "arch": "unknown"}


def locate(data: bytes, sym_addr: int, delta: int, orig: bytes, patch: bytes, window: int):
    candidate = sym_addr + delta
    got = data[candidate : candidate + len(orig)]
    if got == orig:
        return "verified-delta", candidate
    if got == patch:
        return "already-patched", candidate
    if len(orig) > 1:
        found = data.find(orig, sym_addr, min(len(data), sym_addr + window))
        if found != -1:
            return "verified-search", found
        return "miss", None
    return "miss-1byte", None


CLIENT_DIR_KINDS = {
    "discord": "stable",
    "discordptb": "ptb",
    "discord ptb": "ptb",
    "discord-ptb": "ptb",
    "discordcanary": "canary",
    "discord canary": "canary",
    "discord-canary": "canary",
    "vencord": "vencord",
    "equicord": "vencord",
}


def client_kind(path: Path) -> str:
    """Return the client kind from recognized install-directory names, not path substrings."""
    for part in reversed(path.parts):
        kind = CLIENT_DIR_KINDS.get(part.lower())
        if kind is not None:
            return kind
    return "stable"


def build_report(args, node_md5: str, info: dict, resolved: list, window: int) -> dict:
    now = int(time.time())
    match = re.search(r"app-\d+\.\d+\.\d+", str(args.node))
    return {
        "app_version": "1.0.0",
        "schema": 2,
        "ts_unix": now,
        "ts_iso": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now)),
        "report_id": f"{now}-{node_md5[:12]}",
        "os": info["os"],
        "arch": info["arch"],
        "goal": args.goal,
        "lang": "en",
        "node_path": str(args.node),
        "node_md5": node_md5,
        "node_size": args.node.stat().st_size,
        "discord_dir": match.group(0) if match else "",
        "client_kind": client_kind(args.node),
        "table_kind": "none",
        "source_md5": args.ref_md5,
        "sites": resolved,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tables", default="patchdb/tables.json")
    parser.add_argument("--goal", required=True)
    parser.add_argument("--ref-md5", required=True)
    parser.add_argument("--ref-node", required=True, type=Path)
    parser.add_argument("--node", required=True, type=Path)
    parser.add_argument("--mapping", type=Path, help="write the per-site mapping JSON here")
    parser.add_argument("--report", type=Path, help="write a schema-2 report with resolved sites here")
    parser.add_argument("--window", type=lambda value: int(value, 0), default=0x1200)
    args = parser.parse_args()

    db = json.loads(Path(args.tables).read_text(encoding="utf-8"))
    ref = next(
        (t for t in db.get("tables", []) if t.get("goal") == args.goal and t.get("binary_md5") == args.ref_md5),
        None,
    )
    if ref is None:
        print(f"error: no {args.goal} table for md5 {args.ref_md5}", file=sys.stderr)
        return 2
    ref_data = args.ref_node.read_bytes()
    data = args.node.read_bytes()
    ref_addrs, ref_syms, _ = load_symbols(args.ref_node)
    _, _, target_by_name = load_symbols(args.node)
    info = binary_info(args.node, data)
    node_md5 = hashlib.md5(data).hexdigest()

    mapping = []
    resolved = []
    for site in ref.get("sites", []):
        name = site.get("name", "?")
        ref_offset = int(site["offset"], 16)
        orig = bytes.fromhex(site["expected_orig"] if "expected_orig" in site else site["orig"])
        patch = bytes.fromhex(site["expected_patch"] if "expected_patch" in site else site["patch"])
        ref_addr, ref_name = symbol_at(ref_addrs, ref_syms, ref_offset)
        delta = ref_offset - ref_addr
        target = target_by_name.get(ref_name)
        if target is None:
            status, new_offset = "no-symbol", None
        else:
            status, new_offset = locate(data, target, delta, orig, patch, args.window)
        mapping.append(
            {
                "name": name,
                "status": status,
                "ref_offset": ref_offset,
                "ref_function": ref_name,
                "ref_delta": delta,
                "new_offset": new_offset,
                "orig": orig.hex(),
                "patch": patch.hex(),
            }
        )
        if status in {"verified-delta", "verified-search"}:
            resolved.append(
                {
                    "name": name,
                    "state": "ok",
                    "expected_orig": orig.hex(),
                    "expected_patch": patch.hex(),
                    "found": orig.hex(),
                    "neighborhood": None,
                    "offset": new_offset,
                    "new_offset": new_offset,
                    "unique": True,
                    "method": status,
                    "score": 100,
                    "hits": 1,
                    "note": f"mapped from {ref_name} (delta 0x{delta:x}) on the {args.goal} reference",
                }
            )

    counts: dict[str, int] = {}
    for entry in mapping:
        counts[entry["status"]] = counts.get(entry["status"], 0) + 1
        target_offset = f"0x{entry['new_offset']:x}" if entry["new_offset"] is not None else "-"
        print(f"{entry['status']:<16} {entry['name']:<40} {target_offset}")
    print(f"summary: {counts}")
    if args.mapping:
        args.mapping.parent.mkdir(parents=True, exist_ok=True)
        args.mapping.write_text(
            json.dumps(
                {
                    "schema": 1,
                    "goal": args.goal,
                    "ref_md5": args.ref_md5,
                    "node_md5": node_md5,
                    "os": info["os"],
                    "arch": info["arch"],
                    "sites": mapping,
                },
                indent=1,
            )
            + "\n",
            encoding="utf-8",
        )
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(
            json.dumps(build_report(args, node_md5, info, resolved, args.window), indent=1) + "\n",
            encoding="utf-8",
        )
        print(f"report: {len(resolved)} resolved site(s) -> {args.report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
