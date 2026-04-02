#!/usr/bin/env python3
"""
Forge Training Data Merger

Combines all filtered examples into final train.jsonl and val.jsonl files
ready for fine-tuning. Handles format conversion to ChatML for Qwen.

Usage:
    python3 merge.py                    # Merge all, 90/10 split
    python3 merge.py --split 0.85       # Custom train/val split
    python3 merge.py --format chatml    # Output in ChatML format (default)
    python3 merge.py --format alpaca    # Output in Alpaca format
    python3 merge.py --stats            # Show dataset statistics
"""

import json
import random
import sys
from pathlib import Path
from collections import Counter

SCRIPT_DIR = Path(__file__).parent
ROOT_DIR = SCRIPT_DIR.parent
FINAL_DIR = ROOT_DIR / "final"
OUTPUT_DIR = ROOT_DIR

SYSTEM_MESSAGE = (
    "You are Forge, a specialized backend development AI assistant built by FolkTech AI. "
    "You run locally as a Rust CLI tool. You are an expert in Rust systems programming, "
    "API design, security-first development, database patterns, and tool architectures. "
    "You always consider security implications and follow the FolkTech Secure Coding Standard. "
    "You write production-grade code with proper error handling, never happy-path-only implementations."
)


def load_all_examples() -> list[tuple[str, dict]]:
    """Load all filtered examples with their category."""
    examples = []
    for category_dir in sorted(FINAL_DIR.iterdir()):
        if not category_dir.is_dir():
            continue
        category = category_dir.name
        jsonl_file = category_dir / "examples.jsonl"
        if not jsonl_file.exists():
            continue
        with open(jsonl_file, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    examples.append((category, json.loads(line)))
    return examples


def to_chatml(example: dict) -> dict:
    """Convert an example to ChatML format for Qwen fine-tuning."""
    messages = [{"role": "system", "content": SYSTEM_MESSAGE}]

    if "messages" in example:
        # Already a conversation
        messages.extend(example["messages"])
    elif "instruction" in example:
        # Instruction-response pair → single-turn conversation
        messages.append({"role": "user", "content": example["instruction"]})
        messages.append({"role": "assistant", "content": example["response"]})
    else:
        return None

    return {"messages": messages}


def to_alpaca(example: dict) -> dict:
    """Convert to Alpaca format."""
    if "instruction" in example:
        return {
            "instruction": example["instruction"],
            "input": "",
            "output": example["response"],
        }
    elif "messages" in example:
        # Convert conversation to instruction format (use last exchange)
        user_msgs = [m for m in example["messages"] if m["role"] == "user"]
        asst_msgs = [m for m in example["messages"] if m["role"] == "assistant"]
        if user_msgs and asst_msgs:
            return {
                "instruction": user_msgs[-1]["content"],
                "input": "\n".join(m["content"] for m in user_msgs[:-1]),
                "output": asst_msgs[-1]["content"],
            }
    return None


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Merge filtered data into training files")
    parser.add_argument("--split", type=float, default=0.9, help="Train/val split ratio")
    parser.add_argument("--format", choices=["chatml", "alpaca"], default="chatml")
    parser.add_argument("--stats", action="store_true", help="Show stats only")
    parser.add_argument("--seed", type=int, default=42, help="Random seed for split")
    args = parser.parse_args()

    examples = load_all_examples()
    if not examples:
        print("No filtered examples found. Run filter.py first.")
        sys.exit(1)

    # Convert to target format
    converter = to_chatml if args.format == "chatml" else to_alpaca
    converted = []
    category_counts = Counter()

    for category, example in examples:
        result = converter(example)
        if result:
            result["_category"] = category  # metadata, stripped before saving
            converted.append(result)
            category_counts[category] += 1

    if args.stats:
        print(f"\nDataset Statistics:")
        print(f"  Total examples: {len(converted)}")
        print(f"  Format: {args.format}")
        print(f"\n  By category:")
        for cat, count in sorted(category_counts.items()):
            pct = count / len(converted) * 100
            print(f"    {cat:25s} {count:>5} ({pct:5.1f}%)")

        # Estimate token counts
        total_tokens = 0
        for ex in converted:
            text = json.dumps(ex)
            total_tokens += len(text) // 4
        print(f"\n  Estimated total tokens: {total_tokens:,}")
        print(f"  Avg tokens per example: {total_tokens // len(converted):,}")
        return

    # Shuffle and split
    random.seed(args.seed)
    random.shuffle(converted)

    split_idx = int(len(converted) * args.split)
    train_set = converted[:split_idx]
    val_set = converted[split_idx:]

    # Save (strip metadata)
    train_file = OUTPUT_DIR / "train.jsonl"
    val_file = OUTPUT_DIR / "val.jsonl"

    for filepath, dataset in [(train_file, train_set), (val_file, val_set)]:
        with open(filepath, "w", encoding="utf-8") as f:
            for example in dataset:
                clean = {k: v for k, v in example.items() if not k.startswith("_")}
                f.write(json.dumps(clean, ensure_ascii=False) + "\n")

    print(f"\nMerged {len(converted)} examples ({args.format} format)")
    print(f"  Train: {len(train_set)} → {train_file}")
    print(f"  Val:   {len(val_set)} → {val_file}")
    print(f"\n  By category:")
    for cat, count in sorted(category_counts.items()):
        print(f"    {cat:25s} {count:>5}")


if __name__ == "__main__":
    main()
