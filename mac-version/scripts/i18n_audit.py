#!/usr/bin/env python3
"""Deeper i18n audit than `i18n_check_parity.py`.

Parity only compares the locale files TO EACH OTHER, so a key that is
used in code but exists in no locale file is invisible to it — i18next
just renders the inline English default and every language silently
shows English. That is exactly how the whole `settings.you.*` panel
stayed untranslated (found 2026-07-26).

Reports:
  [1] used in code, missing from locales   <- the one that actually bites
  [2] in locales, never used               <- dead weight (v2.0-hidden features live here)
  [3] placeholder mismatches ({{x}})       <- runtime-broken interpolation
  [4] values identical to English          <- possibly untranslated (product names are fine)

Run from the repo root:  python3 scripts/i18n_audit.py
"""
import json, re, pathlib, collections, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC, LOC = ROOT / "src", ROOT / "src/i18n/locales"
CALL = re.compile(r"""\bt\(\s*['"]([\w.-]+)['"]\s*(?:,|\))""")
PH = re.compile(r"\{\{(\w+)\}\}")

used = collections.Counter()
for f in list(SRC.rglob("*.tsx")) + list(SRC.rglob("*.ts")):
    if "i18n/locales" in str(f):
        continue
    for m in CALL.finditer(f.read_text()):
        used[m.group(1)] += 1

def flat(o, p=""):
    out = {}
    for k, v in o.items():
        n = f"{p}.{k}" if p else k
        out.update(flat(v, n)) if isinstance(v, dict) else out.update({n: v})
    return out

locales = {p.stem: flat(json.loads(p.read_text())) for p in sorted(LOC.glob("*.json"))}
en = locales["en"]
print(f"keys used in code: {len(used)}   keys in en.json: {len(en)}\n")

missing = sorted(k for k in used if k not in en)
print(f"[1] USED IN CODE, MISSING FROM LOCALES: {len(missing)}")
for k in missing:
    print(f"      {k}  ({used[k]}x)")

orphans = sorted(k for k in en if k not in used)
print(f"\n[2] IN LOCALES, NEVER USED: {len(orphans)}")
for k in orphans[:20]:
    print(f"      {k}")
if len(orphans) > 20:
    print(f"      … and {len(orphans) - 20} more")

print("\n[3] PLACEHOLDER MISMATCH:")
bad = 0
for k, v in en.items():
    if not isinstance(v, str):
        continue
    ph = set(PH.findall(v))
    if not ph:
        continue
    for l, d in locales.items():
        tv = d.get(k)
        if l != "en" and isinstance(tv, str) and set(PH.findall(tv)) != ph:
            print("      %s: %s  en=%s vs %s" % (l, k, sorted(ph), sorted(set(PH.findall(tv)))))
            bad += 1
print(f"      total: {bad}")

print("\n[4] IDENTICAL TO ENGLISH (product names are expected here):")
for l, d in locales.items():
    if l == "en":
        continue
    same = [k for k, v in d.items()
            if isinstance(v, str) and v == en.get(k) and len(v) > 12 and re.search(r"[A-Za-z]{3}", v)]
    print(f"      {l}: {len(same)}")

# Only [1] and [3] are hard failures.
sys.exit(1 if (missing or bad) else 0)
