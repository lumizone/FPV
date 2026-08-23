#!/usr/bin/env python3
"""Verify a locale file has exactly the same leaf-key set as en.json.

Usage: python3 scripts/i18n_check_parity.py [locales_dir]
Defaults to src/i18n/locales. Exits non-zero if any locale diverges.
"""
import json
import sys
from pathlib import Path


def leaf_keys(obj, prefix=""):
    keys = set()
    for k, v in obj.items():
        kk = f"{prefix}.{k}" if prefix else k
        if isinstance(v, dict):
            keys |= leaf_keys(v, kk)
        else:
            keys.add(kk)
    return keys


def main():
    base = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("src/i18n/locales")
    en = json.load(open(base / "en.json", encoding="utf-8"))
    en_keys = leaf_keys(en)
    print(f"en.json: {len(en_keys)} leaf keys")
    ok = True
    for f in sorted(base.glob("*.json")):
        if f.name == "en.json":
            continue
        d = json.load(open(f, encoding="utf-8"))
        ks = leaf_keys(d)
        missing = en_keys - ks
        extra = ks - en_keys
        status = "OK" if not missing and not extra else "DIVERGES"
        if missing or extra:
            ok = False
        print(f"{f.name}: {len(ks)} keys [{status}]")
        for m in sorted(missing):
            print(f"    MISSING {m}")
        for e in sorted(extra):
            print(f"    EXTRA   {e}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
