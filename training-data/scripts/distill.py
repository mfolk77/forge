#!/usr/bin/env python3
"""
Forge Training Data Distiller

Reads raw extracts and distillation prompts, sends them to a teacher model
(Claude Opus), and saves the generated training examples.

Usage:
    python3 distill.py                              # Distill all
    python3 distill.py --category security           # Distill one category
    python3 distill.py --name conversation-security-tests  # Distill one extract
    python3 distill.py --dry-run                     # Show what would be distilled
    python3 distill.py --estimate-cost               # Estimate API cost

Requires:
    ANTHROPIC_API_KEY environment variable set
    pip install anthropic
"""

import anthropic
import tomllib
import json
import os
import sys
import time
from pathlib import Path
from dataclasses import dataclass

SCRIPT_DIR = Path(__file__).parent
ROOT_DIR = SCRIPT_DIR.parent
RAW_DIR = ROOT_DIR / "raw-extracts"
DISTILLED_DIR = ROOT_DIR / "distilled"
PROMPTS_FILE = ROOT_DIR / "distill-prompts.toml"

# Rate limiting
REQUESTS_PER_MINUTE = 40
REQUEST_INTERVAL = 60.0 / REQUESTS_PER_MINUTE

# Token estimation (rough: 4 chars ≈ 1 token)
CHARS_PER_TOKEN = 4


@dataclass
class DistillJob:
    name: str
    category: str
    code: str
    source_file: str
    why: str
    distill_type: str  # "instruction", "conversation", "reasoning", "negative"
    prompt: str
    estimated_input_tokens: int
    estimated_output_tokens: int


def load_prompts() -> dict:
    with open(PROMPTS_FILE, "rb") as f:
        return tomllib.load(f)


def load_extract(category: str, name: str) -> tuple[str, dict] | None:
    code_file = RAW_DIR / category / f"{name}.code"
    meta_file = RAW_DIR / category / f"{name}.meta.json"

    if not code_file.exists():
        return None

    code = code_file.read_text(encoding="utf-8")
    meta = json.loads(meta_file.read_text()) if meta_file.exists() else {}
    return code, meta


def build_prompt(template: str, code: str, meta: dict) -> str:
    """Fill in the prompt template with actual values."""
    return template.format(
        code=code,
        filename=meta.get("source_file", "unknown"),
        why=meta.get("why", ""),
        count=meta.get("count", 5),
    )


def estimate_tokens(text: str) -> int:
    return len(text) // CHARS_PER_TOKEN


def create_distill_jobs(category_filter: str = None, name_filter: str = None) -> list[DistillJob]:
    """Scan raw-extracts and create distillation jobs."""
    prompts = load_prompts()
    jobs = []

    for category_dir in sorted(RAW_DIR.iterdir()):
        if not category_dir.is_dir():
            continue
        category = category_dir.name

        if category_filter and category != category_filter:
            continue

        # Get prompt templates for this category
        category_prompts = prompts.get(category.replace("-", "_"), {})
        if not category_prompts:
            # Try with hyphens
            category_prompts = prompts.get(category, {})

        for meta_file in sorted(category_dir.glob("*.meta.json")):
            name = meta_file.stem.replace(".meta", "")

            if name_filter and name != name_filter:
                continue

            result = load_extract(category, name)
            if not result:
                continue
            code, meta = result

            # Create a job for each distill type
            distill_types = meta.get("distill_as", ["instruction"])
            for dtype in distill_types:
                # Find the matching prompt template
                prompt_config = category_prompts.get(dtype, {})
                template = prompt_config.get("prompt_template", "")
                if not template:
                    continue

                # Inject count from prompt config if available
                meta_with_count = {**meta, "count": prompt_config.get("count", 5)}
                prompt = build_prompt(template, code, meta_with_count)

                input_tokens = estimate_tokens(prompt)
                output_tokens = 4096  # max per response

                jobs.append(DistillJob(
                    name=name,
                    category=category,
                    code=code,
                    source_file=meta.get("source_file", ""),
                    why=meta.get("why", ""),
                    distill_type=dtype,
                    prompt=prompt,
                    estimated_input_tokens=input_tokens,
                    estimated_output_tokens=output_tokens,
                ))

    return jobs


def distill_single(client: anthropic.Anthropic, job: DistillJob, system_prompt: str) -> str | None:
    """Send a single distillation request to the teacher model."""
    try:
        response = client.messages.create(
            model="claude-opus-4-6",
            max_tokens=4096,
            temperature=0.7,
            system=system_prompt,
            messages=[{"role": "user", "content": job.prompt}],
        )
        return response.content[0].text
    except anthropic.APIError as e:
        print(f"  API error: {e}")
        return None
    except Exception as e:
        print(f"  Error: {e}")
        return None


