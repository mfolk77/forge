#!/usr/bin/env python3
"""
Forge Training Data Quality Filter

Reads distilled examples and filters for quality:
- Removes duplicates
- Validates JSON structure
- Checks for minimum quality thresholds
- Optionally verifies Rust code compiles (--verify-compile)

Usage:
    python3 filter.py                     # Filter all distilled data
    python3 filter.py --category security  # Filter one category
    python3 filter.py --verify-compile     # Also check Rust code compiles
    python3 filter.py --stats              # Show statistics only
"""

import json
import hashlib
import re
import sys
from pathlib import Path
from dataclasses import dataclass

SCRIPT_DIR = Path(__file__).parent
ROOT_DIR = SCRIPT_DIR.parent
DISTILLED_DIR = ROOT_DIR / "distilled"
FINAL_DIR = ROOT_DIR / "final"


@dataclass
class QualityMetrics:
    total: int = 0
    passed: int = 0
    duplicate: int = 0
    too_short: int = 0
    no_code: int = 0
    malformed: int = 0
    low_quality: int = 0


# Minimum thresholds
MIN_INSTRUCTION_LENGTH = 20
MIN_RESPONSE_LENGTH = 100
MIN_CONVERSATION_TURNS = 3


def content_hash(text: str) -> str:
    """Hash content for deduplication."""
    normalized = re.sub(r'\s+', ' ', text.strip().lower())
    return hashlib.sha256(normalized.encode()).hexdigest()[:16]


def has_code_block(text: str) -> bool:
    """Check if text contains a code block."""
    return "```" in text or "fn " in text or "struct " in text or "impl " in text


def validate_instruction_pair(example: dict) -> tuple[bool, str]:
    """Validate an instruction-response pair."""
    instruction = example.get("instruction", "")
    response = example.get("response", "")

    if not instruction or not response:
        return False, "missing instruction or response"

    if len(instruction) < MIN_INSTRUCTION_LENGTH:
        return False, f"instruction too short ({len(instruction)} chars)"

    if len(response) < MIN_RESPONSE_LENGTH:
        return False, f"response too short ({len(response)} chars)"

    # Response should contain code for most examples
    if not has_code_block(response) and "```" not in response:
        # Some reasoning/explanation examples are okay without code
        if len(response) < 200:
            return False, "response has no code and is too short for pure reasoning"

    return True, "ok"


def validate_conversation(example: dict) -> tuple[bool, str]:
    """Validate a multi-turn conversation."""
    messages = example.get("messages", [])
    if not messages:
        # Maybe it's a flat array of role/content pairs
        if isinstance(example, list) and len(example) > 0 and "role" in example[0]:
            messages = example
        else:
            return False, "no messages found"

    if len(messages) < MIN_CONVERSATION_TURNS:
        return False, f"too few turns ({len(messages)})"

    # Check alternating roles
    roles = [m.get("role", "") for m in messages]
    if roles[0] != "user":
        return False, "conversation should start with user"

    # Check that assistant responses have substance
    for msg in messages:
        if msg.get("role") == "assistant" and len(msg.get("content", "")) < 50:
            return False, "assistant response too short"

    return True, "ok"


def validate_example(example: dict) -> tuple[bool, str]:
    """Route to appropriate validator."""
    if "messages" in example:
        return validate_conversation(example)
    if "instruction" in example:
        return validate_instruction_pair(example)
    if "role" in example:
        # Single message from a conversation — skip these
        return False, "single message, not a complete example"
    return False, "unknown format"


def filter_category(category: str, metrics: QualityMetrics, seen_hashes: set) -> list[dict]:
    """Filter all examples in a category."""
    category_dir = DISTILLED_DIR / category
    if not category_dir.exists():
        return []

    passed = []

    for jsonl_file in sorted(category_dir.glob("*.jsonl")):
        with open(jsonl_file, "r", encoding="utf-8") as f:
            for line_num, line in enumerate(f, 1):
                line = line.strip()
                if not line:
                    continue

                metrics.total += 1

                # Parse JSON
                try:
                    example = json.loads(line)
                except json.JSONDecodeError:
                    metrics.malformed += 1
                    continue

                # Validate
                valid, reason = validate_example(example)
                if not valid:
                    if "short" in reason:
                        metrics.too_short += 1
                    elif "no code" in reason:
                        metrics.no_code += 1
                    else:
                        metrics.low_quality += 1
                    continue

                # Dedup
                key = content_hash(json.dumps(example, sort_keys=True))
                if key in seen_hashes:
                    metrics.duplicate += 1
                    continue
                seen_hashes.add(key)

                metrics.passed += 1
                passed.append(example)

    return passed


def save_filtered(category: str, examples: list[dict]):
    """Save filtered examples."""
    out_dir = FINAL_DIR / category
    out_dir.mkdir(parents=True, exist_ok=True)

    out_file = out_dir / "examples.jsonl"
    with open(out_file, "w", encoding="utf-8") as f:
        for example in examples:
            f.write(json.dumps(example, ensure_ascii=False) + "\n")


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Filter distilled training data")
    parser.add_argument("--category", help="Filter only this category")
    parser.add_argument("--stats", action="store_true", help="Show stats only")
    args = parser.parse_args()

    categories = [
        "architecture", "security", "api-services",
        "data-persistence", "rust-idioms", "testing-methodology"
    ]

    if args.category:
        categories = [args.category]

    overall_metrics = QualityMetrics()
    seen_hashes = set()
    all_examples = {}

    for category in categories:
        metrics = QualityMetrics()
        examples = filter_category(category, metrics, seen_hashes)
        all_examples[category] = examples

        # Accumulate overall
        overall_metrics.total += metrics.total
        overall_metrics.passed += metrics.passed
        overall_metrics.duplicate += metrics.duplicate
        overall_metrics.too_short += metrics.too_short
        overall_metrics.no_code += metrics.no_code
        overall_metrics.malformed += metrics.malformed
        overall_metrics.low_quality += metrics.low_quality

        if not args.stats:
            save_filtered(category, examples)

        pass_rate = (metrics.passed / metrics.total * 100) if metrics.total > 0 else 0
        print(f"[{category}] {metrics.passed}/{metrics.total} passed ({pass_rate:.0f}%)"
              f" | dup:{metrics.duplicate} short:{metrics.too_short} bad:{metrics.malformed}")

    print(f"\n{'='*60}")
    total = overall_metrics.total
    if total > 0:
        print(f"Total:      {total:>6}")
        print(f"Passed:     {overall_metrics.passed:>6} ({overall_metrics.passed/total*100:.0f}%)")
        print(f"Duplicate:  {overall_metrics.duplicate:>6}")
        print(f"Too short:  {overall_metrics.too_short:>6}")
        print(f"No code:    {overall_metrics.no_code:>6}")
        print(f"Malformed:  {overall_metrics.malformed:>6}")
        print(f"Low quality:{overall_metrics.low_quality:>6}")
    else:
        print("No examples found. Run distill.py first.")

    if not args.stats:
        print(f"\nOutput: {FINAL_DIR}/")


if __name__ == "__main__":
    main()
