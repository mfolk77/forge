#!/usr/bin/env python3
"""
Forge Training Data Extractor

Reads source-map.toml and extracts code patterns from Forge + Serena
into raw-extracts/ organized by category.

Usage:
    python3 extract.py                    # Extract all
    python3 extract.py --category security # Extract one category
    python3 extract.py --dry-run          # Show what would be extracted
"""

import tomllib
import json
import subprocess
import sys
from pathlib import Path
from dataclasses import dataclass, field

SCRIPT_DIR = Path(__file__).parent
ROOT_DIR = SCRIPT_DIR.parent
SOURCE_MAP = ROOT_DIR / "source-map.toml"
RAW_DIR = ROOT_DIR / "raw-extracts"


@dataclass
class ExtractedPattern:
    name: str
    category: str
    source_file: str
    code: str
    line_count: int
    why: str
    distill_as: list[str]
    extract_strategy: str = "full-file"
    pattern: str = ""


def load_source_map() -> dict:
    with open(SOURCE_MAP, "rb") as f:
        return tomllib.load(f)


def expand_path(path: str) -> Path:
    return Path(path).expanduser()


def extract_full_file(source_path: Path) -> str | None:
    if not source_path.exists():
        print(f"  WARNING: {source_path} does not exist, skipping")
        return None
    return source_path.read_text(encoding="utf-8")


def extract_grep_pattern(source_dir: Path, pattern: str, context_lines: int = 10) -> str | None:
    """Extract code snippets matching a grep pattern with surrounding context."""
    if not source_dir.exists():
        print(f"  WARNING: {source_dir} does not exist, skipping")
        return None

    try:
        # Use ripgrep for fast pattern matching with context
        result = subprocess.run(
            ["rg", "-n", f"-C{context_lines}", "--type", "rust", pattern, str(source_dir)],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass

    # Fallback to grep
    try:
        result = subprocess.run(
            ["grep", "-rn", f"-C{context_lines}", "--include=*.rs", pattern, str(source_dir)],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0:
            return result.stdout
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass

    return None


def extract_entry(category: str, entry: dict) -> ExtractedPattern | None:
    name = entry["name"]
    source = entry["source"]
    why = entry.get("why", "")
    distill_as = entry.get("distill_as", ["instruction"])
    strategy = entry.get("extract_strategy", "full-file")
    pattern = entry.get("pattern", "")

    source_path = expand_path(source)

    print(f"  Extracting: {name} ({strategy})")

    if strategy == "full-file":
        code = extract_full_file(source_path)
    elif strategy == "grep-pattern":
        code = extract_grep_pattern(source_path, pattern)
    else:
        print(f"  WARNING: Unknown strategy '{strategy}', skipping")
        return None

    if code is None:
        return None

    line_count = len(code.splitlines())

    return ExtractedPattern(
        name=name,
        category=category,
        source_file=str(source_path),
        code=code,
        line_count=line_count,
        why=why,
        distill_as=distill_as,
        extract_strategy=strategy,
        pattern=pattern,
    )


def save_extract(extract: ExtractedPattern):
    category_dir = RAW_DIR / extract.category
    category_dir.mkdir(parents=True, exist_ok=True)

    # Save the code
    code_file = category_dir / f"{extract.name}.code"
    code_file.write_text(extract.code, encoding="utf-8")

    # Save metadata alongside
    meta_file = category_dir / f"{extract.name}.meta.json"
    meta = {
        "name": extract.name,
        "category": extract.category,
        "source_file": extract.source_file,
        "line_count": extract.line_count,
        "why": extract.why,
        "distill_as": extract.distill_as,
        "extract_strategy": extract.extract_strategy,
        "pattern": extract.pattern,
    }
    meta_file.write_text(json.dumps(meta, indent=2), encoding="utf-8")


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Extract training data from source repos")
    parser.add_argument("--category", help="Extract only this category")
    parser.add_argument("--dry-run", action="store_true", help="Show what would be extracted")
    args = parser.parse_args()

    source_map = load_source_map()
    categories = [
        "architecture", "security", "api-services",
        "data-persistence", "rust-idioms", "testing-methodology"
    ]

    if args.category:
        categories = [args.category]

    total_extracted = 0
    total_lines = 0
    total_skipped = 0

    for category in categories:
        entries = source_map.get(category, [])
        if not entries:
            print(f"\n[{category}] No entries in source map")
            continue

        print(f"\n[{category}] {len(entries)} entries")

        for entry in entries:
            if args.dry_run:
                source = expand_path(entry["source"])
                exists = "OK" if source.exists() else "MISSING"
                print(f"  {entry['name']}: {source} [{exists}]")
                continue

            extract = extract_entry(category, entry)
            if extract:
                save_extract(extract)
                total_extracted += 1
                total_lines += extract.line_count
                print(f"    → {extract.line_count} lines saved")
            else:
                total_skipped += 1

    if not args.dry_run:
        print(f"\n{'='*50}")
        print(f"Extracted: {total_extracted} patterns ({total_lines:,} lines)")
        print(f"Skipped:   {total_skipped} (missing files)")
        print(f"Output:    {RAW_DIR}/")


if __name__ == "__main__":
    main()