def save_distilled(job: DistillJob, output: str):
    """Save distilled output."""
    out_dir = DISTILLED_DIR / job.category
    out_dir.mkdir(parents=True, exist_ok=True)

    filename = f"{job.name}.{job.distill_type}.jsonl"
    out_file = out_dir / filename

    # Try to parse as JSON array and save as JSONL
    try:
        # The output might be a JSON array or individual JSON objects
        # Try to extract JSON from the response
        parsed = extract_json_examples(output)
        with open(out_file, "a", encoding="utf-8") as f:
            for example in parsed:
                f.write(json.dumps(example, ensure_ascii=False) + "\n")
        return len(parsed)
    except Exception:
        # If parsing fails, save raw output for manual review
        raw_file = out_dir / f"{job.name}.{job.distill_type}.raw.txt"
        raw_file.write_text(output, encoding="utf-8")
        return 0


def extract_json_examples(text: str) -> list[dict]:
    """Extract JSON objects/arrays from teacher model output."""
    examples = []

    # Try parsing as a JSON array
    try:
        parsed = json.loads(text)
        if isinstance(parsed, list):
            return parsed
        return [parsed]
    except json.JSONDecodeError:
        pass

    # Try to find JSON blocks in the text
    import re

    # Match JSON objects
    json_pattern = re.compile(r'\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\}', re.DOTALL)
    for match in json_pattern.finditer(text):
        try:
            obj = json.loads(match.group())
            if "instruction" in obj or "messages" in obj or "role" in obj:
                examples.append(obj)
        except json.JSONDecodeError:
            continue

    # Match JSON arrays
    array_pattern = re.compile(r'\[[\s\S]*?\](?=\s*(?:\n\n|\Z))', re.DOTALL)
    for match in array_pattern.finditer(text):
        try:
            arr = json.loads(match.group())
            if isinstance(arr, list) and len(arr) > 0:
                examples.extend(arr)
        except json.JSONDecodeError:
            continue

    return examples


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Distill training data via teacher model")
    parser.add_argument("--category", help="Distill only this category")
    parser.add_argument("--name", help="Distill only this extract")
    parser.add_argument("--dry-run", action="store_true", help="Show jobs without executing")
    parser.add_argument("--estimate-cost", action="store_true", help="Estimate API cost")
    args = parser.parse_args()

    jobs = create_distill_jobs(args.category, args.name)

    if not jobs:
        print("No distillation jobs found. Run extract.py first.")
        sys.exit(1)

    if args.dry_run or args.estimate_cost:
        total_input = sum(j.estimated_input_tokens for j in jobs)
        total_output = sum(j.estimated_output_tokens for j in jobs)

        # Claude Opus pricing (approximate)
        input_cost = (total_input / 1_000_000) * 15.0
        output_cost = (total_output / 1_000_000) * 75.0
        total_cost = input_cost + output_cost

        print(f"\nDistillation Plan:")
        print(f"  Jobs: {len(jobs)}")
        print(f"  Estimated input tokens:  {total_input:>10,}")
        print(f"  Estimated output tokens: {total_output:>10,}")
        print(f"  Estimated cost:          ${total_cost:>10.2f}")
        print(f"  Estimated time:          {len(jobs) * REQUEST_INTERVAL / 60:.1f} minutes")

        if args.dry_run:
            print(f"\nJobs by category:")
            by_cat = {}
            for j in jobs:
                by_cat.setdefault(j.category, []).append(j)
            for cat, cat_jobs in sorted(by_cat.items()):
                print(f"\n  [{cat}] {len(cat_jobs)} jobs")
                for j in cat_jobs:
                    print(f"    {j.name} ({j.distill_type}) — ~{j.estimated_input_tokens:,} input tokens")
        return

    # Check for API key
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        print("ERROR: ANTHROPIC_API_KEY not set")
        print("Export your API key: export ANTHROPIC_API_KEY=sk-ant-...")
        sys.exit(1)

    client = anthropic.Anthropic(api_key=api_key)
    prompts = load_prompts()
    system_prompt = prompts.get("general", {}).get("system_prompt", "")

    total_examples = 0
    total_jobs = len(jobs)

    print(f"\nStarting distillation: {total_jobs} jobs")
    print(f"Rate limit: {REQUESTS_PER_MINUTE} req/min\n")

    for i, job in enumerate(jobs, 1):
        print(f"[{i}/{total_jobs}] {job.category}/{job.name} ({job.distill_type})")

        output = distill_single(client, job, system_prompt)
        if output:
            count = save_distilled(job, output)
            total_examples += count
            print(f"  → {count} examples saved")
        else:
            print(f"  → FAILED")

        # Rate limiting
        if i < total_jobs:
            time.sleep(REQUEST_INTERVAL)

    print(f"\n{'='*50}")
    print(f"Total examples generated: {total_examples}")
    print(f"Output: {DISTILLED_DIR}/")


if __name__ == "__main__":
    main()
